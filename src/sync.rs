//! The engine's event loop: watches SavedVariables for changes, uploads new exports, stages game imports back,
//! checks for addon/app updates, and fires raid reminders. Runs entirely inside the tokio runtime spawned by
//! [`crate::engine::start`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::api::{self, ApiError};
use crate::engine::Repaint;
use crate::release::{self, VersionStatus};
use crate::state::{
    Account, AccountState, Command, ConnectionState, EventsState, HistoryEntry, HistoryKind,
    InstalledAddon, LoginState, Member, Notice, NoticeLevel, Outcome, ReminderScope, Settings,
    Snapshot, SyncPhase, SyncStatus, TokenState, WowInstall,
};
use crate::{addon_install, config, history, notifications, reminders, secrets, self_update, wow};

const RESCAN_INTERVAL: Duration = Duration::from_secs(60);
const ADDON_RELEASE_INTERVAL: Duration = Duration::from_secs(6 * 3600);
const APP_UPDATE_INTERVAL: Duration = Duration::from_secs(12 * 3600);
const EVENTS_INTERVAL: Duration = Duration::from_secs(15 * 60);
const REMINDER_TICK: Duration = Duration::from_secs(30);
const WOW_POLL_INTERVAL: Duration = Duration::from_secs(10);
const NOTICE_SWEEP_INTERVAL: Duration = Duration::from_secs(1);
const FS_DEBOUNCE: Duration = Duration::from_secs(2);
const NOTICE_LIFETIME: chrono::Duration = chrono::Duration::seconds(6);
/// What Sync now says when every account's export is already on the site (or none has one yet).
const NOTHING_NEW_TO_SYNC: &str =
    "Nothing new to sync. Type /sq export in game, then /reload so WoW saves it, and sync again.";
const BASE_BACKOFF: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);
/// How often a signed-in session is re-checked against `/me` (role changes, removal from the guild).
const ACCOUNT_RECHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// How often a saved token is re-checked after the site couldn't be reached.
const ACCOUNT_RETRY_INTERVAL: Duration = Duration::from_secs(60);

/// Drives the engine until [`Command::Shutdown`] is received, then flushes state and returns.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    state: Arc<RwLock<Snapshot>>,
    mut rx: UnboundedReceiver<Command>,
    repaint: Repaint,
    settings: Settings,
    history: Vec<HistoryEntry>,
    sync_state: config::SyncState,
    token: Option<String>,
    calendar_url: Option<String>,
) {
    let mut engine =
        Engine::new(state, repaint, settings, history, sync_state, token, calendar_url);
    engine.bootstrap().await;

    let mut rescan_interval = tokio::time::interval(RESCAN_INTERVAL);
    let mut addon_release_interval = tokio::time::interval(ADDON_RELEASE_INTERVAL);
    let mut app_update_interval = tokio::time::interval(APP_UPDATE_INTERVAL);
    let mut events_interval = tokio::time::interval(EVENTS_INTERVAL);
    let mut reminder_interval = tokio::time::interval(REMINDER_TICK);
    let mut wow_poll_interval = tokio::time::interval(WOW_POLL_INTERVAL);
    let mut notice_sweep_interval = tokio::time::interval(NOTICE_SWEEP_INTERVAL);
    let mut fs_debounce_until: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            cmd = rx.recv() => {
                match cmd {
                    Some(cmd) => if engine.handle_command(cmd).await { break; },
                    None => break,
                }
            }
            Some(()) = recv_optional(&mut engine.fs_events) => {
                fs_debounce_until = Some(tokio::time::Instant::now() + FS_DEBOUNCE);
            }
            () = sleep_until_optional(fs_debounce_until) => {
                fs_debounce_until = None;
                engine.rescan_and_sync(false).await;
            }
            _ = rescan_interval.tick() => { engine.rescan_and_sync(false).await; }
            _ = addon_release_interval.tick() => { if engine.settings.check_addon_updates { engine.check_addon_release().await; } }
            _ = app_update_interval.tick() => { if engine.settings.check_app_updates { engine.check_app_update().await; } }
            _ = events_interval.tick() => { engine.refresh_events().await; }
            _ = reminder_interval.tick() => { engine.reminder_tick().await; }
            _ = wow_poll_interval.tick() => { engine.poll_wow_running().await; }
            _ = notice_sweep_interval.tick() => { engine.sweep_notices(); }
            () = sleep_until_optional(engine.login_poll.as_ref().map(|p| p.next_poll_at)) => {
                engine.poll_login().await;
            }
            () = sleep_until_optional(engine.next_account_check) => {
                engine.check_account().await;
            }
        }
    }
    engine.flush();
}

async fn recv_optional(rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<()>>) -> Option<()> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

async fn sleep_until_optional(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(instant) => tokio::time::sleep_until(instant).await,
        None => std::future::pending().await,
    }
}

/// An in-flight browser sign-in.
struct LoginPoll {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    interval: Duration,
    next_poll_at: tokio::time::Instant,
    expires_at: DateTime<Utc>,
}

#[derive(Default)]
struct BackoffEntry {
    failures: u32,
    next_attempt_at: DateTime<Utc>,
}

struct Engine {
    state: Arc<RwLock<Snapshot>>,
    repaint: Repaint,
    client: reqwest::Client,
    app_version: String,

    settings: Settings,
    token: Option<String>,
    /// A private calendar link the user saved by hand in Settings; takes priority over `site_calendar_url`.
    calendar_url: Option<String>,
    /// A private calendar link the site returned from `/me`. Never persisted outside this process.
    site_calendar_url: Option<String>,
    sync_state: config::SyncState,
    history: Vec<HistoryEntry>,
    notice_seq: u64,
    notices: Vec<Notice>,

    paused: bool,
    /// Who is signed in, as the website sees them. Nothing guild-related runs unless this is `SignedIn`.
    account: AccountState,
    connection: ConnectionState,
    sync: SyncStatus,
    installs: Vec<WowInstall>,
    wow_running: bool,

    addon_release: crate::state::ReleaseCheck,
    last_addon_release: Option<release::GithubRelease>,
    app_update: crate::state::AppUpdate,
    last_app_release: Option<release::GithubRelease>,
    events: EventsState,
    reminder_scheduler: reminders::ReminderScheduler,

