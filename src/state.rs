//! The contract between the engine (background sync, updates, reminders) and the UI.
//!
//! The engine owns a [`Snapshot`] behind `Arc<RwLock<_>>` and rewrites it as things change; the UI only reads it and
//! sends [`Command`]s. Nothing in here does I/O.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Production site. Users can point the app elsewhere (staging, a local API) in Settings.
pub const DEFAULT_SITE_URL: &str = "https://sidequest-guild.com";

/// Everything the user can change in Settings. Persisted as `settings.json` in the config dir.
///
/// Secrets (the sync token, the private calendar link) are never stored here; they live in the OS keyring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub site_url: String,
    /// Extra WoW install folders the user added by hand (the folder holding `_classic_`, `_retail_`, ...).
    pub wow_roots: Vec<PathBuf>,
    /// Look in the usual install places, Battle.net's list of installed games and the Windows registry as well
    /// as `wow_roots`.
    pub auto_detect: bool,
    /// Upload a new export as soon as WoW writes it.
    pub auto_sync: bool,
    /// After an upload, fetch the game import and stage it in SavedVariables while WoW is closed.
    pub stage_imports: bool,
    pub check_addon_updates: bool,
    /// Install addon updates without asking, WoW running or not (a running game loads them on /reload). On by
    /// default. Saved under a new key so installs that saved the old default (off) start with it on.
    #[serde(rename = "auto_install_addon")]
    pub auto_update_addon: bool,
    pub check_app_updates: bool,
    /// Download and install a newer app version as soon as a check finds it, then restart into it.
    pub auto_update_app: bool,
    pub reminders: ReminderSettings,
    pub launch_at_login: bool,
    /// Start with only the tray icon, no window.
    pub start_hidden: bool,
    /// Closing the window keeps the app running in the tray.
    pub close_to_tray: bool,
    pub notify_on_sync: bool,
    pub notify_on_error: bool,
    /// Accounts (by SavedVariables path) the user switched off; they are never uploaded from or written to.
    pub disabled_accounts: Vec<PathBuf>,
    /// UI zoom, 0.75..=1.5.
    pub ui_scale: f32,
    /// Switch rooms without the painting crossfade/push-in. The UI also honours the OS "reduce motion" setting.
    pub reduce_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            site_url: DEFAULT_SITE_URL.to_owned(),
            wow_roots: Vec::new(),
            auto_detect: true,
            auto_sync: true,
            stage_imports: true,
            check_addon_updates: true,
            auto_update_addon: true,
            check_app_updates: true,
            auto_update_app: true,
            reminders: ReminderSettings::default(),
            launch_at_login: false,
            start_hidden: false,
            close_to_tray: true,
            notify_on_sync: true,
            notify_on_error: true,
            disabled_accounts: Vec::new(),
            ui_scale: 1.0,
            reduce_motion: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReminderSettings {
    pub enabled: bool,
    pub scope: ReminderScope,
    /// Minutes before the start time to notify.
    pub lead_minutes: u32,
    /// Event kinds (the website's `EventKind` values: raid, dungeon, ...) to remind about.
    pub kinds: Vec<String>,
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            scope: ReminderScope::SignedUp,
            lead_minutes: 30,
            kinds: vec!["raid".to_owned()],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderScope {
    /// Only events the user answered Going or Maybe to. Needs the private calendar link.
    SignedUp,
    /// Every scheduled guild event of the chosen kinds (public events list).
    AllEvents,
}

/// What the UI renders. Cloned out of the lock once per frame.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub settings: Settings,
    pub token: TokenState,
    /// Whether a private calendar link is available (saved by hand, or provided by the site after sign-in).
    pub calendar_linked: bool,
    /// Browser sign-in ("device authorization") progress.
    pub login: LoginState,
    /// Who is signed in, as the website sees them. The UI shows only the gate unless this is `SignedIn`.
    pub account: AccountState,
    pub connection: ConnectionState,
    pub sync: SyncStatus,
    pub installs: Vec<WowInstall>,
    /// Newest first, capped (the engine keeps the last 500).
    pub history: Vec<HistoryEntry>,
    pub addon_release: ReleaseCheck,
    pub app_update: AppUpdate,
    pub events: EventsState,
    pub wow_running: bool,
    /// Short-lived messages for the UI's toast area; the engine drops them after a few seconds.
    pub notices: Vec<Notice>,
    pub app_version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TokenState {
    #[default]
    Missing,
    /// `masked` shows the prefix and last 4 characters, e.g. `sqs_…a1B2`.
    Present { masked: String },
}

/// The signed-in person, from `GET /api/epgp/sync-tokens/me`. Nothing about the guild is shown before `SignedIn`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AccountState {
    /// No token saved: show the gate.
    #[default]
    SignedOut,
    /// A token is saved and being checked against the site.
    Checking,
    SignedIn(Member),
    /// The token is valid but its owner isn't a guild member (or left the guild). The token has been discarded.
    NotInGuild,
    /// The site couldn't be reached to verify a saved token. The UI keeps the gate up with this message and a retry.
    Unverified(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// Officers can upload raid nights and download the game import; members get updates and reminders only.
    pub is_officer: bool,
}

/// Browser sign-in: the app asks the site for a one-time code, opens the authorize page in the browser, and polls
/// until the member clicks Authorize. The site then mints a token owned by that member and hands it over once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LoginState {
    #[default]
    Idle,
    /// Asking the site for a code.
    Starting,
    /// The browser is open on `verification_uri_complete`; `user_code` (e.g. `ABCD-EFGH`) is shown in the app and on
    /// the page so the user can check they match.
    Waiting { user_code: String, verification_uri_complete: String, expires_at: DateTime<Utc> },
    /// Signed in; the token is saved and tested. Cleared back to `Idle` after a few seconds.
    Done,
    /// Denied, expired, the site doesn't support browser sign-in yet (404), or a network error. Plain-English text.
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConnectionState {
    #[default]
    Unknown,
    Checking,
    /// The site accepted the token.
    Ok,
    /// 401: missing, wrong or revoked token.
    Unauthorized(String),
    /// 403: the token's owner is no longer an officer.
    Forbidden(String),
    /// Network error, timeout, or a 5xx.
    Unreachable(String),
}

#[derive(Debug, Clone, Default)]
pub struct SyncStatus {
    pub phase: SyncPhase,
    pub last_success: Option<DateTime<Utc>>,
    pub last_attempt: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SyncPhase {
    /// Nothing to do yet (first scan pending).
    #[default]
    Starting,
    /// Watching SavedVariables for changes.
    Watching,
    Uploading,
    /// Auto-sync switched off in the tray or Settings.
    Paused,
    /// Can't sync until the user fixes this (no token, no WoW install found, ...).
    NeedsSetup(String),
    /// The last attempt failed; the engine retries with backoff.
    Error(String),
}

/// A WoW install root, e.g. `/Applications/World of Warcraft`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WowInstall {
    pub root: PathBuf,
    pub source: InstallSource,
    pub flavors: Vec<Flavor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    Detected,
    UserAdded,
}

/// One game flavor folder inside an install, e.g. `_forever_`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flavor {
    /// The folder name, e.g. `_forever_`.
    pub dir_name: String,
    pub path: PathBuf,
    pub addon: InstalledAddon,
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledAddon {
    Missing,
    /// `version` is the `.toc` `## Version` value. A development checkout reads `@project-version@`.
    Installed {
        version: String,
        path: PathBuf,
    },
}

/// One `WTF/Account/<NAME>` folder with a `SideQuest.lua`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub saved_variables: PathBuf,
    pub enabled: bool,
    pub modified: Option<DateTime<Utc>>,
    /// What the addon recorded and hasn't exported/cleared yet. `None` if the file couldn't be read.
    pub pending: Option<PendingCounts>,
    /// The SQX1 export currently in the file, by SHA-256; `None` if there is no export.
    pub export_hash: Option<String>,
    /// Whether that export has already been uploaded.
    pub export_uploaded: bool,
    /// An `SQI1:` import is staged in the file and the addon hasn't applied it yet.
    pub inbox_staged: bool,
    /// When the addon's standings were generated (`SideQuestDB.standings.generated_at`).
    pub standings_generated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingCounts {
    pub sessions: usize,
    pub encounters: usize,
    pub loot: usize,
    pub awards: usize,
    pub roster: bool,
    pub professions: usize,
    pub signups: usize,
    pub bank: usize,
    pub announcements: usize,
    pub recruits: usize,
    pub member_notes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub at: DateTime<Utc>,
    pub kind: HistoryKind,
    pub outcome: Outcome,
    /// What it was about, e.g. `_forever_ · MYACCOUNT` or `addon v0.3.0`.
    pub subject: String,
    /// One line for the list, e.g. `2 sessions, 14 loot, 31 awards`.
    pub summary: String,
    /// Full detail for the expanded row (the site's response, the error text).
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryKind {
    Upload,
    ImportStaged,
    AddonInstalled,
    AppUpdated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Skipped,
    Failed,
}

/// The latest addon release on GitHub (the same zip CurseForge gets) and how each flavor compares.
#[derive(Debug, Clone, Default)]
pub enum ReleaseCheck {
    #[default]
    NotChecked,
    Checking,
    Known {
        latest: Release,
        checked_at: DateTime<Utc>,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Without the leading `v`, e.g. `0.3.0`.
    pub version: String,
    pub tag: String,
    pub published_at: Option<DateTime<Utc>>,
    pub notes: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Default)]
pub enum AppUpdate {
    #[default]
    NotChecked,
    Checking,
    UpToDate {
        checked_at: DateTime<Utc>,
    },
    Available(Release),
    Downloading {
        progress: f32,
    },
    /// Installed; takes effect on restart.
    ReadyToRestart(Release),
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct EventsState {
    pub upcoming: Vec<UpcomingEvent>,
    pub fetched_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpcomingEvent {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub location: String,
    /// The user's own answer (`signed`, `tentative`, `absent`), when known from the calendar link.
    pub my_status: Option<String>,
    pub cancelled: bool,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub id: u64,
    pub level: NoticeLevel,
    pub text: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// What the UI (window or tray) asks the engine to do.
#[derive(Debug, Clone)]
pub enum Command {
    /// Rescan every account and upload whatever is new, ignoring the pause switch.
    SyncNow,
    SetPaused(bool),
    /// Persist and apply new settings (rescans installs if the roots changed; toggles launch-at-login).
    SaveSettings(Settings),
    /// Store a sync token in the keyring and test it against the site (the manual "paste a token" path).
    SetToken(String),
    /// Start browser sign-in (opens the authorize page) and poll until it finishes.
    BeginBrowserLogin,
    CancelBrowserLogin,
    /// Open the authorize page again for the current code.
    ReopenBrowserLogin,
    /// Revoke the token on the site (best effort), forget it locally, and return to the gate.
    SignOut,
    /// Store (`Some`) or remove (`None`) the private calendar link.
    SetCalendarUrl(Option<String>),
    TestConnection,
    Rescan,
    CheckAddonRelease,
    /// Install the latest addon release into this flavor folder.
    InstallAddon {
        flavor: PathBuf,
    },
    /// Fetch the game import and stage it into this SavedVariables file now (WoW must be closed).
    StageImport {
        saved_variables: PathBuf,
    },
    CheckAppUpdate,
    InstallAppUpdate,
    RefreshEvents,
    DismissNotice(u64),
    /// Flush state to disk; the UI exits afterwards.
    Shutdown,
}
