//! Addon update controls shared by the Overview and Addon pages: a "Check for updates" button, a
//! chevron dropdown of released versions (for rollback), and the "pinned to an older version" notice
//! that replaces the usual Install/Update button while a rollback is in effect.

use std::path::PathBuf;

use gpui_kit::base::Disableable as _;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Div, IntoElement, ParentElement as _, Styled as _, div};

use crate::engine::EngineHandle;
use crate::state::{Command, Release};
use crate::ui::theme::palette;

/// "Check for updates" (or "Checking…", disabled, while a check is already in flight).
pub fn check_updates_button(
    id: impl Into<gpui_kit::ElementId>,
    engine: EngineHandle,
    checking: bool,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .small()
        .disabled(checking)
        .label(if checking { "Checking\u{2026}" } else { "Check for updates" })
        .tooltip("Check GitHub for the latest addon release")
        .on_click(move |_, _, _| engine.send(Command::CheckAddonRelease))
}

/// A small chevron icon button opening a dropdown of `releases` (newest first) for `flavor`; picking
/// one installs exactly that version. The installed version (if any) is checked; the newest is
/// labelled "(latest)". Renders nothing if there is nothing to pick from.
pub fn version_dropdown(
    id: impl Into<gpui_kit::ElementId>,
    flavor: PathBuf,
    releases: &[Release],
    installed_version: Option<&str>,
    engine: EngineHandle,
) -> Option<impl IntoElement> {
    if releases.is_empty() {
        return None;
    }
    let latest_version = releases.first().map(|r| r.version.clone());
    let items: Vec<(String, String, bool)> = releases
        .iter()
        .map(|r| {
            let checked = installed_version == Some(r.version.as_str());
            let label = if Some(&r.version) == latest_version.as_ref() {
                format!("v{} (latest)", r.version)
            } else {
                format!("v{}", r.version)
            };
            (r.version.clone(), label, checked)
        })
        .collect();
    Some(
        Button::new(id)
            .ghost()
            .small()
            .icon(IconName::ChevronDown)
            .tooltip("Install a different released version")
            .dropdown_menu(move |menu, _, _| {
                items.iter().fold(menu, |menu, (version, label, checked)| {
                    let engine = engine.clone();
                    let flavor = flavor.clone();
                    let version = version.clone();
                    menu.item(PopupMenuItem::new(label.clone()).checked(*checked).on_click(
                        move |_, _, _| {
                            engine.send(Command::InstallAddon {
                                flavor: flavor.clone(),
                                version: Some(version.clone()),
                            })
                        },
                    ))
                })
            }),
    )
}

/// Replaces the Install/Update button while `pin` is set: a line naming the pinned version plus a
/// "Back to latest" button that installs the latest release and clears the pin.
pub fn pinned_notice(
    id: impl Into<gpui_kit::ElementId>,
    pin: &str,
    flavor: PathBuf,
    engine: EngineHandle,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(palette::mist2())
                .child(format!("Pinned to v{pin} \u{2014} auto-update paused")),
        )
        .child(
            Button::new(id)
                .ghost()
                .small()
                .label("Back to latest")
                .tooltip("Install the latest release and resume auto-update")
                .on_click(move |_, _, _| {
                    engine.send(Command::InstallAddon { flavor: flavor.clone(), version: None })
                }),
        )
}
