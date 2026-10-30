//! Upcoming guild events: the public events list, the member's private `.ics` calendar feed, and
//! the scheduler that decides when a reminder is due.

use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use crate::state::{ReminderScope, ReminderSettings, UpcomingEvent};

/// Fetch upcoming events from the site's public events list, one request per kind in `kinds` (the
/// API only filters by a single kind at a time); an empty `kinds` fetches every upcoming event.
/// Results are de-duplicated by event id.
pub async fn fetch_public_events(
    client: &reqwest::Client,
    site_url: &str,
    kinds: &[String],
) -> Result<Vec<UpcomingEvent>> {
    let site_url = site_url.trim_end_matches('/');
    if kinds.is_empty() {
        return fetch_events_page(client, site_url, None).await;
    }
    let mut events = Vec::new();
    let mut seen = HashSet::new();
    for kind in kinds {
        for event in fetch_events_page(client, site_url, Some(kind)).await? {
            if seen.insert(event.id.clone()) {
                events.push(event);
            }
        }
    }
    Ok(events)
}

async fn fetch_events_page(
    client: &reqwest::Client,
    site_url: &str,
    kind: Option<&str>,
) -> Result<Vec<UpcomingEvent>> {
    let mut url = format!("{site_url}/api/events/?when=upcoming");
    if let Some(kind) = kind {
        url.push_str("&kind=");
        url.push_str(kind);
    }
    let resp = client.get(&url).send().await.with_context(|| format!("requesting {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("the site returned {status} for {url}");
    }
    let body: Vec<EventSummaryDto> =
        resp.json().await.with_context(|| format!("parsing events from {url}"))?;
    Ok(body.into_iter().map(|e| e.into_event(site_url)).collect())
}

#[derive(Debug, Deserialize)]
struct EventSummaryDto {
    id: String,
    kind: String,
    title: String,
    location: String,
    starts_at: DateTime<Utc>,
    ends_at: Option<DateTime<Utc>>,
    status: String,
}

impl EventSummaryDto {
    fn into_event(self, site_url: &str) -> UpcomingEvent {
        UpcomingEvent {
            id: self.id.clone(),
            title: self.title,
            kind: self.kind,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            location: self.location,
            my_status: None,
            cancelled: self.status.eq_ignore_ascii_case("cancelled"),
            url: format!("{site_url}/events/detail?id={}", self.id),
        }
    }
}

/// Fetch and parse the member's private `.ics` calendar feed.
pub async fn fetch_calendar(client: &reqwest::Client, url: &str) -> Result<Vec<UpcomingEvent>> {
    let resp = client.get(url).send().await.with_context(|| format!("requesting {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("the site returned {status} for the calendar feed");
    }
    let text =
        resp.text().await.with_context(|| format!("reading the calendar feed from {url}"))?;
    Ok(parse_ics(&text, Utc::now()))
}

/// Parse an RFC 5545 `.ics` feed into upcoming events: future events plus anything that started in
/// the last 3 hours. Tolerates feeds without `X-SIDEQUEST-KIND`/`X-SIDEQUEST-SIGNUP` by falling
/// back to the kind parenthesised in `SUMMARY` and leaving `my_status` unset.
pub fn parse_ics(text: &str, now: DateTime<Utc>) -> Vec<UpcomingEvent> {
    let lines = unfold(text);
    let mut events = Vec::new();
    let mut current: Option<RawEvent> = None;
    for line in &lines {
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            current = Some(RawEvent::default());
            continue;
        }
        if line.eq_ignore_ascii_case("END:VEVENT") {
            if let Some(raw) = current.take()
                && let Some(event) = build_event(raw)
            {
                events.push(event);
            }
            continue;
        }
        let Some(cur) = current.as_mut() else { continue };
        let Some((name, raw_value)) = split_property(line) else { continue };
        let value = unescape(raw_value);
        match name.to_ascii_uppercase().as_str() {
            "UID" => cur.uid = Some(value),
            "SUMMARY" => cur.summary = Some(value),
            "LOCATION" => cur.location = Some(value),
            "DTSTART" => cur.dtstart = Some(value),
            "DTEND" => cur.dtend = Some(value),
            "STATUS" => cur.status = Some(value),
            "URL" => cur.url = Some(value),
            "X-SIDEQUEST-KIND" => cur.kind = Some(value),
            "X-SIDEQUEST-SIGNUP" => cur.signup = Some(value),
            _ => {}
        }
    }
    let cutoff = now - chrono::Duration::hours(3);
    events.retain(|e: &UpcomingEvent| e.starts_at >= cutoff);
    events
}

#[derive(Debug, Default)]
struct RawEvent {
    uid: Option<String>,
    summary: Option<String>,
    location: Option<String>,
    dtstart: Option<String>,
    dtend: Option<String>,
    status: Option<String>,
    url: Option<String>,
    kind: Option<String>,
    signup: Option<String>,
}

fn build_event(ev: RawEvent) -> Option<UpcomingEvent> {
    let starts_at = parse_ics_datetime(ev.dtstart.as_deref()?)?;
    let ends_at = ev.dtend.as_deref().and_then(parse_ics_datetime);
    let raw_summary = ev.summary.unwrap_or_default();
    let (title, fallback_kind) = split_kind_from_summary(&raw_summary);
    let kind = ev.kind.unwrap_or_else(|| fallback_kind.unwrap_or_else(|| "other".to_owned()));
    let cancelled = ev.status.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED"));
    let id = ev
        .uid
        .as_deref()
        .map(|uid| uid.split('@').next().unwrap_or(uid).to_owned())
        .unwrap_or_default();
    Some(UpcomingEvent {
        id,
        title,
        kind,
        starts_at,
        ends_at,
        location: ev.location.unwrap_or_default(),
        my_status: ev.signup,
        cancelled,
        url: ev.url.unwrap_or_default(),
    })
}

/// Split `"<title> (<Kind label>)"` into `(title, Some(kind))`; the website's labels (`Raid`,
/// `Dungeon`, `Leveling`, `PvP`, `Social`, `Transmog`, `Other`) lowercase directly to the `EventKind`
/// wire values.
fn split_kind_from_summary(summary: &str) -> (String, Option<String>) {
    if let Some(open) = summary.rfind('(')
        && summary.ends_with(')')
        && open > 0
    {
        let label = &summary[open + 1..summary.len() - 1];
        if !label.is_empty() {
            let title = summary[..open].trim_end().to_owned();
            return (title, Some(label.to_ascii_lowercase()));
        }
    }
    (summary.to_owned(), None)
}

fn parse_ics_datetime(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    let value = value.strip_suffix('Z').unwrap_or(value);
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Some(naive.and_utc())
}

/// Un-fold RFC 5545 line folding: a line beginning with a space or tab continues the previous one.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            if let Some(last) = lines.last_mut() {
                last.push_str(&raw[1..]);
            }
        } else {
            lines.push(raw.to_owned());
        }
    }
    lines
}

