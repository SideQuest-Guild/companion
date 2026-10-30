//! The window's root view: a single 44px row that is both the native title bar (traffic lights
//! vertically centred, window drag anywhere empty) and the nav (crest, wordmark, room links; our
//! own caption buttons on Windows/Linux); below it a compact room title; the room's painting runs
//! as a backdrop behind all of this from y=0, about half the window's height, dissolving into
//! night, with the current room's chambers overlapping it from roughly y=116; toasts from
//! `snapshot.notices`, and an app-update banner. Nothing is drawn around the macOS traffic lights
//! or along the window's edge; the OS owns the window's own corner radius and shadow. Until
//! `AccountState::SignedIn`, the gate is the only reachable screen.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::base::InteractiveElementExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, Div, Entity, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, PathBuilder, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Window, WindowControlArea, canvas, div, img, linear_color_stop, linear_gradient, point, px,
    relative,
};

use crate::engine::EngineHandle;
use crate::state::{AccountState, AppUpdate, Command, Notice, NoticeLevel, Snapshot};
use crate::ui::pages::{self, SettingsPage};
use crate::ui::theme::{self, DISPLAY_FONT, palette};
use crate::ui::widgets::tracked_caps;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    History,
    Addon,
    Raids,
    Settings,
}

impl Page {
    /// Parses the `--page` CLI flag's value (`overview`, `history`, `addon`, `raids`, `settings`),
    /// used to open directly on a page for screenshots and manual testing.
    pub fn from_flag(s: &str) -> Option<Self> {
        match s {
            "overview" => Some(Page::Overview),
            "history" => Some(Page::History),
            "addon" => Some(Page::Addon),
            "raids" => Some(Page::Raids),
            "settings" => Some(Page::Settings),
            _ => None,
        }
    }

    /// The room painting this page opens on.
    fn room_art(self) -> &'static str {
        match self {
            Page::Overview => "rooms/courier-loft.webp",
            Page::History => "rooms/ledger-chamber.webp",
            Page::Addon => "rooms/armory.webp",
            Page::Raids => "rooms/raid-hall.webp",
            Page::Settings => "rooms/round-table.webp",
        }
    }

    fn next(self) -> Self {
        let ix = PAGES.iter().position(|(p, _)| *p == self).unwrap_or(0);
        PAGES[(ix + 1) % PAGES.len()].0
    }
}

const PAGES: &[(Page, &str)] = &[
    (Page::Overview, "Overview"),
    (Page::History, "History"),
    (Page::Addon, "Addon"),
    (Page::Raids, "Raids"),
    (Page::Settings, "Settings"),
];

/// The painting backdrop: about half the window's height (a fraction of it, not a fixed pixel
/// count, so it tracks the window at any size), dissolving into night over its lower 60%. The
/// foreground (nav, compact title, chambers) overlaps it starting at the nav row.
const ROOM_HEAD_FRACTION: f32 = 0.52;
const ROOM_HEAD_DISSOLVE_FRACTION: f32 = 0.6;
/// Height of the merged title-bar / nav row: 44px tall hit areas, native traffic lights
/// vertically centred within it (see `ui::mod::run`'s `traffic_light_position`, which centres a
/// ~16px dot cluster in this height the same way GPUI Kit centres it in its own 34px bar).
pub(crate) const NAV_ROW_HEIGHT: f32 = 44.;
/// Flow height reserved for the compact room title directly under the nav row; the first chamber
/// overlaps the painting starting around `NAV_ROW_HEIGHT + TITLE_BLOCK_HEIGHT` (~116px).
const TITLE_BLOCK_HEIGHT: f32 = 72.;
#[cfg(target_os = "macos")]
const CHROME_LEFT_PADDING: f32 = 80.;
#[cfg(not(target_os = "macos"))]
const CHROME_LEFT_PADDING: f32 = 16.;
/// Windows/Linux caption button hit area (DESIGN.md: 46x32).
const CAPTION_BUTTON_WIDTH: f32 = 46.;
const CAPTION_BUTTON_HEIGHT: f32 = 32.;

