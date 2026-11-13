//! A small borderless, transparent popover opened from the tray's "Quick panel" menu item: a
//! single chamber with a 72px strip of the Courier's Loft painting at its top, a ribbon carrying
//! the app's state (WATCHING / PAUSED / SIGN IN), the freshness line, Sync now / Pause, the next
//! raid as one quest-log row, the addon line, and "Open SideQuest Companion". Officer-only rows
//! hide for members. Dismissed by Escape or by losing OS focus.

use std::cell::RefCell;
use std::time::Duration;

use gpui_kit::base::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Pixels, Point, Render, Size, Styled as _,
    StyledImage as _, Window, WindowBounds, WindowKind, WindowOptions, div, img, linear_color_stop,
    linear_gradient, point, px, size,
};

use crate::engine::EngineHandle;
use crate::state::{AccountState, Command, InstalledAddon, ReleaseCheck, Snapshot, SyncPhase};
use crate::ui::format::{ago, clock, event_kind_label, until, weekday};
use crate::ui::theme::palette;
use crate::ui::widgets::{chamber_with, freshness_line, quest_log_row, sub_heading};

const PANEL_WIDTH: f32 = 320.;
const PANEL_HEIGHT: f32 = 412.;
const STRIP_HEIGHT: f32 = 84.;

thread_local! {
    static OPEN_PANEL: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
}

/// Opens the quick panel near the tray, or closes it if one is already open (toggle). `demo`
/// pins the panel to a demo snapshot instead of following the engine (for `--demo` screenshots).
pub fn toggle(engine: EngineHandle, demo: Option<Snapshot>, cx: &mut App) {
    let existing = OPEN_PANEL.with(|cell| *cell.borrow());
    if let Some(handle) = existing {
        OPEN_PANEL.with(|cell| *cell.borrow_mut() = None);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        return;
    }

    let panel_size = size(px(PANEL_WIDTH), px(PANEL_HEIGHT));
    let origin = panel_origin(panel_size, cx);

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size: panel_size })),
        window_min_size: None,
        titlebar: None,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        show: true,
        focus: true,
        #[cfg(not(target_os = "linux"))]
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };

    let Ok((handle, _)) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| QuickPanel::new(engine.clone(), demo.clone(), window, cx))
    }) else {
        tracing::warn!("tray: failed to open the quick panel");
        return;
    };

    tracing::info!("tray: opened the quick panel");
    OPEN_PANEL.with(|cell| *cell.borrow_mut() = Some(handle));
}

/// Top-right under the menu bar on macOS, bottom-right above the taskbar elsewhere. gpui-tray
/// doesn't report the click position on any backend we target, so this is a fixed corner.
fn panel_origin(panel_size: Size<Pixels>, cx: &mut App) -> Point<Pixels> {
    let Some(display) = cx.primary_display() else {
        return point(px(40.), px(40.));
    };
    let bounds = display.bounds();
    if cfg!(target_os = "macos") {
        point(bounds.right() - panel_size.width - px(12.), bounds.top() + px(32.))
    } else {
        point(
            bounds.right() - panel_size.width - px(12.),
            bounds.bottom() - panel_size.height - px(56.),
        )
    }
}

struct QuickPanel {
    engine: EngineHandle,
    snapshot: Snapshot,
    focus: FocusHandle,
    /// True once the panel has had a moment to receive OS activation. Dismiss-on-blur is
    /// ignored until then, so a panel opened before the app itself is frontmost (e.g. right at
    /// launch) doesn't immediately close itself on the activation event that follows.
    settled: bool,
}

impl QuickPanel {
    fn new(
        engine: EngineHandle,
        demo: Option<Snapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        cx.observe_window_activation(window, |this, window, _cx| {
            if this.settled && !window.is_window_active() {
                tracing::info!("tray: quick panel lost OS focus, closing");
                OPEN_PANEL.with(|cell| *cell.borrow_mut() = None);
                window.remove_window();
            }
        })
        .detach();

        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(250)).await;
            let _ = this.update(cx, |this, _| this.settled = true);
        })
        .detach();

        let pinned = demo.is_some();
        let engine_poll = engine.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(()) = this.update(cx, |this, cx| {
                    if !pinned {
                        this.snapshot = engine_poll.snapshot();
                    }
                    cx.notify();
                }) else {
                    return;
                };
            }
        })
        .detach();

        let snapshot = demo.unwrap_or_else(|| engine.snapshot());
        Self { snapshot, engine, focus, settled: false }
    }

    fn close(&mut self, window: &mut Window) {
        OPEN_PANEL.with(|cell| *cell.borrow_mut() = None);
        window.remove_window();
    }
}

