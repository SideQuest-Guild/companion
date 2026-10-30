//! Background engine. CONTRACT (the core implementation must keep these signatures):
//!
//! - `start(repaint)` loads settings and secrets, spawns the tokio runtime on its own thread, and returns at once.
//!   `repaint` is called after every snapshot change so the UI redraws (the UI passes `ctx.request_repaint()`).
//! - `EngineHandle` is cheap to clone and `Send + Sync`; the tray's event handlers hold their own clone.

use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::state::{Command, Snapshot};
use crate::{config, secrets};

pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// How long `shutdown_blocking` waits for the engine thread to flush and exit before giving up.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct EngineHandle {
    state: Arc<RwLock<Snapshot>>,
    tx: UnboundedSender<Command>,
    shutdown: Arc<(Mutex<bool>, Condvar)>,
}

impl EngineHandle {
    /// A copy of the current state.
    pub fn snapshot(&self) -> Snapshot {
        self.state
            .read()
            .map(|s| s.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    /// Send `Command::Shutdown` and block until state is flushed (at most ~3 s).
    pub fn shutdown_blocking(&self) {
        self.send(Command::Shutdown);
        let (lock, condvar) = &*self.shutdown;
        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = condvar
            .wait_timeout_while(guard, SHUTDOWN_TIMEOUT, |done| !*done)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

pub fn start(repaint: Repaint) -> anyhow::Result<EngineHandle> {
    let settings = config::load_settings();
    let history = crate::history::load();
    let sync_state = config::load_state();
    let token = secrets::get_token().unwrap_or_else(|err| {
        tracing::warn!("could not read the sync token from secret storage: {err:#}");
        None
    });
    let calendar_url = secrets::get_calendar_url().unwrap_or_else(|err| {
        tracing::warn!("could not read the calendar link from secret storage: {err:#}");
        None
    });

    let mut initial =
        Snapshot { settings: settings.clone(), history: history.clone(), ..Snapshot::default() };
    initial.app_version = env!("CARGO_PKG_VERSION").to_owned();
    let state = Arc::new(RwLock::new(initial));

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let shutdown = Arc::new((Mutex::new(false), Condvar::new()));

    let thread_state = state.clone();
    let thread_shutdown = shutdown.clone();
    std::thread::Builder::new()
        .name("sidequest-engine".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(err) => {
                    tracing::error!("could not start the engine's async runtime: {err:#}");
                    let (lock, condvar) = &*thread_shutdown;
                    *lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
                    condvar.notify_all();
                    return;
                }
            };
            runtime.block_on(crate::sync::run(
                thread_state,
                rx,
                repaint,
                settings,
                history,
                sync_state,
                token,
                calendar_url,
            ));
            let (lock, condvar) = &*thread_shutdown;
            *lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
            condvar.notify_all();
        })
        .map_err(|err| anyhow::anyhow!("spawning the engine thread: {err}"))?;

    Ok(EngineHandle { state, tx, shutdown })
}