/// The "walking between rooms" moment's durations, ms.
const OLD_LAYER_MS: u64 = 420;
const NEW_LAYER_MS: u64 = 900;
const TITLE_LIFT_MS: u64 = 420;
const TITLE_LIFT_PX: f32 = 6.;

/// Canonical exponential ease-out: starts fast, decelerates smoothly to a stop.
fn ease_out_expo(delta: f32) -> f32 {
    if delta >= 1. { 1. } else { 1. - 2f32.powf(-10. * delta) }
}

pub struct AppRoot {
    engine: EngineHandle,
    demo: bool,
    snapshot: Snapshot,
    page: Page,
    /// The page being transitioned away from; carries the outgoing painting layer for
    /// `OLD_LAYER_MS`, then cleared. `None` outside a transition (including reduced motion).
    previous_page: Option<Page>,
    /// Bumped on every page change; keys the transition's animated layers so a fresh switch
    /// (even back to a page already visited) always replays the entrance, and a stray timer from
    /// a superseded switch can't clear a newer one's `previous_page`.
    transition_gen: u64,
    scroll_handle: ScrollHandle,
    settings: Entity<SettingsPage>,
    pub(crate) history_kind_filter: Option<crate::state::HistoryKind>,
    pub(crate) history_outcome_filter: Option<crate::state::Outcome>,
    pub(crate) history_expanded: Option<usize>,
}

