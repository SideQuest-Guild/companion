use gpui_kit::base::Disableable as _;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Div, IntoElement, ParentElement as _, Styled as _, div};

use crate::engine::EngineHandle;
use crate::release::{self, VersionStatus};
use crate::state::{
    Account, Command, Flavor, InstalledAddon, PendingCounts, ReleaseCheck, Snapshot, SyncPhase,
    TokenState,
};
use crate::ui::addon_widgets;
use crate::ui::format::{ago, clock, event_kind_label, until, weekday};
use crate::ui::theme::palette;
use crate::ui::widgets::{
    Tone, chamber_with, empty_state, freshness_line, quest_log_row, state_tag,
};

pub fn render(snapshot: &Snapshot, engine: &EngineHandle) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(sync_status_chamber(snapshot, engine))
        .child(accounts_chamber(snapshot))
        .child(
            div()
                .flex()
                .items_stretch()
                .gap_4()
                .child(div().flex_1().child(addon_chamber(snapshot, engine).h_full()))
                .child(div().flex_1().child(next_event_chamber(snapshot).h_full())),
        )
}

fn sync_status_chamber(snapshot: &Snapshot, engine: &EngineHandle) -> Div {
    let (label, tone) = match &snapshot.sync.phase {
        SyncPhase::Starting => ("Starting", Tone::Neutral),
        SyncPhase::Watching => ("Watching", Tone::Good),
        SyncPhase::Uploading => ("Uploading", Tone::Good),
        SyncPhase::Paused => ("Paused", Tone::Warn),
        SyncPhase::NeedsSetup(_) => ("Needs setup", Tone::Warn),
        SyncPhase::Error(_) => ("Error", Tone::Bad),
    };
    let paused = matches!(snapshot.sync.phase, SyncPhase::Paused);
    let uploading = matches!(snapshot.sync.phase, SyncPhase::Uploading);
    let detail: Option<String> = match &snapshot.sync.phase {
        SyncPhase::NeedsSetup(reason) | SyncPhase::Error(reason) => Some(reason.clone()),
        _ => None,
    };

    let freshness = match snapshot.sync.last_success {
        Some(at) => freshness_line(format!("Synced {}", ago(at))),
        None => freshness_line("Never synced"),
    };

    let engine_sync = engine.clone();
    let engine_pause = engine.clone();

    chamber_with(
        "Sync Status",
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(state_tag(label, tone))
                    .child(freshness),
            )
            .when_some(detail, |this, detail| {
                this.child(div().text_sm().text_color(palette::warm_error()).child(detail))
            })
            .child(if matches!(snapshot.token, TokenState::Missing) {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        Button::new("sign-in")
                            .primary()
                            .large()
                            .label("Sign in with sidequest-guild.com")
                            .on_click(move |_, _, _| engine_sync.send(Command::BeginBrowserLogin)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette::mist2())
                            .child("Sign in to start syncing your raid data automatically."),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("sync-now")
                            .primary()
                            .disabled(uploading)
                            .label(if uploading { "Syncing\u{2026}" } else { "Sync now" })
                            .on_click(move |_, _, _| engine_sync.send(Command::SyncNow)),
                    )
                    .child(
                        Button::new("pause-resume")
                            .label(if paused { "Resume" } else { "Pause" })
                            .on_click(move |_, _, _| {
                                engine_pause.send(Command::SetPaused(!paused))
                            }),
                    )
                    .into_any_element()
            }),
    )
}

fn accounts_chamber(snapshot: &Snapshot) -> Div {
    let mut rows = div().flex().flex_col().gap_2();
    let mut any = false;
    for install in &snapshot.installs {
        for flavor in &install.flavors {
            for account in &flavor.accounts {
                any = true;
                rows = rows.child(account_row(flavor, account, snapshot.wow_running));
            }
        }
    }
    let body = if any {
        rows.into_any_element()
    } else {
        empty_state("No WoW accounts found yet.").into_any_element()
    };
    chamber_with("Accounts", div().flex().flex_col().gap_3().child(body))
}