    backoff: HashMap<PathBuf, BackoffEntry>,
    pending_stage: HashSet<PathBuf>,

    watcher: Option<notify::RecommendedWatcher>,
    fs_events: Option<tokio::sync::mpsc::UnboundedReceiver<()>>,

    login: LoginState,
    login_poll: Option<LoginPoll>,
    /// When to next check (or re-check) the saved token against `/me`.
    next_account_check: Option<tokio::time::Instant>,
}

impl Engine {
    fn new(
        state: Arc<RwLock<Snapshot>>,
        repaint: Repaint,
        settings: Settings,
        history: Vec<HistoryEntry>,
        sync_state: config::SyncState,
        token: Option<String>,
        calendar_url: Option<String>,
    ) -> Self {
        let client = api::client().unwrap_or_else(|err| {
            tracing::error!("falling back to a minimal HTTP client: {err:#}");
            reqwest::Client::new()
        });
        Self {
            state,
            repaint,
            client,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            settings,
            token,
            calendar_url,
            site_calendar_url: None,
            sync_state,
            history,
            notice_seq: 0,
            notices: Vec::new(),
            paused: false,
            account: AccountState::SignedOut,
            connection: ConnectionState::Unknown,
            sync: SyncStatus::default(),
            installs: Vec::new(),
            wow_running: false,
            addon_release: crate::state::ReleaseCheck::NotChecked,
            last_addon_release: None,
            app_update: crate::state::AppUpdate::NotChecked,
            last_app_release: None,
            events: EventsState::default(),
            reminder_scheduler: reminders::ReminderScheduler::default(),
            backoff: HashMap::new(),
            pending_stage: HashSet::new(),
            watcher: None,
            fs_events: None,
            login: LoginState::Idle,
            login_poll: None,
            next_account_check: None,
        }
    }

    // --- Startup ---------------------------------------------------------------------------------------------

    async fn bootstrap(&mut self) {
        if secrets::using_file_fallback() {
            self.push_notice(
                NoticeLevel::Warning,
                "Your OS keyring isn't available; secrets are stored in a local file instead."
                    .to_owned(),
            );
        }
        self.wow_running = wow::process::is_wow_running();
        self.discover_installs();
        self.publish();

        if self.token.is_some() {
            self.account = AccountState::Checking;
            self.publish();
            self.check_account().await;
        } else {
            self.account = AccountState::SignedOut;
            self.set_phase(SyncPhase::NeedsSetup("Sign in to start syncing.".to_owned()));
        }
        self.rescan_and_sync(false).await;
        if self.settings.check_addon_updates {
            self.check_addon_release().await;
        }
        if self.settings.check_app_updates {
            self.check_app_update().await;
        }
        self.refresh_events().await;
    }

    // --- Commands ----------------------------------------------------------------------------------------------