impl AppRoot {
    pub fn new(
        engine: EngineHandle,
        dirty: Arc<AtomicBool>,
        demo_mode: Option<crate::ui::demo::DemoMode>,
        initial_page: Option<Page>,
        demo_cycle: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let demo = demo_mode.is_some();
        let snapshot = demo_mode.map(|mode| mode.snapshot()).unwrap_or_else(|| engine.snapshot());
        let settings = cx.new(|cx| SettingsPage::new(&snapshot, engine.clone(), window, cx));

        // The OS "reduce motion" setting can change while the window is open (e.g. the user
        // flips it in System Settings and comes back); re-read it whenever the window regains
        // activation, which is the cheapest reliable point to notice without polling.
        cx.observe_window_activation(window, |_this, _window, cx| {
            theme::refresh_system_reduce_motion();
            cx.notify();
        })
        .detach();

        if !demo {
            let dirty = dirty.clone();
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(150)).await;
                    if !dirty.swap(false, Ordering::AcqRel) {
                        continue;
                    }
                    let Ok(()) = this.update_in(cx, |this, _window, cx| {
                        this.snapshot = this.engine.snapshot();
                        cx.notify();
                    }) else {
                        return;
                    };
                }
            })
            .detach();
        }

        if demo_cycle {
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(2000)).await;
                    let Ok(()) = this.update_in(cx, |this, window, cx| {
                        this.go_to(this.page.next(), window, cx);
                    }) else {
                        return;
                    };
                }
            })
            .detach();
        }

        Self {
            engine,
            demo,
            snapshot,
            page: initial_page.unwrap_or(Page::Overview),
            previous_page: None,
            transition_gen: 0,
            scroll_handle: ScrollHandle::new(),
            settings,
            history_kind_filter: None,
            history_outcome_filter: None,
            history_expanded: None,
        }
    }

    fn reduce_motion(&self) -> bool {
        self.snapshot.settings.reduce_motion || theme::system_reduce_motion()
    }

    /// Switches rooms. With motion enabled, keeps the outgoing painting for `OLD_LAYER_MS` while
    /// the new one plays its own entrance (see `backdrop_painting`); with reduced motion, the
    /// switch is instant.
    fn go_to(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if page == self.page {
            return;
        }
        self.transition_gen += 1;
        let my_gen = self.transition_gen;
        if !self.reduce_motion() {
            self.previous_page = Some(self.page);
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(OLD_LAYER_MS)).await;
                let _ = this.update(cx, |this, cx| {
                    if this.transition_gen == my_gen {
                        this.previous_page = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.page = page;
        self.history_expanded = None;
        cx.notify();
    }

    /// The painting backdrop: runs behind the nav row, the compact title and the chambers from
    /// y=0, about half the window tall, with a top scrim so the nav reads and a bottom mask
    /// dissolving into night over its lower 60%. Scroll-linked drift (0.3x, capped at the head
    /// height) and the room-change crossfade both live here.
    fn backdrop_painting(&self, window: &Window) -> Div {
        let head_height = f32::from(window.viewport_size().height) * ROOM_HEAD_FRACTION;
        let art = self.page.room_art();
        let title = PAGES.iter().find(|(p, _)| *p == self.page).map(|(_, l)| *l).unwrap_or("");
        let reduce_motion = self.reduce_motion();

        // Scroll-linked drift: the painting translates down at 0.3x the scroll offset and fades
        // toward night, capped at the head height.
        let scrolled = if reduce_motion {
            0.
        } else {
            f32::from((-self.scroll_handle.offset().y).max(px(0.)).min(px(head_height)))
        };
        let drift_px = scrolled * 0.3;
        let night_progress = (scrolled / head_height).clamp(0., 1.);

        let mut layers: Vec<AnyElement> = Vec::new();
        if let Some(prev) = self.previous_page.filter(|_| !reduce_motion) {
            layers.push(painting_layer(
                PaintingLayer {
                    art: prev.room_art(),
                    key: format!("room-art-out-{}", self.transition_gen).into(),
                    head_height,
                    scale_from: 1.0,
                    scale_to: 1.04,
                    opacity_from: 1.0,
                    opacity_to: 0.0,
                    duration_ms: OLD_LAYER_MS,
                    drift_px: 0.,
                },
                gpui_kit::ease_out_quint(),
            ));
        }
        layers.push(if reduce_motion {
            img(art)
                .id(format!("room-art-{title}"))
                .absolute()
                .inset_0()
                .w_full()
                .h_full()
                .object_fit(gpui_kit::ObjectFit::Cover)
                .into_any_element()
        } else {
            painting_layer(
                PaintingLayer {
                    art,
                    key: format!("room-art-in-{}", self.transition_gen).into(),
                    head_height,
                    scale_from: 1.06,
                    scale_to: 1.0,
                    opacity_from: 0.4,
                    opacity_to: 1.0,
                    duration_ms: NEW_LAYER_MS,
                    drift_px,
                },
                ease_out_expo,
            )
        });

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(head_height))
            .overflow_hidden()
            .bg(palette::night())
            .children(layers)
            // Scroll-linked fade toward night, capped at the head height.
            .child(div().absolute().inset_0().bg(palette::night()).opacity(night_progress))
            // Top scrim: night at 70% fading to transparent over the first 56px, keeps the chrome
            // row legible.
            .child(div().absolute().top_0().left_0().right_0().h(px(56.)).bg(linear_gradient(
                180.,
                linear_color_stop(palette::night().opacity(0.7), 0.),
                linear_color_stop(palette::night().opacity(0.), 1.),
            )))
            // Bottom mask: dissolves the painting into night over its lower 60%.
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(head_height * ROOM_HEAD_DISSOLVE_FRACTION))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(palette::night().opacity(0.), 0.),
                        linear_color_stop(palette::night(), 1.),
                    )),
            )
    }

    /// Compact room title directly under the nav row: display caps ~18px, flanked by short
    /// hairline rules. Reserves a fixed `TITLE_BLOCK_HEIGHT` of flow space; the animated lift
    /// happens entirely inside that reserved box (an absolutely positioned child), so it never
    /// reflows the chambers below it.
    fn compact_room_title(&self) -> Div {
        let title = PAGES.iter().find(|(p, _)| *p == self.page).map(|(_, l)| *l).unwrap_or("");
        let reduce_motion = self.reduce_motion();

        let content = div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .gap_3()
            .child(hairline_rule(false, 28.))
            .child(
                div()
                    .font_family(DISPLAY_FONT)
                    .text_size(px(18.))
                    .text_color(palette::ink())
                    .child(tracked_caps(title)),
            )
            .child(hairline_rule(true, 28.));
        let content: AnyElement = if reduce_motion {
            content.into_any_element()
        } else {
            content
                .with_animation(
                    format!("room-title-anim-{}", self.transition_gen),
                    Animation::new(Duration::from_millis(TITLE_LIFT_MS))
                        .with_easing(gpui_kit::ease_out_quint()),
                    |el, delta| el.top(px(TITLE_LIFT_PX * (1. - delta))).opacity(delta),
                )
                .into_any_element()
        };

        // A soft night scrim behind the title (the website's `.scrim`), so a bright patch of painting (the
        // Round Table's stained glass) never sits behind the letters. Fades out at both ends; never a box.
        let half = |angle: f32| {
            div().w(px(220.)).h_full().bg(linear_gradient(
                angle,
                linear_color_stop(palette::night().opacity(0.), 0.),
                linear_color_stop(palette::night().opacity(0.62), 1.),
            ))
        };
        let scrim = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(half(90.))
            .child(half(270.));
        div().relative().flex_shrink_0().h(px(TITLE_BLOCK_HEIGHT)).child(scrim).child(content)
    }

    /// The crest + wordmark, left of the merged chrome row's content.
    fn nav_brand(&self) -> Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(img("plates/nav-crest.png").w(px(17.)).h(px(24.)))
            .child(
                div()
                    .font_family(DISPLAY_FONT)
                    .text_size(px(13.))
                    .text_color(palette::ink())
                    .child(tracked_caps("SideQuest Companion")),
            )
            .when(self.demo, |d| {
                d.child(
                    div().ml_2().text_xs().italic().text_color(palette::gold()).child("Demo data"),
                )
            })
    }

    /// The room links, right of the merged chrome row's content.
    fn nav_links(&mut self, cx: &mut Context<Self>) -> Div {
        let is_officer =
            matches!(&self.snapshot.account, AccountState::SignedIn(m) if m.is_officer);
        div().flex().items_center().gap_5().children(
            PAGES.iter().filter(|(page, _)| is_officer || *page != Page::History).map(
                |(page, label)| {
                    let active = *page == self.page;
                    let page = *page;
                    div()
                        .id(*label)
                        .flex()
                        .items_center()
                        .h(px(NAV_ROW_HEIGHT))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(if active { palette::ink() } else { palette::mist() })
                        .when(active, |d| d.border_b_1().border_color(palette::gold_deep()))
                        .child(*label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.go_to(page, window, cx);
                        }))
                },
            ),
        )
    }

    /// One 44px row that is both the native title bar (traffic lights vertically centred, see
    /// `ui::mod::run`; window drag anywhere empty) and the nav: crest + wordmark at left (after
    /// the traffic lights on macOS), room links at right. On Windows/Linux our own caption
    /// buttons sit at the far right. `nav` is `None` for the gate, which carries no links.
    fn chrome_row(
        &mut self,
        nav: Option<(Div, Div)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<Div> {
        let is_macos = cfg!(target_os = "macos");
        let is_linux = cfg!(target_os = "linux");
        let is_web = cfg!(target_family = "wasm");
        let state = window.use_state(cx, |_, _| ChromeRowState { should_move: false });

        div()
            .id("chrome-row")
            .relative()
            .flex_shrink_0()
            .h(px(NAV_ROW_HEIGHT))
            .flex()
            .items_center()
            .pl(px(CHROME_LEFT_PADDING))
            .when(is_macos, |d| d.on_double_click(|_, window, _| window.titlebar_double_click()))
            .when(is_linux, |d| d.on_double_click(|_, window, _| window.zoom_window()))
            .on_mouse_down_out(window.listener_for(&state, |state, _, _, _| {
                state.should_move = false;
            }))
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&state, |state, _, _, _| {
                    state.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&state, |state, _, _, _| {
                    state.should_move = false;
                }),
            )
            .on_mouse_move(window.listener_for(&state, |state, _, window, _| {
                if state.should_move {
                    state.should_move = false;
                    window.start_window_move();
                }
            }))
            .child(
                div()
                    .id("chrome-bar")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pr(if is_macos { px(16.) } else { px(0.) })
                    .when(!is_web, |d| d.window_control_area(WindowControlArea::Drag))
                    .when_some(nav, |d, (left, right)| d.child(left).child(right)),
            )
            .when(!is_macos, |d| d.child(self.caption_buttons(window, cx)))
    }

    /// Windows/Linux caption buttons: 46x32 hit areas, 1px line glyphs, hover fills stone-2,
    /// close hover fills error at 80%.
    fn caption_buttons(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let controls = window.window_controls();
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .when(controls.minimize, |d| {
                d.child(caption_button(
                    "caption-minimize",
                    CaptionKind::Minimize,
                    WindowControlArea::Min,
                    false,
                    |window| window.minimize_window(),
                    cx,
                ))
            })
            .when(controls.maximize, |d| {
                d.child(caption_button(
                    "caption-maximize",
                    CaptionKind::Maximize,
                    WindowControlArea::Max,
                    false,
                    |window| window.zoom_window(),
                    cx,
                ))
            })
            .child(caption_button(
                "caption-close",
                CaptionKind::Close,
                WindowControlArea::Close,
                true,
                |window| window.remove_window(),
                cx,
            ))
    }

    fn notices_overlay(&self, cx: &mut Context<Self>) -> Div {
        div().absolute().bottom(px(16.)).right(px(16.)).flex().flex_col().gap_2().children(
            self.snapshot.notices.iter().map(|notice| notice_toast(notice, &self.engine, cx)),
        )
    }

    fn update_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        match &self.snapshot.app_update {
            AppUpdate::Available(release) => Some(banner(
                format!("SideQuest Companion v{} is available.", release.version),
                "Install",
                cx.listener(|this, _, _, _| this.engine.send(Command::InstallAppUpdate)),
            )),
            AppUpdate::ReadyToRestart(release) => Some(banner(
                format!("v{} is ready \u{2014} restart to finish updating.", release.version),
                "Restart now",
                cx.listener(|this, _, window, cx| {
                    let engine = this.engine.clone();
                    window
                        .spawn(cx, async move |_| {
                            engine.shutdown_blocking();
                            if crate::self_update::can_self_update() {
                                let _ = crate::self_update::relaunch();
                            }
                            std::process::exit(0);
                        })
                        .detach();
                }),
            )),
            _ => None,
        }
    }
}

