//! Desktop notifications via `notify-rust`. Always non-blocking: showing a notification runs on a
//! spawned thread so it never stalls the engine, and a failure is logged, never panics.

const APP_NAME: &str = "SideQuest Companion";

/// Show a desktop notification. Returns immediately; the notification is shown on a background
/// thread, and any failure is logged rather than surfaced.
pub fn notify(title: &str, body: &str) {
    let title = title.to_owned();
    let body = body.to_owned();
    std::thread::spawn(move || {
        if let Err(err) =
            notify_rust::Notification::new().appname(APP_NAME).summary(&title).body(&body).show()
        {
            tracing::warn!("failed to show notification {title:?}: {err}");
        }
    });
}
