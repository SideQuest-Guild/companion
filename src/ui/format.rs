//! Shared "2h ago" / "in 6h" formatting. The freshness line always shows how old data is
//! (`../addon/DESIGN.md`), so every page that shows a timestamp uses these.

use chrono::{DateTime, Utc};

/// `"just now"`, `"5m ago"`, `"2h ago"`, `"3d ago"`.
pub fn ago(at: DateTime<Utc>) -> String {
    let delta = Utc::now().signed_duration_since(at);
    if delta.num_seconds() < 45 {
        "just now".to_owned()
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else {
        format!("{}d ago", delta.num_days())
    }
}

/// `"in 6h"`, `"in 2d"`, `"now"`, `"started"`.
pub fn until(at: DateTime<Utc>) -> String {
    let delta = at.signed_duration_since(Utc::now());
    if delta.num_seconds() < 0 {
        "started".to_owned()
    } else if delta.num_seconds() < 60 {
        "now".to_owned()
    } else if delta.num_minutes() < 60 {
        format!("in {}m", delta.num_minutes())
    } else if delta.num_hours() < 48 {
        format!("in {}h", delta.num_hours())
    } else {
        format!("in {}d", delta.num_days())
    }
}

/// Local clock time, tabular, e.g. `"23:00"`. Paired with [`weekday`] for quest-log rows (time
/// stacked over day).
pub fn clock(at: DateTime<Utc>) -> String {
    at.with_timezone(&chrono::Local).format("%H:%M").to_string()
}

/// Local weekday, e.g. `"Sat"`. Paired with [`clock`].
pub fn weekday(at: DateTime<Utc>) -> String {
    at.with_timezone(&chrono::Local).format("%a").to_string()
}