struct ChromeRowState {
    should_move: bool,
}

// Mirrors `gpui_component::TitleBar`'s own `TitleBarState`: `window.use_state` currently requires
// its state type to be usable as an `Entity`, which in practice means it needs a `Render` impl
// even though nothing ever renders it directly.
impl Render for ChromeRowState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone, Copy)]
enum CaptionKind {
    Minimize,
    Maximize,
    Close,
}

/// A 1px line glyph (minimize bar, maximize square, close cross), drawn with stroked paths rather
/// than a stock icon font, per `DESIGN.md`.
fn caption_glyph(kind: CaptionKind, color: Hsla) -> Div {
    div().absolute().inset_0().child(canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let mx = bounds.origin.x + bounds.size.width / 2.;
            let my = bounds.origin.y + bounds.size.height / 2.;
            let half = px(5.);
            let mut stroke = PathBuilder::stroke(px(1.));
            match kind {
                CaptionKind::Minimize => {
                    stroke.move_to(point(mx - half, my));
                    stroke.line_to(point(mx + half, my));
                }
                CaptionKind::Maximize => {
                    stroke.move_to(point(mx - half, my - half));
                    stroke.line_to(point(mx + half, my - half));
                    stroke.line_to(point(mx + half, my + half));
                    stroke.line_to(point(mx - half, my + half));
                    stroke.close();
                }
                CaptionKind::Close => {
                    stroke.move_to(point(mx - half, my - half));
                    stroke.line_to(point(mx + half, my + half));
                    stroke.move_to(point(mx + half, my - half));
                    stroke.line_to(point(mx - half, my + half));
                }
            }
            if let Ok(path) = stroke.build() {
                window.paint_path(path, color);
            }
        },
    ))
}

