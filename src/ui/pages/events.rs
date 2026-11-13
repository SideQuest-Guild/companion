use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, InteractiveElement as _, IntoElement, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, div,
};

use chrono::Utc;

use crate::state::{Drive, DriveRecruitRow, ReminderScope, SiteEvent, Snapshot, UpcomingEvent};
use crate::ui::format::{ago, clock, event_kind_label, until, weekday};
use crate::ui::theme::palette;
use crate::ui::widgets::{
    Tone, chamber_with, empty_state, freshness_line, quest_log_row, state_tag, sub_heading,
};

pub fn render(
    snapshot: &Snapshot,
    open_reminder_settings: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App)
    + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(site_events_chamber(snapshot))
        .when_some(snapshot.guild.drive.clone(), |d, drive| d.child(drive_chamber(&drive)))
        .child(events_chamber(snapshot))
        .child(reminder_summary_chamber(snapshot, open_reminder_settings))
}

fn site_events_chamber(snapshot: &Snapshot) -> Div {
    let body = if snapshot.guild.site_events.is_empty() {
        empty_state("No site events yet.").into_any_element()
    } else {
        let mut list = div().flex().flex_col().gap_2();
        for event in &snapshot.guild.site_events {
            list = list.child(site_event_row(event));
        }
        list.into_any_element()
    };
    let freshness = match snapshot.guild.fetched_at {
        Some(at) => freshness_line(format!("Updated {}", ago(at))),
        None => freshness_line("Not fetched yet"),
    };
    chamber_with(
        "Site Events",
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().flex().items_center().justify_end().child(freshness))
            .child(body),
    )
}

fn site_event_row(event: &SiteEvent) -> Div {
    if event.kind == "recruitment" {
        return recruitment_drive_row(event);
    }
    let (label, tone) = match event.my_answer.as_deref() {
        Some("signed") => ("Going", Tone::Good),
        Some("tentative") => ("Maybe", Tone::Warn),
        Some("absent") => ("Can't go", Tone::Bad),
        _ => ("No answer", Tone::Neutral),
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .py_2()
        .px_3()
        .bg(palette::night3())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(palette::ink()).child(event.title.clone()))
                .child(div().text_xs().text_color(palette::mist2()).child(format!(
                    "{} \u{b7} {}",
                    event_kind_label(&event.kind),
                    until(event.starts_at)
                ))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::mist2())
                        .child(format!("{} going, {} maybe", event.going, event.maybe)),
                )
                .child(state_tag(label, tone)),
        )
}

/// A recruitment drive takes no sign-ups, so its row never shows going/maybe counts or an answer tag, just
/// when it starts, or when it ends (or ended).
fn recruitment_drive_row(event: &SiteEvent) -> Div {
    let now = Utc::now();
    let meta = if event.starts_at > now {
        format!("Recruitment drive \u{b7} starts {}", until(event.starts_at))
    } else {
        match event.ends_at {
            Some(ends_at) if ends_at > now => {
                format!("Recruitment drive \u{b7} ends {}", until(ends_at))
            }
            Some(_) => "Recruitment drive \u{b7} ended".to_owned(),
            None => "Recruitment drive".to_owned(),
        }
    };
    div().flex().items_center().justify_between().py_2().px_3().bg(palette::night3()).child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_sm().text_color(palette::ink()).child(event.title.clone()))
            .child(div().text_xs().text_color(palette::mist2()).child(meta)),
    )
}

fn drive_chamber(drive: &Drive) -> Div {
    let max_qualified = drive.leaders.iter().map(|leader| leader.qualified).max().unwrap_or(0);
    let leaders_body = if drive.leaders.is_empty() {
        empty_state("No recruits yet.").into_any_element()
    } else {
        let mut list = div().flex().flex_col().gap_2();
        for leader in &drive.leaders {
            let leading = max_qualified > 0 && leader.qualified == max_qualified;
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py_2()
                    .px_3()
                    .bg(palette::night3())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(palette::ink())
                                    .child(leader.inviter.clone()),
                            )
                            .when(leading, |d| d.child(state_tag("Leading", Tone::Good))),
                    )
                    .child(div().text_xs().text_color(palette::mist2()).child(format!(
                        "{} qualified \u{b7} {} potential \u{b7} {} invited",
                        leader.qualified, leader.potential, leader.invited
                    ))),
            );
        }
        list.into_any_element()
    };
    let your_recruits = if drive.your_recruits.is_empty() {
        empty_state("You haven't invited any recruits yet.").into_any_element()
    } else {
        let mut list = div().flex().flex_col().gap_2();
        for recruit in &drive.your_recruits {
            list = list.child(recruit_row(recruit));
        }
        list.into_any_element()
    };
    chamber_with(
        drive.title.clone(),
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(sub_heading("Leaderboard"))
            .child(leaders_body)
            .child(sub_heading("Your Recruits"))
            .child(your_recruits),
    )
}

