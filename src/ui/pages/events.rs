use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Div, IntoElement, ParentElement as _, Stateful, Styled as _, div};

use crate::state::{ReminderScope, Snapshot, UpcomingEvent};
use crate::ui::format::{ago, clock, event_kind_label, until, weekday};
use crate::ui::theme::palette;
use crate::ui::widgets::{Tone, chamber_with, empty_state, quest_log_row, state_tag};

pub fn render(snapshot: &Snapshot) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(events_chamber(snapshot))
        .child(reminder_summary_chamber(snapshot))
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

fn reminder_summary_chamber(snapshot: &Snapshot) -> Div {
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
                    .text_xs()
                    .text_color(palette::gold())
                    .child("Change in Settings \u{2192} Reminders"),
            ),
    )
}
