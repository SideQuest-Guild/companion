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
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::assets::{Assets as ComponentAssets, icon_assets};
use gpui_kit::{
    AppContext as _, AssetSource, Bounds, Result as GpuiResult, SharedString, Size,
    TitlebarOptions, WindowBounds, WindowKind, WindowOptions, point, px, size,
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

/// Options for [`run`], grouped into a struct to keep the function under clippy's argument cap.
pub struct RunOptions {
    pub engine: EngineHandle,
    pub dirty: Arc<AtomicBool>,
    pub demo_mode: Option<demo::DemoMode>,
    pub start_hidden: bool,
    pub initial_page: Option<app::Page>,
    pub open_quick_panel: bool,
    pub demo_cycle: bool,
    pub window_size_override: Option<(f32, f32)>,
}

/// Runs the GPUI application: installs assets and the brand theme, opens the main window (unless
/// `start_hidden`), and mounts the tray. Blocks until the app quits.
pub fn run(options: RunOptions) {
    let RunOptions {
        engine,
        dirty,
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

        let mut window_size = size(px(960.), px(640.));
        if let Some((w, h)) = window_size_override {
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
            show: !start_hidden,
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

        let (window_handle, root) = gpui_kit::open_window(options, cx, |window, cx| {
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
        .expect("failed to open the SideQuest Companion window");

        tray::install(engine.clone(), window_handle, cx);

        if open_quick_panel {
            quick_panel::toggle(
                engine.clone(),
                window_handle,
                demo_mode.map(demo::DemoMode::snapshot),
                cx,
            );
        }

        let _ = root;
    });
}