/// A single Windows/Linux caption button. On Windows the OS drives the click via the window's own
/// hit-testing once the area is marked (`window_control_area`); on Linux we drive it ourselves,
/// mirroring `gpui_component`'s own `ControlIcon`.
fn caption_button(
    id: &'static str,
    kind: CaptionKind,
    area: WindowControlArea,
    is_close: bool,
    action: impl Fn(&mut Window) + 'static,
    cx: &mut Context<AppRoot>,
) -> gpui_kit::Stateful<Div> {
    let is_windows = cfg!(target_os = "windows");
    let is_linux = cfg!(target_os = "linux");
    div()
        .id(id)
        .relative()
        .w(px(CAPTION_BUTTON_WIDTH))
        .h(px(CAPTION_BUTTON_HEIGHT))
        .flex_shrink_0()
        .cursor_pointer()
        .when(is_close, |d| d.hover(|s| s.bg(palette::warm_error().opacity(0.8))))
        .when(!is_close, |d| d.hover(|s| s.bg(palette::stone2())))
        .when(is_windows, |d| d.window_control_area(area))
        .when(is_linux, |d| {
            d.on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |_, _, window, cx| {
                cx.stop_propagation();
                action(window);
            }))
        })
        .child(caption_glyph(kind, palette::ink()))
}

