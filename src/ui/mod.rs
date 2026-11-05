//! The window and tray icon. Reads [`crate::state::Snapshot`], sends [`crate::state::Command`]s.

pub mod app;
pub mod format;
pub mod pages;
pub mod quick_panel;
pub mod theme;
pub mod tray;
pub mod widgets;

pub mod demo;

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::assets::{Assets as ComponentAssets, icon_assets};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AssetSource, Bounds, Result as GpuiResult, SharedString,
    Size, TitlebarOptions, WindowBounds, WindowKind, WindowOptions, point, px, size,
};
use rust_embed::RustEmbed;

use crate::engine::EngineHandle;

icon_assets!(ExtraIcons, [Clock, Package, Download, CalendarClock]);

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icon.png"]
#[include = "rooms/*"]
#[include = "plates/*"]
struct AppFiles;

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> GpuiResult<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(file) = AppFiles::get(path) {
            return Ok(Some(file.data));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        ComponentAssets.load(path)
    }

    fn list(&self, path: &str) -> GpuiResult<Vec<SharedString>> {
        let mut paths = ComponentAssets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(AppFiles::iter().filter_map(|p| p.starts_with(path).then(|| p.into())));
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// What a second launch asked the running app to do (see `main.rs`'s instance server). Set from that
/// server's thread, acted on by the UI thread.
#[derive(Default)]
pub struct InstanceRequests {
    /// Bring the main window to the front, reopening it if it was closed.
    pub show: AtomicBool,
    /// Shut down so an installer can replace the app (`--quit`).
    pub quit: AtomicBool,
}

/// Options for [`run`], grouped into a struct to keep the function under clippy's argument cap.
pub struct RunOptions {
    pub engine: EngineHandle,
    pub dirty: Arc<AtomicBool>,
    pub requests: Arc<InstanceRequests>,
    pub demo_mode: Option<demo::DemoMode>,
    pub start_hidden: bool,
    pub initial_page: Option<app::Page>,
    pub open_quick_panel: bool,
    pub demo_cycle: bool,
    pub window_size_override: Option<(f32, f32)>,
}

type MainWindowOpener = Rc<dyn Fn(bool, &mut App) -> Option<AnyWindowHandle>>;

thread_local! {
    /// The main window while it's open. Closing it destroys it; [`show_main_window`] opens a new one.
    static MAIN_WINDOW: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
    static MAIN_WINDOW_OPENER: RefCell<Option<MainWindowOpener>> = const { RefCell::new(None) };
}

/// Bring the main window to the front, or open a new one if it was closed. The tray, the quick panel
/// and a second launch all come through here.
pub fn show_main_window(cx: &mut App) {
    cx.activate(true);
    let existing = MAIN_WINDOW.with(|cell| *cell.borrow());
    if let Some(handle) = existing
        && handle.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let Some(open) = MAIN_WINDOW_OPENER.with(|cell| cell.borrow().clone()) else { return };
    if let Some(handle) = open(true, cx) {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
    }
}

fn quit(engine: &EngineHandle, cx: &mut App) {
    engine.shutdown_blocking();
    cx.quit();
}

/// Runs the GPUI application: installs assets and the brand theme, opens the main window (unless
/// `start_hidden`), and mounts the tray. Blocks until the app quits.
pub fn run(options: RunOptions) {
    let RunOptions {
        engine,
        dirty,
        requests,
        demo_mode,
        start_hidden,
        initial_page,
        open_quick_panel,
        demo_cycle,
        window_size_override,
    } = options;
    gpui_kit::application().with_assets(AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        theme::install(cx);
        cx.set_app_identity("com.sidequest-guild.companion", "SideQuest Companion");
        cx.activate(true);

        let spec = MainWindowSpec {
            engine: engine.clone(),
            dirty,
            demo_mode,
            initial_page,
            demo_cycle,
            window_size_override,
        };
        let opener: MainWindowOpener = Rc::new(move |show, cx| {
            let handle = open_main_window(&spec, show, cx)?;
            MAIN_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));
            Some(handle)
        });
        MAIN_WINDOW_OPENER.with(|cell| *cell.borrow_mut() = Some(opener.clone()));
        if opener(!start_hidden, cx).is_none() {
            tracing::error!("failed to open the SideQuest Companion window");
            quit(&engine, cx);
            return;
        }

        // Closing the window keeps the app in the tray unless the user turned that off.
        let engine_closed = engine.clone();
        cx.on_window_closed(move |cx, closed| {
            let was_main = MAIN_WINDOW.with(|cell| {
                let mut cell = cell.borrow_mut();
                let was_main = cell.is_some_and(|handle| handle.window_id() == closed);
                if was_main {
                    *cell = None;
                }
                was_main
            });
            if was_main && !engine_closed.snapshot().settings.close_to_tray {
                quit(&engine_closed, cx);
            }
        })
        .detach();

        let engine_requests = engine.clone();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                if requests.quit.swap(false, Ordering::AcqRel) {
                    tracing::info!("another launch asked us to quit");
                    cx.update(|cx| quit(&engine_requests, cx));
                    return;
                }
                if requests.show.swap(false, Ordering::AcqRel) {
                    cx.update(show_main_window);
                }
            }
        })
        .detach();

        tray::install(engine.clone(), cx);

        if open_quick_panel {
            quick_panel::toggle(engine.clone(), demo_mode.map(demo::DemoMode::snapshot), cx);
        }
    });
}

