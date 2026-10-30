//! System tray: status line, Sync now, Pause auto-sync, Open window, Quick panel, Open website,
//! Quit. Icon variants (idle/syncing/paused/attention) are a status dot composited onto the base
//! icon at runtime and refreshed on a poll loop; macOS composites the dot into the monochrome
//! template image (opacity/size only, since the system tints the whole icon one colour), other
//! platforms get a coloured dot matching the brand palette.

use std::cell::Cell;
use std::time::Duration;

use gpui_kit::{AnyWindowHandle, App, actions};
use gpui_tray::{Icon as TrayIcon, Tray};

use crate::engine::EngineHandle;
use crate::state::{Command, SyncPhase};
use crate::ui::quick_panel;
#[cfg(not(target_os = "macos"))]
use crate::ui::theme::palette_bytes;

actions!(
    tray,
    [TraySyncNow, TrayTogglePause, TrayOpenWindow, TrayQuickPanel, TrayOpenWebsite, TrayQuit]
);

#[cfg(not(target_os = "macos"))]
const TRAY_RGBA: &[u8] = include_bytes!("../../assets/tray.png");
#[cfg(target_os = "macos")]
const TRAY_TEMPLATE_RGBA: &[u8] = include_bytes!("../../assets/tray-template.png");

thread_local! {
    static TRAY: std::cell::RefCell<Option<Tray>> = const { std::cell::RefCell::new(None) };
    static LAST_ICON_STATE: Cell<Option<IconState>> = const { Cell::new(None) };
}

/// Installs the tray icon and its action handlers. The tray is kept alive in a process-lifetime
/// static (dropping it would remove the native icon), since `run` never returns while the app is open.
pub fn install(engine: EngineHandle, window: AnyWindowHandle, cx: &mut App) {
    let initial_state = icon_state(&engine.snapshot().sync.phase);
    let Some(icon) = build_icon(initial_state) else {
        tracing::error!("tray: skipping the tray icon \u{2014} it failed to decode");
        return;
    };

    let engine_sync = engine.clone();
    cx.on_action(move |_: &TraySyncNow, _cx| {
        tracing::info!("tray: Sync now clicked");
        engine_sync.send(Command::SyncNow);
    });

    let engine_pause = engine.clone();
    cx.on_action(move |_: &TrayTogglePause, _cx| {
        let paused = matches!(engine_pause.snapshot().sync.phase, SyncPhase::Paused);
        tracing::info!(resuming = paused, "tray: Pause/Resume auto-sync clicked");
        engine_pause.send(Command::SetPaused(!paused));
    });

    cx.on_action(move |_: &TrayOpenWindow, cx| {
        tracing::info!("tray: Open SideQuest Companion clicked");
        let _ = window.update(cx, |_, window, _| {
            window.activate_window();
        });
    });

    let engine_panel = engine.clone();
    cx.on_action(move |_: &TrayQuickPanel, cx| {
        tracing::info!("tray: Quick panel clicked");
        quick_panel::toggle(engine_panel.clone(), window, None, cx);
    });

    cx.on_action(move |_: &TrayOpenWebsite, cx| {
        tracing::info!("tray: Open website clicked");
        cx.open_url("https://sidequest-guild.com");
    });

    let engine_quit = engine.clone();
    cx.on_action(move |_: &TrayQuit, cx| {
        tracing::info!("tray: Quit clicked");
        engine_quit.shutdown_blocking();
        cx.quit();
    });

    let engine_menu = engine.clone();
    let tray = Tray::builder()
        .icon(icon)
        .tooltip("SideQuest Companion")
        .menu(move |_cx| {
            let snapshot = engine_menu.snapshot();
            let signed_in = matches!(snapshot.account, crate::state::AccountState::SignedIn(_));
            if !signed_in {
                return vec![
                    gpui_kit::MenuItem::action("Open SideQuest Companion", TrayOpenWindow),
                    gpui_kit::MenuItem::separator(),
                    gpui_kit::MenuItem::action("Quit", TrayQuit),
                ];
            }
            let paused = matches!(snapshot.sync.phase, SyncPhase::Paused);
            vec![
                gpui_kit::MenuItem::action(status_label(&engine_menu), TraySyncNow).disabled(true),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Sync now", TraySyncNow),
                gpui_kit::MenuItem::action("Pause auto-sync", TrayTogglePause).checked(paused),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Open SideQuest Companion", TrayOpenWindow),
                gpui_kit::MenuItem::action("Quick panel", TrayQuickPanel),
                gpui_kit::MenuItem::action("Open website", TrayOpenWebsite),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Quit", TrayQuit),
            ]
        })
        .on_activate(TrayOpenWindow)
        .build(cx);
    let tray = match tray {
        Ok(tray) => tray,
        Err(err) => {
            tracing::error!(%err, "tray: failed to create the tray icon \u{2014} skipping the tray");
            return;
        }
    };

    TRAY.with(|cell| *cell.borrow_mut() = Some(tray));
    LAST_ICON_STATE.with(|cell| cell.set(Some(initial_state)));
    tracing::info!(?initial_state, "tray: installed");

    spawn_icon_poll(engine, cx);
}