/// Parameters for one `painting_layer` (grouped to keep the function under clippy's argument cap).
struct PaintingLayer {
    art: &'static str,
    key: SharedString,
    head_height: f32,
    scale_from: f32,
    scale_to: f32,
    opacity_from: f32,
    opacity_to: f32,
    duration_ms: u64,
    drift_px: f32,
}

/// One scaled, faded painting layer: the outgoing room's painting or the incoming one (see
/// `backdrop_painting`). There's no 2D transform for images in this GPUI version, so the "scale"
/// illusion is built the way a CSS `transform: scale` reads on a cover-fit background: the image
/// is oversized and offset to stay centred within the `head_height`-tall backdrop. `drift_px` (0
/// for the outgoing layer) adds the scroll-linked downward parallax on top.
fn painting_layer(layer: PaintingLayer, easing: impl Fn(f32) -> f32 + 'static) -> AnyElement {
    let PaintingLayer {
        art,
        key,
        head_height,
        scale_from,
        scale_to,
        opacity_from,
        opacity_to,
        duration_ms,
        drift_px,
    } = layer;
    img(art)
        .id(key.clone())
        .absolute()
        .object_fit(gpui_kit::ObjectFit::Cover)
        .with_animation(
            key,
            Animation::new(Duration::from_millis(duration_ms)).with_easing(easing),
            move |el, delta| {
                let scale = scale_from + (scale_to - scale_from) * delta;
                let opacity = opacity_from + (opacity_to - opacity_from) * delta;
                el.w(relative(scale))
                    .h(px(head_height * scale))
                    .top(px(head_height * (1. - scale) / 2. + drift_px))
                    .left(relative((1. - scale) / 2.))
                    .opacity(opacity)
            },
        )
        .into_any_element()
}