fn recruit_row(recruit: &DriveRecruitRow) -> Div {
    let (label, tone) = recruit_status_tag(recruit);
    let mut entry = div().flex().flex_col().gap_1().child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .py_1()
            .px_3()
            .child(div().text_xs().text_color(palette::mist()).child(recruit.recruit.clone()))
            .child(state_tag(label, tone)),
    );
    if recruit.status == "disqualified"
        && let Some(reason) = recruit.reason.as_deref()
    {
        entry = entry.child(
            div()
                .pl_3()
                .text_xs()
                .text_color(palette::mist2())
                .child(disqualify_reason_text(reason)),
        );
    }
    entry
}

/// The tag label and tone for one of the signed-in member's recruits. `counting`/`joined` (still inside the
/// 7-day qualification clock) show as "Potential", with days left when `qualifies_at` is known; the raw
/// status word is never shown.
fn recruit_status_tag(recruit: &DriveRecruitRow) -> (String, Tone) {
    match recruit.status.as_str() {
        "qualified" => ("Qualified".to_owned(), Tone::Good),
        "disqualified" => ("Doesn't count".to_owned(), Tone::Bad),
        "invited" => ("Invited".to_owned(), Tone::Neutral),
        _ => match recruit.qualifies_at {
            Some(qualifies_at) => {
                let days = (qualifies_at - Utc::now()).num_days().max(0);
                (format!("Potential \u{b7} {days}d left"), Tone::Warn)
            }
            None => ("Potential".to_owned(), Tone::Warn),
        },
    }
}

/// Why a `disqualified` recruit doesn't count, matching the site and Discord's wording.
fn disqualify_reason_text(reason: &str) -> &'static str {
    match reason {
        "returning" => "Was in the guild before",
        "own_alt" => "Inviter's own alt",
        "left" => "Left during the week",
        "low_level" => "Below level 10",
        "never_joined" => "Never joined",
        _ => "Doesn't count",
    }
}

fn events_chamber(snapshot: &Snapshot) -> Div {
    let body = if snapshot.events.upcoming.is_empty() {
        empty_state("No upcoming events.").into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(snapshot.events.upcoming.iter().map(event_row))
            .into_any_element()
    };
    let freshness = match snapshot.events.fetched_at {
        Some(at) => format!("Checked {}", ago(at)),
        None => "Not checked yet".to_owned(),
    };
    chamber_with(
        "Upcoming Events",
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(div().text_xs().text_color(palette::mist2()).child(freshness)),
            )
            .when_some(snapshot.events.error.clone(), |d, err| {
                d.child(div().text_sm().text_color(palette::warm_error()).child(err))
            })
            .child(body),
    )
}

fn event_row(event: &UpcomingEvent) -> Stateful<Div> {
    let (status_label, tone) = match event.my_status.as_deref() {
        Some("signed") => ("Going", Tone::Good),
        Some("tentative") => ("Maybe", Tone::Warn),
        Some("absent") => ("Can't go", Tone::Bad),
        _ => ("No answer", Tone::Neutral),
    };
    let done = event.my_status.is_some() || event.cancelled;
    let meta = format!(
        "{} \u{b7} {} \u{b7} {}",
        event_kind_label(&event.kind),
        event.location,
        until(event.starts_at)
    );
    quest_log_row(
        format!("event-{}", event.id),
        done,
        event.title.clone(),
        meta,
        clock(event.starts_at),
        weekday(event.starts_at),
    )
    .when(event.cancelled, |d| d.opacity(0.5))
    .child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .when(event.cancelled, |d| d.child(state_tag("Cancelled", Tone::Bad)))
            .child(state_tag(status_label, tone))
            .child(
                Button::new(format!("open-event-{}", event.id))
                    .ghost()
                    .xsmall()
                    .label("Open")
                    .on_click({
                        let url = event.url.clone();
                        move |_, _, cx| cx.open_url(&url)
                    }),
            ),
    )
}

fn reminder_summary_chamber(
    snapshot: &Snapshot,
    open_reminder_settings: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App)
    + 'static,
) -> Div {
    let r = &snapshot.settings.reminders;
    let scope = match r.scope {
        ReminderScope::SignedUp => "events you're signed up for",
        ReminderScope::AllEvents => "every scheduled event",
    };
    let kinds = if r.kinds.is_empty() {
        "no kinds selected".to_owned()
    } else {
        r.kinds.iter().map(|k| event_kind_label(k)).collect::<Vec<_>>().join(", ")
    };
    let summary = if r.enabled {
        format!("{} minutes before {} ({})", r.lead_minutes, scope, kinds)
    } else {
        "Reminders are turned off.".to_owned()
    };
    chamber_with(
        "Reminders",
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_sm().text_color(palette::mist()).child(summary))
            .child(
                div()
                    .id("open-reminder-settings")
                    .cursor_pointer()
                    .text_xs()
                    .text_color(palette::gold())
                    .hover(|s| s.underline())
                    .child("Change in Settings \u{2192} Reminders")
                    .on_click(open_reminder_settings),
            ),
    )
}