    /// Returns `true` when the engine should stop (i.e. on `Shutdown`).
    async fn handle_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::SyncNow => self.rescan_and_sync(true).await,
            Command::SetPaused(paused) => {
                self.paused = paused;
                self.set_phase(if paused { SyncPhase::Paused } else { SyncPhase::Watching });
                self.publish();
            }
            Command::SaveSettings(new_settings) => self.save_settings(new_settings).await,
            Command::SetToken(token) => self.set_token(token).await,
            Command::BeginBrowserLogin => self.begin_browser_login().await,
            Command::CancelBrowserLogin => {
                self.login_poll = None;
                self.login = LoginState::Idle;
                self.publish();
            }
            Command::ReopenBrowserLogin => {
                if let Some(poll) = &self.login_poll {
                    open_in_browser(&poll.verification_uri_complete);
                }
            }
            Command::SignOut => self.sign_out().await,
            Command::SetCalendarUrl(url) => self.set_calendar_url(url).await,
            Command::TestConnection => self.test_connection().await,
            Command::Rescan => self.rescan_and_sync(false).await,
            Command::CheckAddonRelease => self.check_addon_release().await,
            Command::InstallAddon { flavor } => self.handle_install_addon_command(flavor).await,
            Command::StageImport { saved_variables } => {
                self.handle_stage_import_command(saved_variables).await
            }
            Command::CheckAppUpdate => self.check_app_update().await,
            Command::InstallAppUpdate => self.install_app_update().await,
            Command::RefreshEvents => self.refresh_events().await,
            Command::DismissNotice(id) => {
                self.notices.retain(|n| n.id != id);
                self.publish();
            }
            Command::Shutdown => {
                self.flush();
                return true;
            }
        }
        false
    }

    async fn save_settings(&mut self, mut new_settings: Settings) {
        config::clamp_settings(&mut new_settings);
        let roots_changed = new_settings.wow_roots != self.settings.wow_roots
            || new_settings.auto_detect != self.settings.auto_detect
            || new_settings.disabled_accounts != self.settings.disabled_accounts;
        let launch_changed = new_settings.launch_at_login != self.settings.launch_at_login;
        self.settings = new_settings;
        if let Err(err) = config::save_settings(&self.settings) {
            self.push_notice(NoticeLevel::Error, format!("Could not save settings: {err:#}"));
        }
        if launch_changed {
            let enabled = self.settings.launch_at_login;
            let result =
                tokio::task::spawn_blocking(move || crate::autostart::set_launch_at_login(enabled))
                    .await
                    .unwrap_or_else(|err| Err(anyhow::anyhow!(err)));
            if let Err(err) = result {
                self.push_notice(
                    NoticeLevel::Warning,
                    format!("Could not update launch-at-login: {err:#}"),
                );
            }
        }
        if roots_changed {
            self.rescan_and_sync(false).await;
        } else {
            self.publish();
        }
    }

    async fn set_token(&mut self, token: String) {
        if let Err(err) = secrets::set_token(&token) {
            self.push_notice(NoticeLevel::Error, format!("Could not save the sync token: {err:#}"));
            self.publish();
            return;
        }
        self.token = Some(token);
        self.backoff.clear();
        self.account = AccountState::Checking;
        self.connection = ConnectionState::Checking;
        self.publish();
        self.check_account().await;
        self.rescan_and_sync(false).await;
        self.refresh_events().await;
    }

    async fn begin_browser_login(&mut self) {
        self.login_poll = None;
        self.login = LoginState::Starting;
        self.publish();
        let device_name = device_name();
        let started =
            api::start_device_authorization(&self.client, &self.settings.site_url, &device_name)
                .await;
        match started {
            Ok(auth) => {
                let interval = Duration::from_secs(auth.interval.clamp(1, 60));
                let expires_at = Utc::now()
                    + chrono::Duration::seconds(i64::try_from(auth.expires_in).unwrap_or(600));
                open_in_browser(&auth.verification_uri_complete);
                self.login = LoginState::Waiting {
                    user_code: auth.user_code.clone(),
                    verification_uri_complete: auth.verification_uri_complete.clone(),
                    expires_at,
                };
                self.login_poll = Some(LoginPoll {
                    device_code: auth.device_code,
                    user_code: auth.user_code,
                    verification_uri_complete: auth.verification_uri_complete,
                    interval,
                    next_poll_at: tokio::time::Instant::now() + interval,
                    expires_at,
                });
            }
            Err(ApiError::NotFound) => {
                self.login = LoginState::Failed(
                    "This site doesn't support signing in from the browser yet. Paste a sync token instead."
                        .to_owned(),
                );
            }
            Err(err) => {
                self.login = LoginState::Failed(format!("Could not start signing in: {err}"))
            }
        }
        self.publish();
    }

    async fn poll_login(&mut self) {
        let Some(poll) = self.login_poll.as_mut() else { return };
        if Utc::now() >= poll.expires_at {
            self.login_poll = None;
            self.login = LoginState::Failed(
                "The sign-in code expired before it was approved. Start again.".to_owned(),
            );
            self.publish();
            return;
        }
        let device_code = poll.device_code.clone();
        // Schedule the next poll first, so an error below never makes the loop spin.
        poll.next_poll_at = tokio::time::Instant::now() + poll.interval;
        let result =
            api::poll_device_token(&self.client, &self.settings.site_url, &device_code).await;
        match result {
            Ok(api::DeviceTokenPoll::Pending) => {}
            Ok(api::DeviceTokenPoll::SlowDown) => {
                if let Some(poll) = self.login_poll.as_mut() {
                    poll.interval += Duration::from_secs(5);
                    poll.next_poll_at = tokio::time::Instant::now() + poll.interval;
                }
            }
            Ok(api::DeviceTokenPoll::Denied) => {
                self.login_poll = None;
                self.login = LoginState::Failed("Sign-in was cancelled on the website.".to_owned());
                self.publish();
            }
            Ok(api::DeviceTokenPoll::Expired) => {
                self.login_poll = None;
                self.login = LoginState::Failed(
                    "The sign-in code expired or was already used. Start again.".to_owned(),
                );
                self.publish();
            }
            Ok(api::DeviceTokenPoll::Approved { token }) => {
                let user_code = self.login_poll.take().map(|p| p.user_code).unwrap_or_default();
                tracing::info!("browser sign-in approved for code {user_code}");
                self.login = LoginState::Done;
                self.push_notice(NoticeLevel::Success, "Signed in. Syncing is ready.".to_owned());
                self.set_token(token).await;
            }
            // Network blips: keep polling until the code expires.
            Err(err) => tracing::warn!("sign-in poll failed: {err}"),
        }
    }

    /// Remove the saved token locally (does not revoke it on the site).
    fn forget_token(&mut self) {
        if let Err(err) = secrets::clear_token() {
            self.push_notice(
                NoticeLevel::Error,
                format!("Could not clear the sync token: {err:#}"),
            );
        }
        self.token = None;
        self.backoff.clear();
    }

    /// Stop showing anything guild-related once the user is signed out: the gate hides it anyway, but keep the
    /// engine's own state honest.
    fn clear_guild_data(&mut self) {
        self.events = EventsState::default();
        self.site_calendar_url = None;
    }

    async fn sign_out(&mut self) {
        if let Some(token) = self.token.clone()
            && let Err(err) = api::revoke_me(&self.client, &self.settings.site_url, &token).await
        {
            tracing::warn!("could not revoke the sync token on sign out: {err}");
        }
        self.forget_token();
        self.next_account_check = None;
        self.account = AccountState::SignedOut;
        self.connection = ConnectionState::Unknown;
        self.clear_guild_data();
        self.set_phase(SyncPhase::NeedsSetup("Sign in to start syncing.".to_owned()));
        self.publish();
    }

    async fn set_calendar_url(&mut self, url: Option<String>) {
        let result = match &url {
            Some(u) => secrets::set_calendar_url(u),
            None => secrets::clear_calendar_url(),
        };
        if let Err(err) = result {
            self.push_notice(
                NoticeLevel::Error,
                format!("Could not save the calendar link: {err:#}"),
            );
        }
        self.calendar_url = url;
        self.publish();
        self.refresh_events().await;
    }

    /// The private calendar link used for reminders: one saved by hand takes priority over the site's.
    fn effective_calendar_url(&self) -> Option<&str> {
        self.calendar_url.as_deref().or(self.site_calendar_url.as_deref())
    }

    fn is_signed_in(&self) -> bool {
        matches!(self.account, AccountState::SignedIn(_))
    }

    fn is_officer(&self) -> bool {
        matches!(&self.account, AccountState::SignedIn(member) if member.is_officer)
    }

    /// Check (or re-check) the saved token against `GET /me`, updating [`AccountState`] and scheduling the next
    /// check. Also used for `Command::TestConnection`.
    async fn test_connection(&mut self) {
        self.check_account().await;
    }

    async fn check_account(&mut self) {
        let Some(token) = self.token.clone() else {
            self.account = AccountState::SignedOut;
            self.connection = ConnectionState::Unknown;
            self.next_account_check = None;
            self.publish();
            return;
        };
        self.account = AccountState::Checking;
        self.connection = ConnectionState::Checking;
        self.publish();
        match api::me(&self.client, &self.settings.site_url, &token).await {
            Ok(me) => {
                self.on_signed_in(me.display_name, me.avatar_url, me.is_officer, me.calendar_url)
            }
            Err(ApiError::Unauthorized) => {
                self.forget_token();
                self.account = AccountState::SignedOut;
                self.connection =
                    ConnectionState::Unauthorized("Token missing, invalid or revoked.".to_owned());
                self.clear_guild_data();
                self.next_account_check = None;
                self.push_notice(
                    NoticeLevel::Warning,
                    "Your sign-in was revoked. Sign in again.".to_owned(),
                );
                self.set_phase(SyncPhase::NeedsSetup("Sign in to start syncing.".to_owned()));
                self.publish();
            }
            Err(ApiError::Forbidden(message)) => {
                self.forget_token();
                self.account = AccountState::NotInGuild;
                self.connection = ConnectionState::Forbidden(message);
                self.clear_guild_data();
                self.next_account_check = None;
                self.set_phase(SyncPhase::NeedsSetup("Sign in to start syncing.".to_owned()));
                self.publish();
            }
            // An older site without this route yet: fall back to the officer-only token test.
            Err(ApiError::NotFound) => {
                match api::test_token(&self.client, &self.settings.site_url, &token).await {
                    Ok(()) => self.on_signed_in("Officer".to_owned(), None, true, None),
                    Err(ApiError::Unauthorized) | Err(ApiError::Forbidden(_)) => {
                        self.forget_token();
                        self.account = AccountState::SignedOut;
                        self.connection = ConnectionState::Unauthorized(
                            "Token missing, invalid or revoked.".to_owned(),
                        );
                        self.clear_guild_data();
                        self.next_account_check = None;
                        self.set_phase(SyncPhase::NeedsSetup(
                            "Sign in to start syncing.".to_owned(),
                        ));
                        self.publish();
                    }
                    Err(err) => self.on_unverified(err.to_string()),
                }
            }
            Err(err) => self.on_unverified(err.to_string()),
        }
    }

    fn on_signed_in(
        &mut self,
        display_name: String,
        avatar_url: Option<String>,
        is_officer: bool,
        calendar_url: Option<String>,
    ) {
        self.connection = ConnectionState::Ok;
        self.site_calendar_url = calendar_url;
        self.account = AccountState::SignedIn(Member { display_name, avatar_url, is_officer });
        self.next_account_check = Some(tokio::time::Instant::now() + ACCOUNT_RECHECK_INTERVAL);
        self.publish();
    }

    fn on_unverified(&mut self, message: String) {
        self.account = AccountState::Unverified(message.clone());
        self.connection = ConnectionState::Unreachable(message);
        self.next_account_check = Some(tokio::time::Instant::now() + ACCOUNT_RETRY_INTERVAL);
        self.publish();
    }

    async fn handle_install_addon_command(&mut self, flavor: PathBuf) {
        let release = match self.last_addon_release.clone() {
            Some(release) => release,
            None => match release::latest(&self.client, release::ADDON_REPO).await {
                Ok(release) => release,
                Err(err) => {
                    self.push_notice(
                        NoticeLevel::Error,
                        format!("Could not check for addon releases: {err:#}"),
                    );
                    self.publish();
                    return;
                }
            },
        };
        self.install_addon(&release, &flavor).await;
    }

    async fn handle_stage_import_command(&mut self, saved_variables: PathBuf) {
        if !self.is_officer() {
            self.push_notice(
                NoticeLevel::Warning,
                "Staging a game import is for officers.".to_owned(),
            );
            self.publish();
            return;
        }
        if wow::process::is_wow_running() {
            self.push_notice(
                NoticeLevel::Warning,
                "Close WoW before staging an import.".to_owned(),
            );
            self.publish();
            return;
        }
        let Some(token) = self.token.clone() else {
            self.push_notice(
                NoticeLevel::Warning,
                "Add a sync token before staging an import.".to_owned(),
            );
            self.publish();
            return;
        };
        self.stage_import_now(&saved_variables, &token).await;
        self.discover_installs();
        self.publish();
    }

    async fn install_app_update(&mut self) {
        let Some(release) = self.last_app_release.clone() else {
            self.push_notice(NoticeLevel::Warning, "Check for app updates first.".to_owned());
            self.publish();
            return;
        };
        self.app_update = crate::state::AppUpdate::Downloading { progress: 0.0 };
        self.publish();

        let state = self.state.clone();
        let repaint = self.repaint.clone();
        let progress: Box<dyn Fn(f32) + Send + Sync> = Box::new(move |fraction: f32| {
            if let Ok(mut snapshot) = state.write() {
                snapshot.app_update = crate::state::AppUpdate::Downloading { progress: fraction };
            }
            repaint();
        });

        match self_update::download_and_install(&self.client, &release, progress.as_ref()).await {
            Ok(()) => {
                self.app_update = crate::state::AppUpdate::ReadyToRestart(release.to_state());
                self.push_history(
                    HistoryKind::AppUpdated,
                    Outcome::Success,
                    "SideQuest Companion",
                    &format!("Updated to {}", release.version),
                    None,
                );
            }
            Err(err) => {
                // `{:#}` keeps the cause ("Access is denied", ...) after the step that failed.
                let message = format!("{err:#}");
                tracing::warn!("app update failed: {message}");
                self.app_update = crate::state::AppUpdate::Failed(message.clone());
                self.push_history(
                    HistoryKind::AppUpdated,
                    Outcome::Failed,
                    "SideQuest Companion",
                    &message,
                    None,
                );
            }
        }
        self.publish();
    }

    // --- Discovery and syncing ----------------------------------------------------------------------------------

    fn discover_installs(&mut self) {
        let mut installs = wow::discovery::scan_all(
            &self.settings.wow_roots,
            self.settings.auto_detect,
            &self.settings.disabled_accounts,
        );
        for install in &mut installs {
            for flavor in &mut install.flavors {
                for account in &mut flavor.accounts {
                    enrich_account(account, &self.sync_state);
                }
            }
        }
        self.installs = installs;
        self.register_watchers();
    }

    fn register_watchers(&mut self) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let mut watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if res.is_ok() {
                    let _ = tx.send(());
                }
            }) {
                Ok(watcher) => watcher,
                Err(err) => {
                    tracing::warn!("could not start a SavedVariables file watcher: {err}");
                    self.watcher = None;
                    self.fs_events = None;
                    return;
                }
            };
        for install in &self.installs {
            for flavor in &install.flavors {
                let accounts_dir = flavor.path.join("WTF/Account");
                if accounts_dir.is_dir()
                    && let Err(err) = notify::Watcher::watch(
                        &mut watcher,
                        &accounts_dir,
                        notify::RecursiveMode::Recursive,
                    )
                {
                    tracing::warn!("could not watch {}: {err}", accounts_dir.display());
                }
            }
        }
        self.watcher = Some(watcher);
        self.fs_events = Some(rx);
    }

    async fn rescan_and_sync(&mut self, force: bool) {
        self.discover_installs();
        self.publish();

        if !self.is_signed_in() {
            self.needs_setup("Sign in to start syncing.", force);
            return;
        }
        if !self.is_officer() {
            self.needs_setup(
                "Sync is for officers; you'll still get addon updates and reminders.",
                force,
            );
            return;
        }
        if (self.paused || !self.settings.auto_sync) && !force {
            self.set_phase(SyncPhase::Paused);
            self.publish();
            return;
        }
        let Some(token) = self.token.clone() else {
            self.needs_setup("Add a sync token in Settings to start syncing.", force);
            return;
        };
        self.set_phase(SyncPhase::Watching);
        if force {
            // Sync now is an explicit retry: don't skip accounts still waiting out a failure's backoff.
            self.backoff.clear();
        }
        let attempted = self.upload_pending_exports(&token).await;
        if force && attempted == 0 {
            // Every upload already says how it went; a click that found nothing to send must say so too.
            self.push_notice(NoticeLevel::Info, NOTHING_NEW_TO_SYNC.to_owned());
        }
        self.discover_installs();
        self.publish();
    }

    /// Sets a needs-setup phase. A forced sync (Sync now) also says why in a toast, so the click isn't silent.
    fn needs_setup(&mut self, reason: &str, force: bool) {
        self.set_phase(SyncPhase::NeedsSetup(reason.to_owned()));
        if force {
            self.push_notice(NoticeLevel::Warning, format!("Couldn't sync. {reason}"));
        }
        self.publish();
    }

    /// Uploads every enabled account's new export; returns how many uploads it tried.
    async fn upload_pending_exports(&mut self, token: &str) -> usize {
        let now = Utc::now();
        let candidates: Vec<(PathBuf, String, String)> = self
            .installs
            .iter()
            .flat_map(|install| install.flavors.iter())
            .flat_map(|flavor| flavor.accounts.iter())
            .filter(|account| account.enabled && !account.export_uploaded)
            .filter(|account| {
                !self.backoff.get(&account.saved_variables).is_some_and(|b| b.next_attempt_at > now)
            })
            .filter_map(|account| {
                let text = std::fs::read_to_string(&account.saved_variables).ok()?;
                let export = wow::savedvars::extract(&text).last_export?;
                Some((account.saved_variables.clone(), account.name.clone(), export))
            })
            .collect();

        let attempted = candidates.len();
        for (saved_variables, account_name, export) in candidates {
            self.upload_one(token, &saved_variables, &account_name, &export).await;
        }
        attempted
    }

    async fn upload_one(
        &mut self,
        token: &str,
        saved_variables: &Path,
        account_name: &str,
        export: &str,
    ) {
        self.set_phase(SyncPhase::Uploading);
        self.publish();
        match api::upload(&self.client, &self.settings.site_url, token, export).await {
            Ok(result) => {
                let hash = sha256_hex(export);
                self.sync_state.mark_uploaded(&hash, Utc::now());
                if let Err(err) = config::save_state(&self.sync_state) {
                    tracing::warn!("could not persist sync state: {err:#}");
                }
                self.backoff.remove(saved_variables);
                let summary = api::summary(&result);
                self.push_history(
                    HistoryKind::Upload,
                    Outcome::Success,
                    account_name,
                    &summary,
                    None,
                );
                if self.settings.notify_on_sync {
                    notifications::notify(
                        "SideQuest Companion",
                        &format!("{account_name}: {summary}"),
                    );
                }
                self.push_notice(NoticeLevel::Success, format!("Synced {account_name}: {summary}"));
                self.set_phase(SyncPhase::Watching);
                self.sync.last_success = Some(Utc::now());

                if self.settings.stage_imports {
                    self.maybe_stage_import(saved_variables, token).await;
                }
            }
            Err(err) => self.handle_upload_error(saved_variables, account_name, err).await,
        }
    }

    async fn handle_upload_error(
        &mut self,
        saved_variables: &Path,
        account_name: &str,
        err: ApiError,
    ) {
        let message = err.to_string();
        let mut recheck_account = false;
        match err {
            ApiError::Unauthorized => {
                self.connection = ConnectionState::Unauthorized(message.clone());
                self.set_phase(SyncPhase::NeedsSetup(format!("Sync token problem: {message}")));
                self.backoff.remove(saved_variables);
            }
            ApiError::Forbidden(_) => {
                self.connection = ConnectionState::Forbidden(message.clone());
                self.set_phase(SyncPhase::NeedsSetup(format!("Sync token problem: {message}")));
                self.backoff.remove(saved_variables);
                // The role may have changed since we last checked; re-check who we are.
                recheck_account = true;
            }
            _ => {
                let entry = self.backoff.entry(saved_variables.to_path_buf()).or_default();
                entry.failures += 1;
                entry.next_attempt_at = Utc::now()
                    + chrono::Duration::from_std(backoff_delay(entry.failures))
                        .unwrap_or(chrono::Duration::seconds(60));
                self.set_phase(SyncPhase::Error(message.clone()));
            }
        }
        self.push_history(HistoryKind::Upload, Outcome::Failed, account_name, &message, None);
        if self.settings.notify_on_error {
            notifications::notify("SideQuest Companion sync failed", &message);
        }
        self.push_notice(NoticeLevel::Error, format!("{account_name}: {message}"));
        if recheck_account {
            self.check_account().await;
        }
    }

    async fn maybe_stage_import(&mut self, saved_variables: &Path, token: &str) {
        if wow::process::is_wow_running() {
            self.pending_stage.insert(saved_variables.to_path_buf());
            return;
        }
        self.stage_import_now(saved_variables, token).await;
    }

    async fn stage_import_now(&mut self, saved_variables: &Path, token: &str) {
        let label = account_label(saved_variables);
        match api::game_import(&self.client, &self.settings.site_url, token).await {
            Ok(game_import) => match wow::savedvars::stage_inbox(
                saved_variables,
                &game_import.payload,
                game_import.generated_at,
                Utc::now(),
            ) {
                Ok(()) => {
                    self.push_history(
                        HistoryKind::ImportStaged,
                        Outcome::Success,
                        &label,
                        "Game import staged".to_owned().as_str(),
                        None,
                    );
                    self.pending_stage.remove(saved_variables);
                }
                Err(err) => {
                    self.push_history(
                        HistoryKind::ImportStaged,
                        Outcome::Failed,
                        &label,
                        &err.to_string(),
                        None,
                    );
                }
            },
            // An old site answers 401 here even with a valid (already-tested) token: it just doesn't support
            // staging yet. Leave it queued rather than erroring.
            Err(ApiError::Unauthorized) => {
                tracing::debug!(
                    "game-import not supported by this site yet (treating as unsupported, not a token error)"
                );
            }
            Err(err) => {
                self.push_history(
                    HistoryKind::ImportStaged,
                    Outcome::Failed,
                    &label,
                    &err.to_string(),
                    None,
                );
            }
        }
    }

    async fn poll_wow_running(&mut self) {
        let running = wow::process::is_wow_running();
        let just_closed = self.wow_running && !running;
        self.wow_running = running;

        if just_closed
            && !self.pending_stage.is_empty()
            && let Some(token) = self.token.clone()
        {
            let paths: Vec<PathBuf> = self.pending_stage.iter().cloned().collect();
            for path in paths {
                self.stage_import_now(&path, &token).await;
            }
            self.discover_installs();
        }
        if just_closed && self.settings.auto_update_addon && self.settings.check_addon_updates {
            self.auto_install_outdated_addons().await;
        }
        self.publish();
    }

    // --- Addon / app updates -------------------------------------------------------------------------------------

    async fn check_addon_release(&mut self) {
        self.addon_release = crate::state::ReleaseCheck::Checking;
        self.publish();
        match release::latest(&self.client, release::ADDON_REPO).await {
            Ok(release) => {
                self.addon_release = crate::state::ReleaseCheck::Known {
                    latest: release.to_state(),
                    checked_at: Utc::now(),
                };
                self.last_addon_release = Some(release.clone());
                self.publish();
                if self.settings.auto_update_addon && !self.wow_running {
                    self.auto_install_outdated_addons().await;
                }
            }
            Err(err) => {
                self.addon_release = crate::state::ReleaseCheck::Failed(err.to_string());
                self.publish();
            }
        }
    }

    async fn auto_install_outdated_addons(&mut self) {
        let Some(release) = self.last_addon_release.clone() else { return };
        let outdated_flavors: Vec<PathBuf> = self
            .installs
            .iter()
            .flat_map(|install| install.flavors.iter())
            .filter(|flavor| {
                matches!(&flavor.addon, InstalledAddon::Installed { version, .. } if release::compare(version, &release.version) == VersionStatus::Outdated)
            })
            .map(|flavor| flavor.path.clone())
            .collect();
        for flavor_path in outdated_flavors {
            self.install_addon(&release, &flavor_path).await;
        }
    }

    async fn install_addon(&mut self, release: &release::GithubRelease, flavor_dir: &Path) {
        let label =
            flavor_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match addon_install::install_latest(&self.client, release, flavor_dir).await {
            Ok(version) => {
                self.push_history(
                    HistoryKind::AddonInstalled,
                    Outcome::Success,
                    &label,
                    &format!("Installed addon {version}"),
                    None,
                );
                self.push_notice(
                    NoticeLevel::Success,
                    format!("Addon updated to {version} ({label})"),
                );
            }
            Err(err) => {
                self.push_history(
                    HistoryKind::AddonInstalled,
                    Outcome::Failed,
                    &label,
                    &err.to_string(),
                    None,
                );
                self.push_notice(
                    NoticeLevel::Error,
                    format!("Addon install failed ({label}): {err}"),
                );
            }
        }
        self.discover_installs();
        self.publish();
    }

    async fn check_app_update(&mut self) {
        self.app_update = crate::state::AppUpdate::Checking;
        self.publish();
        match release::latest(&self.client, release::APP_REPO).await {
            Ok(release) => {
                self.last_app_release = Some(release.clone());
                self.app_update = match release::compare(&self.app_version, &release.version) {
                    VersionStatus::Outdated => {
                        crate::state::AppUpdate::Available(release.to_state())
                    }
                    _ => crate::state::AppUpdate::UpToDate { checked_at: Utc::now() },
                };
            }
            Err(err) => {
                tracing::warn!("checking for an app update failed: {err:#}");
                self.app_update = crate::state::AppUpdate::Failed(format!("{err:#}"));
            }
        }
        self.publish();

        let auto_version = match &self.app_update {
            crate::state::AppUpdate::Available(latest)
                if should_auto_update_app(
                    &self.settings,
                    self_update::can_self_update(),
                    self.sync_state.auto_update_attempted.as_deref(),
                    &latest.version,
                ) =>
            {
                Some(latest.version.clone())
            }
            _ => None,
        };
        if let Some(version) = auto_version {
            self.auto_update_app(version).await;
        }
    }

    /// Installs the update a check just found and restarts into it. The version is recorded first, so a
    /// relaunch that still finds itself outdated can't install and restart over and over.
    async fn auto_update_app(&mut self, version: String) {
        self.sync_state.auto_update_attempted = Some(version.clone());
        if let Err(err) = config::save_state(&self.sync_state) {
            tracing::warn!(
                "could not persist the auto-update attempt, not updating automatically: {err:#}"
            );
            return;
        }
        tracing::info!(%version, "installing app update automatically");
        self.install_app_update().await;
        if !matches!(self.app_update, crate::state::AppUpdate::ReadyToRestart(_)) {
            return; // the failure is in the snapshot and history; the Install button stays
        }
        notifications::notify(
            "SideQuest Companion",
            &format!("Updated to v{version}; restarting to finish."),
        );
        self.flush();
        if self_update::INSTALLER_RESTARTS_APP {
            return; // setup closes this app through `--quit` and starts the new version
        }
        match self_update::relaunch() {
            Ok(()) => std::process::exit(0),
            Err(err) => tracing::warn!("installed v{version} but could not relaunch: {err:#}"),
        }
    }

    // --- Events and reminders -----------------------------------------------------------------------------------

    async fn refresh_events(&mut self) {
        if !self.is_signed_in() {
            return;
        }
        let result = if self.settings.reminders.scope == ReminderScope::SignedUp {
            match self.effective_calendar_url().map(str::to_owned) {
                Some(url) => reminders::fetch_calendar(&self.client, &url).await,
                None => Err(anyhow::anyhow!("no private calendar link saved; add one in Settings")),
            }
        } else {
            reminders::fetch_public_events(
                &self.client,
                &self.settings.site_url,
                &self.settings.reminders.kinds,
            )
            .await
        };
        match result {
            Ok(events) => {
                self.events =
                    EventsState { upcoming: events, fetched_at: Some(Utc::now()), error: None };
            }
            Err(err) => self.events.error = Some(err.to_string()),
        }
        self.publish();
    }

    async fn reminder_tick(&mut self) {
        if !self.is_signed_in() || self.events.upcoming.is_empty() {
            return;
        }
        let due = self.reminder_scheduler.due(
            &self.events.upcoming,
            &self.settings.reminders,
            Utc::now(),
        );
        if due.is_empty() {
            return;
        }
        for event in &due {
            let body = format!("{} starts at {}", event.title, event.starts_at.format("%H:%M UTC"));
            notifications::notify(&format!("Upcoming: {}", event.title), &body);
            self.push_notice(
                NoticeLevel::Info,
                format!("Reminder: {} at {}", event.title, event.starts_at.format("%H:%M UTC")),
            );
        }
        self.publish();
    }

    // --- Notices, history, publishing ----------------------------------------------------------------------------

    fn push_notice(&mut self, level: NoticeLevel, text: String) {
        self.notice_seq += 1;
        self.notices.push(Notice { id: self.notice_seq, level, text, at: Utc::now() });
    }

    fn sweep_notices(&mut self) {
        let cutoff = Utc::now() - NOTICE_LIFETIME;
        let before = self.notices.len();
        self.notices.retain(|n| n.at > cutoff);
        if self.notices.len() != before {
            self.publish();
        }
    }

    fn push_history(
        &mut self,
        kind: HistoryKind,
        outcome: Outcome,
        subject: &str,
        summary: &str,
        detail: Option<String>,
    ) {
        history::append(
            &mut self.history,
            HistoryEntry {
                at: Utc::now(),
                kind,
                outcome,
                subject: subject.to_owned(),
                summary: summary.to_owned(),
                detail,
            },
        );
        if let Err(err) = history::save(&self.history) {
            tracing::warn!("could not persist history: {err:#}");
        }
    }

    fn set_phase(&mut self, phase: SyncPhase) {
        self.sync.phase = phase;
        self.sync.last_attempt = Some(Utc::now());
    }

    fn publish(&self) {
        let snapshot = Snapshot {
            settings: self.settings.clone(),
            token: self
                .token
                .as_deref()
                .map(|t| TokenState::Present { masked: secrets::masked(t) })
                .unwrap_or(TokenState::Missing),
            calendar_linked: self.effective_calendar_url().is_some(),
            login: self.login.clone(),
            account: self.account.clone(),
            connection: self.connection.clone(),
            sync: self.sync.clone(),
            installs: self.installs.clone(),
            history: self.history.clone(),
            addon_release: self.addon_release.clone(),
            app_update: self.app_update.clone(),
            events: self.events.clone(),
            wow_running: self.wow_running,
            notices: self.notices.clone(),
            app_version: self.app_version.clone(),
        };
        match self.state.write() {
            Ok(mut guard) => *guard = snapshot,
            Err(poisoned) => *poisoned.into_inner() = snapshot,
        }
        (self.repaint)();
    }

    /// Persist everything that isn't already saved incrementally, so a restart loses nothing.
    fn flush(&mut self) {
        if let Err(err) = config::save_settings(&self.settings) {
            tracing::warn!("flush: could not save settings: {err:#}");
        }
        if let Err(err) = config::save_state(&self.sync_state) {
            tracing::warn!("flush: could not save sync state: {err:#}");
        }
        if let Err(err) = history::save(&self.history) {
            tracing::warn!("flush: could not save history: {err:#}");
        }
    }
}

