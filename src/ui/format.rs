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

/// Every event kind the website schedules (`EventKind` in the website's `schemas/events.py`), in its order.
pub const EVENT_KINDS: &[&str] =
    &["raid", "dungeon", "world_boss", "leveling", "pvp", "social", "transmog", "other"];

/// A website event kind as the site labels it (`EVENT_KIND_LABELS`): `"world_boss"` to `"World Boss"`, `"pvp"` to
/// `"PvP"`. A kind this build doesn't know yet (the site deploys on its own) is shown with its underscores as spaces.
pub fn event_kind_label(kind: &str) -> String {
    match kind {
        "pvp" => "PvP".to_owned(),
        "world_boss" => "World Boss".to_owned(),
        other => {
            let spaced = other.replace('_', " ");
            let mut chars = spaced.chars();
            chars
                .next()
                .map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
        }
    }
}

/// Capitalizes the first letter, e.g. a WoW class string (`"warrior"` to `"Warrior"`).
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_labels_read_as_words() {
        assert_eq!(event_kind_label("raid"), "Raid");
        assert_eq!(event_kind_label("world_boss"), "World Boss");
        assert_eq!(event_kind_label("pvp"), "PvP");
        assert_eq!(event_kind_label("brand_new_kind"), "Brand new kind");
        assert_eq!(event_kind_label(""), "");
    }

    #[test]
    fn capitalize_uppercases_only_the_first_letter() {
        assert_eq!(capitalize("warrior"), "Warrior");
        assert_eq!(capitalize(""), "");
    }
}