impl Render for QuickPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine_sync = self.engine.clone();
        let engine_pause = self.engine.clone();
        let paused = matches!(self.snapshot.sync.phase, SyncPhase::Paused);
        let uploading = matches!(self.snapshot.sync.phase, SyncPhase::Uploading);
        let signed_in = matches!(self.snapshot.account, AccountState::SignedIn(_));

        let state_label = if !signed_in {
            "SIGN IN"
        } else {
            match self.snapshot.sync.phase {
                SyncPhase::Paused => "PAUSED",
                _ => "WATCHING",
            }
        };

        // The panel has no toast area, so the newest notice (a sync result, while it lasts) takes
        // the freshness line's place: a Sync now click always gets an answer here.
        let freshness = if signed_in {
            match (self.snapshot.notices.last(), self.snapshot.sync.last_success) {
                (Some(notice), _) => notice.text.clone(),
                (None, Some(at)) => format!("Synced {}", ago(at)),
                (None, None) => "Nothing synced yet".to_owned(),
            }
        } else {
            "Updates and event reminders are on.".to_owned()
        };

        let next_event = self.snapshot.events.upcoming.iter().find(|e| !e.cancelled).map(|event| {
            let meta = format!(
                "{} \u{b7} {} \u{b7} {}",
                event_kind_label(&event.kind),
                event.location,
                until(event.starts_at)
            );
            quest_log_row(
                "quick-event",
                event.my_status.is_some(),
                event.title.clone(),
                meta,
                clock(event.starts_at),
                weekday(event.starts_at),
            )
        });

        let latest_version = match &self.snapshot.addon_release {
            ReleaseCheck::Known { latest, .. } => Some(latest.version.clone()),
            _ => None,
        };
        let addon_outdated = self.snapshot.installs.iter().any(|install| {
            install.flavors.iter().any(|flavor| match (&flavor.addon, &latest_version) {
                (InstalledAddon::Installed { version, .. }, Some(latest)) => {
                    crate::release::compare(version, latest)
                        == crate::release::VersionStatus::Outdated
                }
                _ => false,
            })
        });
        let addon_line = if addon_outdated {
            div().text_xs().text_color(palette::gold()).child("Addon update available")
        } else {
            div().text_xs().text_color(palette::mist2()).child("Addon up to date")
        };

        let content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(freshness_line(freshness))
            .when(signed_in, |d| {
                d.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("quick-sync-now")
                                .small()
                                .primary()
                                .disabled(uploading)
                                .label(if uploading { "Syncing\u{2026}" } else { "Sync now" })
                                .on_click(move |_, _, _| {
                                    tracing::info!("tray: quick panel Sync now clicked");
                                    engine_sync.send(Command::SyncNow);
                                }),
                        )
                        .child(
                            Button::new("quick-pause-resume")
                                .small()
                                .label(if paused { "Resume" } else { "Pause" })
                                .on_click(move |_, _, _| {
                                    tracing::info!(
                                        resuming = paused,
                                        "tray: quick panel pause/resume clicked"
                                    );
                                    engine_pause.send(Command::SetPaused(!paused));
                                }),
                        ),
                )
            })
            .child(sub_heading("Next event"))
            .child(next_event.map(IntoElement::into_any_element).unwrap_or_else(|| {
                div()
                    .text_sm()
                    .text_color(palette::mist2())
                    .child("No events coming up.")
                    .into_any_element()
            }))
            .child(sub_heading("Addon"))
            .child(addon_line)
            .child(div().pt_1().child(
                Button::new("quick-open-app").small().label("Open SideQuest Companion").on_click(
                    move |_, window, cx| {
                        tracing::info!("tray: quick panel Open SideQuest Companion clicked");
                        super::show_main_window(cx);
                        OPEN_PANEL.with(|cell| *cell.borrow_mut() = None);
                        window.remove_window();
                    },
                ),
            ));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette::night())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, _cx| {
                if event.keystroke.key.as_str() == "escape" {
                    tracing::info!("tray: quick panel dismissed via Escape");
                    this.close(window);
                }
            }))
            // The Courier's Loft strip, dissolving into night; the chamber's ribbon (the app's
            // state) sits across its lower edge.
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .h(px(STRIP_HEIGHT))
                    .overflow_hidden()
                    .child(
                        img("rooms/courier-loft.webp")
                            .w_full()
                            .h_full()
                            .object_fit(gpui_kit::ObjectFit::Cover),
                    )
                    .child(div().absolute().inset_0().bg(linear_gradient(
                        180.,
                        linear_color_stop(palette::night().opacity(0.05), 0.),
                        linear_color_stop(palette::night(), 1.),
                    ))),
            )
            .child(
                div()
                    .flex_1()
                    .mt(px(-14.))
                    .px(px(10.))
                    .pb(px(10.))
                    .child(chamber_with(state_label, content)),
            )
    }
}