fn split_property(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let (name_part, rest) = line.split_at(colon);
    let value = &rest[1..];
    let name = name_part.split(';').next().unwrap_or(name_part);
    Some((name, value))
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Tracks which (event, start time) pairs have already fired a reminder, so each fires once.
#[derive(Debug, Default)]
pub struct ReminderScheduler {
    fired: HashSet<(String, DateTime<Utc>)>,
}

impl ReminderScheduler {
    /// Returns the events that just entered their reminder window (`lead_minutes` before start,
    /// up to the start itself) and haven't fired yet. Skips cancelled events; with
    /// [`ReminderScope::SignedUp`], skips anything the member isn't `signed`/`tentative` for.
    pub fn due(
        &mut self,
        events: &[UpcomingEvent],
        settings: &ReminderSettings,
        now: DateTime<Utc>,
    ) -> Vec<UpcomingEvent> {
        if !settings.enabled {
            return Vec::new();
        }
        let lead = chrono::Duration::minutes(i64::from(settings.lead_minutes));
        let mut due = Vec::new();
        for event in events {
            if event.cancelled {
                continue;
            }
            if !settings.kinds.is_empty()
                && !settings.kinds.iter().any(|k| k.eq_ignore_ascii_case(&event.kind))
            {
                continue;
            }
            if settings.scope == ReminderScope::SignedUp
                && !matches!(event.my_status.as_deref(), Some("signed") | Some("tentative"))
            {
                continue;
            }
            let key = (event.id.clone(), event.starts_at);
            if self.fired.contains(&key) {
                continue;
            }
            let fire_at = event.starts_at - lead;
            if now >= fire_at && now < event.starts_at {
                self.fired.insert(key);
                due.push(event.clone());
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS_SAMPLE: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
UID:11111111-1111-1111-1111-111111111111@sidequest-guild.com\r\n\
DTSTART:20250115T190000Z\r\n\
DTEND:20250115T220000Z\r\n\
SUMMARY:Molten Core\\, Night One (Raid)\r\n\
LOCATION:Molten Core\r\n\
URL:https://sidequest-guild.com/events/detail?id=1\r\n\
X-SIDEQUEST-KIND:raid\r\n\
X-SIDEQUEST-SIGNUP:signed\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:22222222-2222-2222-2222-222222222222@sidequest-guild.com\r\n\
DTSTART:20250116T190000Z\r\n\
SUMMARY:Old-Style Event (Dungeon)\r\n\
STATUS:CANCELLED\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:33333333-3333-3333-3333-333333333333@sidequest-guild.com\r\n\
DTSTART:20200101T190000Z\r\n\
SUMMARY:Ancient History (Social)\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parse_ics_unfolds_and_unescapes_and_reads_kind_signup() {
        let now = DateTime::parse_from_rfc3339("2025-01-10T00:00:00Z").unwrap().with_timezone(&Utc);
        let events = parse_ics(ICS_SAMPLE, now);
        // The cancelled event is kept (callers decide what to do with it) but the 2020 event is
        // dropped as too far in the past.
        assert_eq!(events.len(), 2);

        let raid = &events[0];
        assert_eq!(raid.id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(raid.title, "Molten Core, Night One");
        assert_eq!(raid.kind, "raid");
        assert_eq!(raid.location, "Molten Core");
        assert_eq!(raid.my_status.as_deref(), Some("signed"));
        assert!(!raid.cancelled);

        let dungeon = &events[1];
        assert_eq!(dungeon.title, "Old-Style Event");
        // No X-SIDEQUEST-KIND: falls back to the parenthesised label in SUMMARY.
        assert_eq!(dungeon.kind, "dungeon");
        assert!(dungeon.cancelled);
        assert_eq!(dungeon.my_status, None);
    }

    #[test]
    fn parse_ics_drops_events_older_than_three_hours() {
        let now = DateTime::parse_from_rfc3339("2025-01-15T20:00:00Z").unwrap().with_timezone(&Utc);
        // Starts 3h30m before `now`: dropped.
        let old = "BEGIN:VEVENT\r\nUID:a@sidequest-guild.com\r\nDTSTART:20250115T163000Z\r\nSUMMARY:Too Old (Raid)\r\nEND:VEVENT\r\n";
        // Starts 2h before `now`: kept (within the 3h grace window).
        let recent = "BEGIN:VEVENT\r\nUID:b@sidequest-guild.com\r\nDTSTART:20250115T180000Z\r\nSUMMARY:Just Started (Raid)\r\nEND:VEVENT\r\n";
        let text = format!("BEGIN:VCALENDAR\r\n{old}{recent}END:VCALENDAR\r\n");
        let events = parse_ics(&text, now);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Just Started");
    }

    fn event(
        id: &str,
        kind: &str,
        starts_at: DateTime<Utc>,
        my_status: Option<&str>,
        cancelled: bool,
    ) -> UpcomingEvent {
        UpcomingEvent {
            id: id.to_owned(),
            title: "Test Event".to_owned(),
            kind: kind.to_owned(),
            starts_at,
            ends_at: None,
            location: String::new(),
            my_status: my_status.map(str::to_owned),
            cancelled,
            url: String::new(),
        }
    }

    #[test]
    fn scheduler_fires_once_within_lead_window() {
        let settings = ReminderSettings {
            enabled: true,
            scope: ReminderScope::AllEvents,
            lead_minutes: 30,
            kinds: vec!["raid".into()],
        };
        let start =
            DateTime::parse_from_rfc3339("2025-01-15T19:00:00Z").unwrap().with_timezone(&Utc);
        let ev = event("1", "raid", start, None, false);
        let mut scheduler = ReminderScheduler::default();

        // Too early: 40 minutes before start, outside the 30-minute lead window.
        let too_early = start - chrono::Duration::minutes(40);
        assert!(scheduler.due(std::slice::from_ref(&ev), &settings, too_early).is_empty());

        // Inside the window: fires.
        let in_window = start - chrono::Duration::minutes(10);
        let due = scheduler.due(std::slice::from_ref(&ev), &settings, in_window);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "1");

        // Checking again later in the same window must not fire a second time.
        let still_in_window = start - chrono::Duration::minutes(5);
        assert!(scheduler.due(&[ev], &settings, still_in_window).is_empty());
    }

    #[test]
    fn scheduler_skips_cancelled_events() {
        let settings = ReminderSettings {
            enabled: true,
            scope: ReminderScope::AllEvents,
            lead_minutes: 30,
            kinds: vec![],
        };
        let start = Utc::now() + chrono::Duration::minutes(10);
        let ev = event("1", "raid", start, None, true);
        let mut scheduler = ReminderScheduler::default();
        assert!(scheduler.due(&[ev], &settings, Utc::now()).is_empty());
    }

    #[test]
    fn scheduler_respects_signed_up_scope() {
        let settings = ReminderSettings {
            enabled: true,
            scope: ReminderScope::SignedUp,
            lead_minutes: 30,
            kinds: vec![],
        };
        let start = Utc::now() + chrono::Duration::minutes(10);
        let mut scheduler = ReminderScheduler::default();

        let not_signed = event("1", "raid", start, None, false);
        assert!(scheduler.due(&[not_signed], &settings, Utc::now()).is_empty());

        let absent = event("2", "raid", start, Some("absent"), false);
        assert!(scheduler.due(&[absent], &settings, Utc::now()).is_empty());

        let tentative = event("3", "raid", start, Some("tentative"), false);
        assert_eq!(scheduler.due(&[tentative], &settings, Utc::now()).len(), 1);
    }

    #[test]
    fn scheduler_disabled_never_fires() {
        let settings = ReminderSettings {
            enabled: false,
            scope: ReminderScope::AllEvents,
            lead_minutes: 30,
            kinds: vec![],
        };
        let start = Utc::now() + chrono::Duration::minutes(10);
        let ev = event("1", "raid", start, None, false);
        let mut scheduler = ReminderScheduler::default();
        assert!(scheduler.due(&[ev], &settings, Utc::now()).is_empty());
    }
}
