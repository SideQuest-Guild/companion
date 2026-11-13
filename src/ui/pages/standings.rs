use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Div, IntoElement, ParentElement as _, Styled as _, div};

use crate::state::{Snapshot, StandingsRow};
use crate::ui::format::{ago, capitalize};
use crate::ui::theme::palette;
use crate::ui::widgets::{Tone, chamber_with, empty_state, freshness_line, state_tag};

pub fn render(snapshot: &Snapshot) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(your_standing_chamber(snapshot))
        .child(top_standings_chamber(snapshot))
}

fn freshness(snapshot: &Snapshot) -> Div {
    div().flex().items_center().justify_end().child(match snapshot.guild.fetched_at {
        Some(at) => freshness_line(format!("Updated {}", ago(at))),
        None => freshness_line("Not fetched yet"),
    })
}

fn your_standing_chamber(snapshot: &Snapshot) -> Div {
    let standings = snapshot.guild.standings.as_ref();
    let body = match standings.and_then(|s| s.you.as_ref()) {
        Some(row) => div()
            .flex()
            .items_center()
            .justify_between()
            .py_2()
            .px_3()
            .bg(palette::night3())
            .child(standing_detail(row))
            .when_some(standings.and_then(|s| s.your_rank), |d, rank| {
                d.child(state_tag(format!("Rank #{rank}"), Tone::Good))
            })
            .into_any_element(),
        None => empty_state("Your standing isn't available yet.").into_any_element(),
    };
    chamber_with(
        "Your Standing",
        div().flex().flex_col().gap_3().child(freshness(snapshot)).child(body),
    )
}

fn top_standings_chamber(snapshot: &Snapshot) -> Div {
    let rows = snapshot.guild.standings.as_ref().map(|s| s.top.as_slice()).unwrap_or(&[]);
    let body = if rows.is_empty() {
        empty_state("No standings yet.").into_any_element()
    } else {
        let mut list = div().flex().flex_col().gap_2();
        for (ix, row) in rows.iter().enumerate() {
            list = list.child(standings_row(ix + 1, row));
        }
        list.into_any_element()
    };
    chamber_with("Top 10", div().flex().flex_col().gap_3().child(body))
}

fn standing_detail(row: &StandingsRow) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_sm().text_color(palette::ink()).child(row.name.clone()))
        .child(div().text_xs().text_color(palette::mist2()).child(format!(
            "EP {:.0} \u{b7} GP {:.0} \u{b7} PR {:.2}{}",
            row.ep,
            row.gp,
            row.pr,
            if row.trial { " \u{b7} Trial" } else { "" }
        )))
}

fn standings_row(rank: usize, row: &StandingsRow) -> Div {
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
                .gap_3()
                .child(div().text_sm().text_color(palette::gold()).child(format!("#{rank}")))
                .child(standing_detail(row)),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::mist2())
                .when_some(row.class.clone(), |d, class| d.child(capitalize(&class))),
        )
}