fn account_row(flavor: &Flavor, account: &Account, wow_running: bool) -> Div {
    // extract() only fills `pending` when the whole file parsed, so a problem means no counts.
    let pending_summary = match (&account.pending, &account.read_problem) {
        (Some(pending), _) => pending_summary_text(pending),
        (None, Some(problem)) => format!("couldn't read the save: {problem}"),
        (None, None) => "nothing recorded yet".to_owned(),
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .py_2()
        .px_3()
        .bg(palette::night3())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div().text_sm().text_color(palette::ink()).child(account.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::mist2())
                                .child(flavor.dir_name.clone()),
                        )
                        .when(wow_running, |this| this.child(state_tag("WoW running", Tone::Warn))),
                )
                .child(div().text_xs().text_color(palette::mist2()).child(pending_summary))
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::mist2())
                        .child(account.saved_variables.display().to_string()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(state_tag(
                    if account.export_uploaded {
                        "Uploaded"
                    } else if account.export_hash.is_some() {
                        "Pending upload"
                    } else {
                        "No export"
                    },
                    if account.export_uploaded {
                        Tone::Good
                    } else if account.export_hash.is_some() {
                        Tone::Warn
                    } else {
                        Tone::Neutral
                    },
                ))
                .child(state_tag(
                    if flavor.inbox_staged_at.is_some() {
                        "Site data ready"
                    } else {
                        "No site data"
                    },
                    if flavor.inbox_staged_at.is_some() { Tone::Good } else { Tone::Neutral },
                )),
        )
}

fn pending_summary_text(p: &PendingCounts) -> String {
    let mut parts = Vec::new();
    if p.sessions > 0 {
        parts.push(format!("{} sessions", p.sessions));
    }
    if p.loot > 0 {
        parts.push(format!("{} loot", p.loot));
    }
    if p.awards > 0 {
        parts.push(format!("{} awards", p.awards));
    }
    if p.signups > 0 {
        parts.push(format!("{} signups", p.signups));
    }
    if p.roster {
        parts.push("roster".to_owned());
    }
    if parts.is_empty() { "nothing pending".to_owned() } else { parts.join(", ") }
}