/// Everything a main window is built from, kept so a closed window can be opened again.
struct MainWindowSpec {
    engine: EngineHandle,
    dirty: Arc<AtomicBool>,
    demo_mode: Option<demo::DemoMode>,
    initial_page: Option<app::Page>,
    demo_cycle: bool,
    window_size_override: Option<(f32, f32)>,
}

/// Opens one main window. `show` is false only for a `start_hidden` launch.
fn open_main_window(spec: &MainWindowSpec, show: bool, cx: &mut App) -> Option<AnyWindowHandle> {
    let MainWindowSpec { engine, dirty, demo_mode, initial_page, demo_cycle, window_size_override } =
        spec;
    let (demo_mode, initial_page, demo_cycle) = (*demo_mode, *initial_page, *demo_cycle);
    let mut window_size = size(px(960.), px(640.));
    if let Some((w, h)) = *window_size_override {
        window_size = size(px(w), px(h));
    } else if let Some(display) = cx.primary_display() {
        let display_size = display.bounds().size;
        window_size.width = window_size.width.min(display_size.width * 0.9);
        window_size.height = window_size.height.min(display_size.height * 0.9);
    }
    let window_bounds = Bounds::centered(None, window_size, cx);

    // The native traffic lights sit inside the merged 44px title-bar/nav row (see
    // `app::AppRoot::chrome_row`), vertically centred the same way GPUI Kit centres its own
    // dot cluster (~16px tall) in its 34px bar: y = (row height - 16) / 2.
    let traffic_light_position = point(px(9.), px((app::NAV_ROW_HEIGHT - 16.) / 2.));

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(window_bounds)),
        window_min_size: Some(Size { width: px(720.), height: px(520.) }),
        kind: WindowKind::Normal,
        show,
        #[cfg(not(target_os = "linux"))]
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Client),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(traffic_light_position),
        }),
        // The chrome row draws itself and moves the window via `start_window_move`, so
        // AppKit must not treat it as a system window-move region (see `app::ChromeRowState`).
        app_owns_titlebar_drag: true,
        ..Default::default()
    };

    gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("SideQuest Companion");
        cx.new(|cx| {
            app::AppRoot::new(
                engine.clone(),
                dirty.clone(),
                demo_mode,
                initial_page,
                demo_cycle,
                window,
                cx,
            )
        })
    })
    .map_err(|err| tracing::error!("opening the main window: {err:#}"))
    .ok()
    .map(|(handle, _)| handle)
}
