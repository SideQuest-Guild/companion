use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Div, IntoElement, ParentElement as _, Styled as _, div};

use crate::state::{RosterCharacter, RosterMember, Snapshot};
use crate::ui::format::{ago, capitalize};
use crate::ui::theme::palette;
use crate::ui::widgets::{chamber_with, empty_state, freshness_line};

pub fn render(snapshot: &Snapshot) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(roster_chamber(snapshot))
        .child(news_chamber(snapshot))
}

fn freshness(snapshot: &Snapshot) -> Div {
    div().flex().items_center().justify_end().child(match snapshot.guild.fetched_at {
        Some(at) => freshness_line(format!("Updated {}", ago(at))),
        None => freshness_line("Not fetched yet"),
    })
}

fn roster_chamber(snapshot: &Snapshot) -> Div {
    let body = if snapshot.guild.roster.is_empty() {
        empty_state("No roster yet.").into_any_element()
    } else {
        let mut list = div().flex().flex_col().gap_2();
        for member in &snapshot.guild.roster {
            list = list.child(roster_member_row(member));
        }
        list.into_any_element()
    };
    chamber_with("Roster", div().flex().flex_col().gap_3().child(freshness(snapshot)).child(body))
}

fn roster_member_row(member: &RosterMember) -> Div {
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().text_sm().text_color(palette::ink()).child(member.name.clone()))
        .when_some(member.rank.clone(), |d, rank| {
            d.child(div().text_xs().text_color(palette::mist2()).child(rank))
        });
    let mut row = div().flex().flex_col().gap_1().py_2().px_3().bg(palette::night3()).child(header);
    for character in &member.characters {
        row = row.child(character_row(character));
    }
    row
}

fn character_row(character: &RosterCharacter) -> Div {
    let seen = match character.last_seen_at {
        Some(at) => format!("Seen {}", ago(at)),
        None => "Not seen yet".to_owned(),
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .pl_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(palette::mist()).child(character.name.clone()))
                .when_some(character.class.clone(), |d, class| {
                    d.child(div().text_xs().text_color(palette::mist2()).child(capitalize(&class)))
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::mist2())
                        .child(format!("Lv {}", character.level)),
                ),
        )
        .child(div().text_xs().text_color(palette::mist2()).child(seen))
}

fn news_chamber(snapshot: &Snapshot) -> Div {
    let body = if snapshot.guild.news.is_empty() {
        empty_state("No news yet.").into_any_element()
    } else {
        let mut list = div().flex().flex_col().gap_2();
        for item in &snapshot.guild.news {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py_2()
                    .px_3()
                    .bg(palette::night3())
                    .child(div().text_sm().text_color(palette::ink()).child(item.text.clone()))
                    .child(div().text_xs().text_color(palette::mist2()).child(ago(item.at))),
            );
        }
        list.into_any_element()
    };
    chamber_with("Guild News", div().flex().flex_col().gap_3().child(body))
}