fn addon_chamber(snapshot: &Snapshot, engine: &EngineHandle) -> Div {
    let latest_version = match &snapshot.addon_release {
        ReleaseCheck::Known { latest, .. } => Some(latest.version.clone()),
        _ => None,
    };
    let checking = matches!(snapshot.addon_release, ReleaseCheck::Checking);
    let checked_line = match &snapshot.addon_release {
        ReleaseCheck::Known { checked_at, .. } => Some(format!("checked {}", ago(*checked_at))),
        _ => None,
    };
    let pin = snapshot.settings.addon_pinned_version.clone();

    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .child(addon_widgets::check_updates_button("check-addon-updates", engine.clone(), checking))
        .when_some(checked_line, |this, line| this.child(freshness_line(line)));

    let mut body = div().flex().flex_col().gap_2();
    let mut any = false;
    for install in &snapshot.installs {
        for flavor in &install.flavors {
            any = true;
            let installed = match &flavor.addon {
                InstalledAddon::Missing => None,
                InstalledAddon::Installed { version, .. } => Some(version.clone()),
            };
            let status = match (&installed, &latest_version) {
                (Some(i), Some(l)) => Some(release::compare(i, l)),
                _ => None,
            };
            // Only a missing or older install offers the button; a test build ahead of the release or a dev
            // checkout doesn't, and neither does an install we can't compare (offline, odd version string).
            let needs_update =
                installed.is_none() || matches!(status, Some(VersionStatus::Outdated));
            let pinned_and_outdated =
                pin.is_some() && matches!(status, Some(VersionStatus::Outdated));
            let version_line = match (&installed, &latest_version) {
                (Some(i), Some(l)) if needs_update => format!("v{i} \u{2192} v{l} available"),
                (Some(i), _) => format!("v{i}"),
                (None, _) => "not installed".to_owned(),
            };
            let (tag_label, tag_tone) = match status {
                Some(VersionStatus::UpToDate) => ("Up to date", Tone::Good),
                Some(VersionStatus::Ahead) => ("Ahead of latest", Tone::Neutral),
                Some(VersionStatus::Development) => ("Dev checkout", Tone::Neutral),
                _ => ("Installed", Tone::Neutral),
            };
            let flavor_path = flavor.path.clone();
            let engine = engine.clone();
            let wow_running = snapshot.wow_running;
            let version_menu = addon_widgets::version_dropdown(
                format!("addon-version-menu-{}", flavor.dir_name),
                flavor.path.clone(),
                &snapshot.addon_releases,
                installed.as_deref(),
                engine.clone(),
            );
            let action = if pinned_and_outdated {
                addon_widgets::pinned_notice(
                    format!("addon-back-to-latest-{}", flavor.dir_name),
                    pin.as_deref().unwrap_or_default(),
                    flavor_path.clone(),
                    engine.clone(),
                )
                .into_any_element()
            } else if !needs_update {
                state_tag(tag_label, tag_tone).into_any_element()
            } else {
                Button::new(format!("update-addon-{}", flavor.dir_name))
                    .small()
                    .primary()
                    .label(if installed.is_none() { "Install" } else { "Update" })
                    .tooltip("Install the latest addon release")
                    .on_click(move |_, _, _| {
                        engine.send(crate::state::Command::InstallAddon {
                            flavor: flavor_path.clone(),
                            version: None,
                        })
                    })
                    .into_any_element()
            };
            body =
                body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(palette::ink())
                                                .child(flavor.dir_name.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(palette::mist2())
                                                        .child(version_line),
                                                )
                                                .children(version_menu),
                                        ),
                                )
                                .child(action),
                        )
                        .when(wow_running && needs_update, |this| {
                            this.child(div().text_xs().text_color(palette::mist2()).child(
                                "WoW is running \u{2014} type /reload in game after updating",
                            ))
                        })
                        .when_some(flavor.inbox_staged_at, |this, at| {
                            this.child(
                                div().text_xs().text_color(palette::mist2()).child(format!(
                                    "Site data ready, /reload to load ({})",
                                    ago(at)
                                )),
                            )
                        }),
                );
        }
    }
    let body = if any {
        body.into_any_element()
    } else {
        empty_state("No WoW folder found yet. Add it in Settings \u{2192} WoW Folders.")
            .into_any_element()
    };
    chamber_with("Addon", div().flex().flex_col().gap_3().child(header).child(body))
}

fn event_status(my_status: Option<&str>) -> (&'static str, Tone) {
    match my_status {
        Some("signed") => ("Going", Tone::Good),
        Some("tentative") => ("Maybe", Tone::Warn),
        Some("absent") => ("Can't go", Tone::Bad),
        _ => ("No answer", Tone::Neutral),
    }
}

fn next_event_chamber(snapshot: &Snapshot) -> Div {
    let next = snapshot.events.upcoming.iter().find(|e| !e.cancelled);
    let body = match next {
        Some(event) => {
            let done = event.my_status.is_some();
            let meta = format!(
                "{} \u{b7} {} \u{b7} {}",
                event_kind_label(&event.kind),
                event.location,
                until(event.starts_at)
            );
            let (label, tone) = event_status(event.my_status.as_deref());
            quest_log_row(
                "next-event",
                done,
                event.title.clone(),
                meta,
                clock(event.starts_at),
                weekday(event.starts_at),
            )
            .child(div().flex_shrink_0().child(state_tag(label, tone)))
            .into_any_element()
        }
        None => empty_state("No upcoming events.").into_any_element(),
    };
    chamber_with("Next Event", div().flex().flex_col().gap_3().child(body))
}