/// Polls the engine's sync phase and refreshes the tray icon (and lets the native menu pick up
/// the new status label next time it opens) whenever the visible state changes.
fn spawn_icon_poll(engine: EngineHandle, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let state = icon_state(&engine.snapshot().sync.phase);
            let changed = LAST_ICON_STATE.with(|cell| cell.replace(Some(state)) != Some(state));
            if !changed {
                continue;
            }
            cx.update(|cx| {
                TRAY.with(|cell| {
                    if let Some(tray) = cell.borrow().as_ref() {
                        tracing::info!(?state, "tray: icon state changed");
                        if let Some(icon) = build_icon(state) {
                            let _ = tray.set_icon(icon, cx);
                        }
                        let _ = tray.refresh_menu(cx);
                    }
                });
            });
        }
    })
    .detach();
}

/// The visible tray states, collapsing the two string-carrying [`SyncPhase`] variants into one
/// "needs attention" state so icon redraws don't churn on message text changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconState {
    Idle,
    Syncing,
    Paused,
    Attention,
}

fn icon_state(phase: &SyncPhase) -> IconState {
    match phase {
        SyncPhase::Starting | SyncPhase::Watching => IconState::Idle,
        SyncPhase::Uploading => IconState::Syncing,
        SyncPhase::Paused => IconState::Paused,
        SyncPhase::NeedsSetup(_) | SyncPhase::Error(_) => IconState::Attention,
    }
}

fn status_label(engine: &EngineHandle) -> String {
    match engine.snapshot().sync.phase {
        SyncPhase::Starting => "Starting\u{2026}".to_owned(),
        SyncPhase::Watching => "Watching for changes".to_owned(),
        SyncPhase::Uploading => "Uploading\u{2026}".to_owned(),
        SyncPhase::Paused => "Paused".to_owned(),
        SyncPhase::NeedsSetup(_) => "Needs setup".to_owned(),
        SyncPhase::Error(_) => "Sync error".to_owned(),
    }
}

/// Decodes the tray icon for `state`, or `None` (logging the reason) if the embedded PNG fails
/// to decode or the native tray backend rejects the pixel buffer.
fn build_icon(state: IconState) -> Option<gpui_tray::Icon> {
    #[cfg(target_os = "macos")]
    {
        let (rgba, w, h) = decode_png(TRAY_TEMPLATE_RGBA)?;
        let rgba = composite_dot(rgba, w, h, template_dot(state));
        // 44 px at 2x = a 22 pt menu-bar icon (macOS's standard height), crisp on Retina.
        match TrayIcon::from_rgba(rgba, w, h).scale_factor(2.0).template(true).build() {
            Ok(icon) => Some(icon),
            Err(err) => {
                tracing::error!(%err, "tray: failed to build the macOS template icon");
                None
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let (rgba, w, h) = decode_png(TRAY_RGBA)?;
        let rgba = composite_dot(rgba, w, h, color_dot(state));
        match TrayIcon::from_rgba(rgba, w, h).build() {
            Ok(icon) => Some(icon),
            Err(err) => {
                tracing::error!(%err, "tray: failed to build the tray icon");
                None
            }
        }
    }
}

/// `(color, alpha, radius)` for the status dot, or `None` to leave the icon untouched.
#[cfg(target_os = "macos")]
fn template_dot(state: IconState) -> Option<([u8; 3], u8, f32)> {
    match state {
        IconState::Idle => None,
        IconState::Syncing => Some(([255, 255, 255], 255, 3.5)),
        IconState::Paused => Some(([255, 255, 255], 130, 3.5)),
        IconState::Attention => Some(([255, 255, 255], 255, 5.0)),
    }
}

#[cfg(not(target_os = "macos"))]
fn color_dot(state: IconState) -> Option<([u8; 3], u8, f32)> {
    match state {
        IconState::Idle => None,
        IconState::Syncing => Some((palette_bytes::GOLD, 255, 11.0)),
        IconState::Paused => Some((palette_bytes::MIST2, 255, 9.0)),
        IconState::Attention => Some((palette_bytes::WARM_ERROR, 255, 13.0)),
    }
}

/// Alpha-blends a filled circle into the bottom-right corner of a tightly packed RGBA8 buffer.
fn composite_dot(mut rgba: Vec<u8>, w: u32, h: u32, dot: Option<([u8; 3], u8, f32)>) -> Vec<u8> {
    let Some((color, alpha, radius)) = dot else { return rgba };
    let cx = w as f32 - radius - 1.0;
    let cy = h as f32 - radius - 1.0;
    let sa = alpha as f32 / 255.0;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            // 1px anti-aliased edge so the dot doesn't look jagged at tray-icon sizes.
            let coverage = (radius + 0.5 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let a = sa * coverage;
            let idx = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                rgba[idx + c] =
                    (color[c] as f32 * a + rgba[idx + c] as f32 * (1.0 - a)).round() as u8;
            }
            rgba[idx + 3] = (rgba[idx + 3] as f32 * (1.0 - a) + 255.0 * a).round() as u8;
        }
    }
    rgba
}

/// Decodes a PNG into (RGBA8 pixels, width, height), or `None` (logging the reason) if the bytes
/// aren't a valid PNG.
fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = match decoder.read_info() {
        Ok(reader) => reader,
        Err(err) => {
            tracing::error!(%err, "tray: embedded tray icon PNG is invalid");
            return None;
        }
    };
    let (w, h) = (reader.info().width, reader.info().height);
    let Some(size) = reader.output_buffer_size() else {
        tracing::error!(w, h, "tray: embedded tray icon PNG is too large to decode");
        return None;
    };
    let mut buf = vec![0; size];
    let info = match reader.next_frame(&mut buf) {
        Ok(info) => info,
        Err(err) => {
            tracing::error!(%err, "tray: failed to decode the embedded tray icon PNG frame");
            return None;
        }
    };
    buf.truncate(info.buffer_size());
    Some((buf, w, h))
}
