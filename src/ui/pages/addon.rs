use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, px};

use crate::engine::EngineHandle;
use crate::release::{self, VersionStatus};
use crate::state::{Command, InstalledAddon, ReleaseCheck, Snapshot};
use crate::ui::addon_widgets;
use crate::ui::format::ago;
use crate::ui::theme::palette;
use crate::ui::widgets::{Tone, chamber_with, empty_state, item_tooltip, state_tag};

pub fn render(snapshot: &Snapshot, engine: &EngineHandle) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(release_chamber(snapshot, engine))
        .child(flavors_chamber(snapshot, engine))
}

fn release_chamber(snapshot: &Snapshot, engine: &EngineHandle) -> Div {
    let checking = matches!(snapshot.addon_release, ReleaseCheck::Checking);
    let header = div().flex().justify_end().child(addon_widgets::check_updates_button(
        "check-addon-updates",
        engine.clone(),
        checking,
    ));
    let body = match &snapshot.addon_release {
        ReleaseCheck::NotChecked => {
            empty_state("Check for the latest addon release above.").into_any_element()
        }
        ReleaseCheck::Checking => {
            empty_state("Checking GitHub for the latest release\u{2026}").into_any_element()
        }
        ReleaseCheck::Failed(err) => div()
            .text_sm()
            .text_color(palette::warm_error())
            .child(format!("Couldn't check for updates: {err}"))
            .into_any_element(),
        ReleaseCheck::Known { latest, checked_at } => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .text_color(palette::ink())
                            .child(format!("v{}", latest.version)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette::mist2())
                            .child(format!("checked {}", ago(*checked_at))),
                    ),
            )
            .child(TextView::markdown("addon-release-notes", latest.notes.clone()))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        Button::new("addon-release-github")
                            .link()
                            .label("GitHub release")
                            .on_click({
                                let url = latest.html_url.clone();
                                move |_, _, cx| cx.open_url(&url)
                            }),
                    )
                    .child(
                        Button::new("addon-release-curseforge")
                            .link()
                            .label("CurseForge")
                            .on_click(|_, _, cx| {
                                cx.open_url("https://www.curseforge.com/projects/1773662")
                            }),
                    ),
            )
            .into_any_element(),
    };
    chamber_with("Latest Release", div().flex().flex_col().gap_3().child(header).child(body))
}

fn flavors_chamber(snapshot: &Snapshot, engine: &EngineHandle) -> Div {
    let latest_version = match &snapshot.addon_release {
        ReleaseCheck::Known { latest, .. } => Some(latest.version.clone()),
        _ => None,
    };
    let release_notes: Option<SharedString> = match &snapshot.addon_release {
        ReleaseCheck::Known { latest, .. } if !latest.notes.trim().is_empty() => {
            let summary = latest
                .notes
                .lines()
                .map(|line| line.trim().trim_start_matches(['-', '\u{2022}']).trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            Some(format!("\u{201c}{summary}\u{201d}").into())
        }
        _ => None,
    };

    let mut cards = div().flex().flex_col().gap_3();
    let mut any = false;
    let pin = snapshot.settings.addon_pinned_version.clone();
    for install in &snapshot.installs {
        for flavor in &install.flavors {
            any = true;
            let installed_version = match &flavor.addon {
                InstalledAddon::Missing => None,
                InstalledAddon::Installed { version, .. } => Some(version.clone()),
            };
            let (installed_label, status) = match &installed_version {
                None => ("not installed".to_owned(), None),
                Some(version) => {
                    let status = latest_version.as_deref().map(|l| release::compare(version, l));
                    (format!("v{version}"), status)
                }
            };
            let flavor_path = flavor.path.clone();
            let engine = engine.clone();
            let (pill_label, pill_tone) = match status {
                Some(VersionStatus::UpToDate) => ("Up to date", Tone::Good),
                Some(VersionStatus::Ahead) => ("Ahead of latest", Tone::Neutral),
                Some(VersionStatus::Development) => ("Dev checkout", Tone::Neutral),
                Some(VersionStatus::Outdated) | Some(VersionStatus::Unknown) | None => {
                    ("Unknown", Tone::Neutral)
                }
            };
            let needs_action = matches!(flavor.addon, InstalledAddon::Missing)
                || matches!(status, Some(VersionStatus::Outdated));
            let pinned_and_outdated =
                pin.is_some() && matches!(status, Some(VersionStatus::Outdated));
            let latest_label = latest_version
                .as_deref()
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| "unknown".to_owned());

            let version_menu = addon_widgets::version_dropdown(
                format!("addon-version-menu-{}", flavor.dir_name),
                flavor.path.clone(),
                &snapshot.addon_releases,
                installed_version.as_deref(),
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
            } else if needs_action {
                Button::new(format!("addon-install-{}", flavor.dir_name))
                    .primary()
                    .small()
                    .label("Install update")
                    .tooltip("Install the latest signed release from GitHub")
                    .on_click(move |_, _, _| {
                        engine.send(Command::InstallAddon {
                            flavor: flavor_path.clone(),
                            version: None,
                        })
                    })
                    .into_any_element()
            } else {
                state_tag(pill_label, pill_tone).into_any_element()
            };

            let card = item_tooltip(
                "SideQuest Guild",
                format!(
                    "Version {}",
                    latest_version.clone().unwrap_or_else(|| "unknown".to_owned())
                ),
                vec![
                    ("Installed".into(), installed_label.into()),
                    ("Latest".into(), latest_label.into()),
                ],
                release_notes.clone(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette::mist2())
                                    .child(flavor.dir_name.clone()),
                            )
                            .children(version_menu),
                    )
                    .child(action),
            )
            .when(snapshot.wow_running && needs_action, |d| {
                d.child(div().text_xs().text_color(palette::mist2()).child(
                    "WoW is running \u{2014} type /reload in game after the update to load it",
                ))
            });

            cards = cards.child(
                div()
                    .bg(palette::well())
                    .border_1()
                    .border_color(palette::stone())
                    .p(px(14.))
                    .child(card),
            );
        }
    }
    let body = if any {
        cards.into_any_element()
    } else {
        empty_state("No WoW folder found yet. Add it in Settings \u{2192} WoW Folders.")
            .into_any_element()
    };
    chamber_with("Installs", body)
}