/// The computer's name for the authorize page and the token name, e.g. `Coffee's MacBook Pro`.
fn device_name() -> String {
    sysinfo::System::host_name()
        .ok()
        .map(|name| name.trim().trim_end_matches(".local").to_owned())
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(40).collect())
        .unwrap_or_else(|| "this computer".to_owned())
}

fn open_in_browser(url: &str) {
    if let Err(err) = open::that_detached(url) {
        tracing::warn!("could not open the browser: {err}");
    }
}

fn enrich_account(account: &mut Account, sync_state: &config::SyncState) {
    let Ok(text) = std::fs::read_to_string(&account.saved_variables) else { return };
    let parsed = wow::savedvars::extract(&text);
    account.pending = parsed.pending;
    account.inbox_staged = parsed.inbox_staged;
    account.standings_generated_at = parsed.standings_generated_at;
    match parsed.last_export {
        Some(export) => {
            let hash = sha256_hex(&export);
            account.export_uploaded = sync_state.is_uploaded(&hash);
            account.export_hash = Some(hash);
        }
        None => {
            account.export_hash = None;
            account.export_uploaded = false;
        }
    }
}

fn sha256_hex(payload: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    hex::encode(hasher.finalize())
}

fn account_label(saved_variables: &Path) -> String {
    // .../<ACCOUNT>/SavedVariables/SideQuest.lua -> "<flavor> · <ACCOUNT>"
    let account = saved_variables
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let flavor = saved_variables.ancestors().find_map(|p| {
        let name = p.file_name()?.to_string_lossy();
        (name.starts_with('_') && name.ends_with('_')).then(|| name.into_owned())
    });
    match (flavor, account) {
        (Some(flavor), Some(account)) => format!("{flavor} · {account}"),
        (None, Some(account)) => account,
        _ => saved_variables.display().to_string(),
    }
}

