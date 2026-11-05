//! Entry point: logging, single-instance handshake, `--hidden`/`--demo` flags, starts the
//! background engine and hands off to the GPUI window + tray.

// Release builds on Windows are GUI apps: no console window opens next to the tray icon. Logs go to the log file.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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
    let quit_flag = args.iter().any(|a| a == "--quit");

    init_logging();

    if quit_flag {
        // The Windows installer and uninstaller run this to close the app before replacing it.
        std::process::exit(if quit_running_instance() { 0 } else { 1 });
    }

    if !demo && send_to_running_instance(b"show\n").is_some() {
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

    let requests = Arc::new(ui::InstanceRequests::default());
    if !demo {
        start_instance_server(handle.clone(), requests.clone());
        sidequest_companion::self_update::record_installed_version();
    }

    ui::run(ui::RunOptions {
        engine: handle,
        dirty,
        requests,
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

/// Sends `message` to the already running instance, if there is one. Returns the open connection so
/// the caller can wait on it; `None` means no instance answered (this process may become it).
fn send_to_running_instance(message: &[u8]) -> Option<TcpStream> {
    let port_file = instance_port_file()?;
    let port = std::fs::read_to_string(&port_file).ok()?.trim().parse::<u16>().ok()?;
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(500))).ok();
    stream.write_all(message).ok()?;
    Some(stream)
}

/// Asks the running instance to shut down and waits until it has. True if none is left running.
/// Versions before 0.4.0 ignore the request, which is why the installer falls back to ending the
/// process when this reports failure.
fn quit_running_instance() -> bool {
    let Some(mut stream) = send_to_running_instance(b"quit\n") else { return true };
    // The instance never writes back; reading returns once its process exits and the socket closes.
    stream.set_read_timeout(Some(Duration::from_secs(20))).ok();
    let mut buf = [0u8; 1];
    let closed = matches!(stream.read(&mut buf), Ok(0) | Err(_));
    closed && send_to_running_instance(b"ping\n").is_none()
}

/// Binds a localhost port, writes it to `instance.port`, and serves one-line requests from later
/// launches: `show` brings the window to the front (reopening it if it was closed) and `quit` shuts
/// the app down for an installer. Runs on a background thread for the process lifetime.
fn start_instance_server(handle: engine::EngineHandle, requests: Arc<ui::InstanceRequests>) {
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
            // Quit connections stay open until the process exits, so `--quit` can wait on them.
            let mut waiting_to_quit = Vec::new();
            for mut stream in listener.incoming().flatten() {
                let mut line = [0u8; 8];
                let Ok(read) = stream.read(&mut line) else { continue };
                let line = &line[..read];
                if line.starts_with(b"show") {
                    tracing::info!("a second launch asked us to show the window");
                    handle.send(sidequest_companion::state::Command::Rescan);
                    requests.show.store(true, Ordering::Release);
                } else if line.starts_with(b"quit") {
                    tracing::info!("a second launch asked us to quit");
                    requests.quit.store(true, Ordering::Release);
                    waiting_to_quit.push(stream);
                }
            }
        })
        .ok();
}