fn hairline_rule(flip: bool, width: f32) -> Div {
    let (from, to) = if flip {
        (palette::gold_deep().opacity(0.), palette::gold_deep())
    } else {
        (palette::gold_deep(), palette::gold_deep().opacity(0.))
    };
    div().w(px(width)).h(px(1.)).bg(linear_gradient(
        90.,
        linear_color_stop(from, 0.),
        linear_color_stop(to, 1.),
    ))
}

fn banner(
    text: impl Into<SharedString>,
    action_label: &'static str,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(16.))
        .h(px(32.))
        .bg(palette::royal())
        .border_b_1()
        .border_color(palette::gold_deep())
        .text_sm()
        .text_color(palette::ink())
        .child(text.into())
        .child(Button::new("update-banner-action").label(action_label).on_click(on_click))
}

fn notice_toast(notice: &Notice, engine: &EngineHandle, cx: &mut Context<AppRoot>) -> Div {
    let color = match notice.level {
        NoticeLevel::Info => palette::mist(),
        NoticeLevel::Success => palette::gold(),
        NoticeLevel::Warning => palette::gold(),
        NoticeLevel::Error => palette::warm_error(),
    };
    let id = notice.id;
    let engine = engine.clone();
    div()
        .flex()
        .items_center()
        .gap_3()
        .px(px(12.))
        .py(px(10.))
        .max_w(px(320.))
        .bg(palette::night2())
        .border_1()
        .border_color(palette::gold_inner())
        .child(div().w(px(5.)).h(px(5.)).bg(color).flex_shrink_0())
        .child(div().flex_1().text_sm().text_color(palette::ink()).child(notice.text.clone()))
        .child(
            div()
                .id(("dismiss-notice", id))
                .cursor_pointer()
                .text_xs()
                .text_color(palette::mist2())
                .child("\u{2715}")
                .on_click(cx.listener(move |_, _, _, _| engine.send(Command::DismissNotice(id)))),
        )
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let signed_in = matches!(self.snapshot.account, AccountState::SignedIn(_));

        let shell = div()
            .id("app-root")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(palette::night())
            .text_color(palette::ink());

        if !signed_in {
            let engine = self.engine.clone();
            let snapshot = self.snapshot.clone();
            let chrome = self.chrome_row(None, window, cx);
            return shell
                .child(div().size_full().child(pages::gate::render(&snapshot, &engine)))
                .child(div().absolute().top_0().left_0().right_0().child(chrome));
        }

        let engine = self.engine.clone();
        let snapshot = self.snapshot.clone();
        let page_content: AnyElement = match self.page {
            Page::Overview => pages::overview::render(&snapshot, &engine).into_any_element(),
            Page::History => pages::history::render(
                &snapshot,
                self.history_kind_filter,
                self.history_outcome_filter,
                self.history_expanded,
                cx,
            )
            .into_any_element(),
            Page::Addon => pages::addon::render(&snapshot, &engine).into_any_element(),
            Page::Raids => pages::raids::render(&snapshot).into_any_element(),
            Page::Settings => self.settings.clone().update(cx, |settings, cx| {
                settings.render(&snapshot, window, cx).into_any_element()
            }),
        };

        let scroll_handle = self.scroll_handle.clone();
        let backdrop = self.backdrop_painting(window);
        let nav = (self.nav_brand(), self.nav_links(cx));
        let chrome = self.chrome_row(Some(nav), window, cx);
        let title_block = self.compact_room_title();

        shell
            .child(backdrop)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(chrome)
                    .child(title_block)
                    .when_some(self.update_banner(cx), |d, banner| d.child(banner))
                    .child(
                        div()
                            .id("page-scroll")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&scroll_handle)
                            .child(page_content),
                    ),
            )
            .child(self.notices_overlay(cx))
    }
}