/// Whether a check that found `latest` should install it unattended: the user left both update switches on,
/// this install can replace itself, and this version hasn't been tried automatically before.
fn should_auto_update_app(
    settings: &Settings,
    can_self_update: bool,
    already_attempted: Option<&str>,
    latest: &str,
) -> bool {
    settings.check_app_updates
        && settings.auto_update_app
        && can_self_update
        && already_attempted != Some(latest)
}

fn backoff_delay(failures: u32) -> Duration {
    let multiplier = 1u32.checked_shl(failures.saturating_sub(1).min(10)).unwrap_or(u32::MAX);
    let candidate = BASE_BACKOFF.saturating_mul(multiplier);
    candidate.min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delay_doubles_and_caps() {
        assert_eq!(backoff_delay(1), Duration::from_secs(15));
        assert_eq!(backoff_delay(2), Duration::from_secs(30));
        assert_eq!(backoff_delay(3), Duration::from_secs(60));
        assert_eq!(backoff_delay(20), MAX_BACKOFF);
    }

    #[test]
    fn account_label_combines_flavor_and_account_name() {
        let path = PathBuf::from(
            "/Applications/World of Warcraft/_retail_/WTF/Account/MYACCOUNT/SavedVariables/SideQuest.lua",
        );
        assert_eq!(account_label(&path), "_retail_ · MYACCOUNT");
    }

    fn test_engine(site_url: String, roots: Vec<PathBuf>) -> Engine {
        let settings =
            Settings { site_url, wow_roots: roots, auto_detect: false, ..Settings::default() };
        let mut engine = Engine::new(
            Arc::new(RwLock::new(Snapshot::default())),
            Arc::new(|| {}),
            settings,
            Vec::new(),
            config::SyncState::default(),
            Some("sqs_test".to_owned()),
            None,
        );
        engine.account = AccountState::SignedIn(crate::state::Member {
            display_name: "Officer".to_owned(),
            avatar_url: None,
            is_officer: true,
        });
        engine
    }

    #[tokio::test]
    async fn sync_now_with_nothing_new_says_so() {
        let mut engine = test_engine("http://127.0.0.1:9".to_owned(), Vec::new());
        engine.rescan_and_sync(true).await;
        assert!(engine.notices.iter().any(|n| n.text == NOTHING_NEW_TO_SYNC));

        // A background rescan finding nothing stays quiet.
        engine.notices.clear();
        engine.rescan_and_sync(false).await;
        assert!(engine.notices.is_empty());
    }

    #[tokio::test]
    async fn sync_now_explains_why_it_cannot_sync() {
        let mut engine = test_engine("http://127.0.0.1:9".to_owned(), Vec::new());
        engine.account = AccountState::SignedIn(crate::state::Member {
            display_name: "Member".to_owned(),
            avatar_url: None,
            is_officer: false,
        });
        engine.rescan_and_sync(true).await;
        assert!(matches!(engine.notices.as_slice(), [n] if n.level == NoticeLevel::Warning
            && n.text.starts_with("Couldn't sync.")));
    }

    #[test]
    fn auto_update_installs_each_version_once_and_respects_the_switches() {
        let on = Settings::default();
        assert!(on.auto_update_app, "on by default");
        assert!(should_auto_update_app(&on, true, None, "0.2.2"));
        assert!(
            should_auto_update_app(&on, true, Some("0.2.1"), "0.2.2"),
            "a newer version is tried"
        );
        assert!(
            !should_auto_update_app(&on, true, Some("0.2.2"), "0.2.2"),
            "never twice: no restart loop"
        );
        assert!(
            !should_auto_update_app(&on, false, None, "0.2.2"),
            "a read-only install waits for the user"
        );

        let off = Settings { auto_update_app: false, ..Settings::default() };
        assert!(!should_auto_update_app(&off, true, None, "0.2.2"));
        let no_checks = Settings { check_app_updates: false, ..Settings::default() };
        assert!(!should_auto_update_app(&no_checks, true, None, "0.2.2"));
    }

    #[test]
    fn settings_saved_before_auto_update_existed_turn_it_on() {
        let old: Settings = serde_json::from_str(r#"{"check_app_updates": true}"#).unwrap();
        assert!(old.auto_update_app);
    }

    #[tokio::test]
    async fn sync_now_retries_an_account_waiting_out_a_backoff() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let root = tempfile::tempdir().unwrap();
        let sv_dir = root.path().join("_forever_/WTF/Account/ACC/SavedVariables");
        std::fs::create_dir_all(&sv_dir).unwrap();
        let saved_variables = sv_dir.join("SideQuest.lua");
        std::fs::write(
            &saved_variables,
            "SideQuestDB = {\n    [\"lastExport\"] = \"SQX1:abc\",\n}\n",
        )
        .unwrap();

        let mut engine = test_engine(server.uri(), vec![root.path().to_path_buf()]);
        engine.backoff.insert(
            saved_variables,
            BackoffEntry {
                failures: 3,
                next_attempt_at: Utc::now() + chrono::Duration::minutes(5),
            },
        );

        engine.rescan_and_sync(false).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 0, "auto sync waits");

        engine.rescan_and_sync(true).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "Sync now retries");
        assert!(engine.notices.iter().any(|n| n.level == NoticeLevel::Error));
        assert!(!engine.notices.iter().any(|n| n.text == NOTHING_NEW_TO_SYNC));
    }
}
