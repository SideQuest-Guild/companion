//! Entry point: logging, single-instance handshake, `--hidden`/`--demo` flags, starts the
//! background engine and hands off to the GPUI window + tray.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sidequest_companion::engine;
use sidequest_companion::paths;
use sidequest_companion::ui;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let demo_mode = sidequest_companion::ui::demo::DemoMode::from_flags(&args);
    let demo = demo_mode.is_some();
    let hidden_flag = args.iter().any(|a| a == "--hidden");
    let open_quick_panel = args.iter().any(|a| a == "--quick-panel");
    let demo_cycle = args.iter().any(|a| a == "--demo-cycle");
    let size_override = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| args.get(i + 1))
        .and_then(|value| value.split_once(['x', 'X']))
        .and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)));
    let initial_page = args
        .iter()
        .position(|a| a == "--page")
        .and_then(|i| args.get(i + 1))
        .and_then(|value| sidequest_companion::ui::app::Page::from_flag(value));

    init_logging();

    if !demo && already_running() {
        tracing::info!(
            "another instance is already running; asked it to show its window and exiting"
        );
        return;
    }

    let dirty = Arc::new(AtomicBool::new(true));
    let dirty_for_repaint = dirty.clone();
    let repaint: engine::Repaint =
        Arc::new(move || dirty_for_repaint.store(true, Ordering::Release));

    let handle = engine::start(repaint).expect("failed to start the background engine");
    let start_hidden = hidden_flag || (!demo && handle.snapshot().settings.start_hidden);

    if !demo {
        start_instance_server(handle.clone(), dirty.clone());
    }

    ui::run(ui::RunOptions {
        engine: handle,
        dirty,
        demo_mode,
        start_hidden,
        initial_page,
        open_quick_panel,
        demo_cycle,
        window_size_override: size_override,
    });
}

static LOG_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
    std::sync::OnceLock::new();

fn init_logging() {
    use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

    let file_layer = paths::log_dir().ok().map(|dir| {
        let file_appender = tracing_appender::rolling::daily(dir, "sidequest-companion.log");
        let (writer, guard) = tracing_appender::non_blocking(file_appender);
        // The guard must outlive the process for buffered logs to flush; this runs once at startup
        // and owns it in a process-lifetime static rather than leaking an allocation.
        let _ = LOG_GUARD.set(guard);
        tracing_subscriber::fmt::layer().with_writer(writer).with_ansi(false)
    });

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .init();

    prune_old_logs();
}

fn prune_old_logs() {
    let Ok(dir) = paths::log_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut logs: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("sidequest-companion.log"))
        .collect();
    logs.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    while logs.len() > 7 {
        let entry = logs.remove(0);
        let _ = std::fs::remove_file(entry.path());
    }
}

fn instance_port_file() -> Option<std::path::PathBuf> {
    paths::data_dir().ok().map(|d| d.join("instance.port"))
}

/// If another instance is already listening on the recorded port, ask it to show its window and
/// return `true`. Otherwise returns `false` (this process should become the running instance).
fn already_running() -> bool {
    let Some(port_file) = instance_port_file() else { return false };
    let Ok(contents) = std::fs::read_to_string(&port_file) else { return false };
    let Ok(port) = contents.trim().parse::<u16>() else { return false };
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else { return false };
    stream.set_write_timeout(Some(Duration::from_millis(500))).ok();
    stream.write_all(b"show\n").is_ok()
}

/// Binds a localhost port, writes it to `instance.port`, and accepts the literal line `show` to
/// bring this instance's window to the front. Runs on a background thread for the process lifetime.
fn start_instance_server(handle: engine::EngineHandle, dirty: Arc<AtomicBool>) {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
        tracing::warn!(
            "could not bind the single-instance port; a second launch will open a second window"
        );
        return;
    };
    let Ok(addr) = listener.local_addr() else { return };
    if let Some(port_file) = instance_port_file() {
        let _ = std::fs::write(&port_file, addr.port().to_string());
    }

    std::thread::Builder::new()
        .name("instance-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut line = [0u8; 8];
                if stream.read(&mut line).is_ok() && line.starts_with(b"show") {
                    tracing::info!("a second launch asked us to show the window");
                    handle.send(sidequest_companion::state::Command::Rescan);
                    dirty.store(true, Ordering::Release);
                    // The UI's own poll loop notices `dirty`; showing the window itself happens via
                    // the tray's `TrayOpenWindow` action dispatch, which `ui::tray` also drives from
                    // a `window.activate_window()` call reachable from any thread through GPUI's
                    // foreground executor. Here we only need the process to know it should wake up.
                }
            }
        })
        .ok();
}
