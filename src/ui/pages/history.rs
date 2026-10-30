use gpui_kit::component::{WindowExt as _, clipboard::Clipboard};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::state::{HistoryEntry, HistoryKind, Outcome, Snapshot};
use crate::ui::app::AppRoot;
use crate::ui::format::ago;
use crate::ui::theme::palette;
use crate::ui::widgets::{CaretDir, Tone, caret, chamber_with, empty_state, state_tag};

const KINDS: &[(HistoryKind, &str)] = &[
    (HistoryKind::Upload, "Upload"),
    (HistoryKind::ImportStaged, "Import staged"),
    (HistoryKind::AddonInstalled, "Addon installed"),
    (HistoryKind::AppUpdated, "App updated"),
];

const OUTCOMES: &[(Outcome, &str)] =
    &[(Outcome::Success, "Success"), (Outcome::Skipped, "Skipped"), (Outcome::Failed, "Failed")];

pub fn render(
    snapshot: &Snapshot,
    kind_filter: Option<HistoryKind>,
    outcome_filter: Option<Outcome>,
    expanded: Option<usize>,
    cx: &mut Context<AppRoot>,
) -> impl IntoElement {
    let rows: Vec<(usize, &HistoryEntry)> = snapshot
        .history
        .iter()
        .enumerate()
        .filter(|(_, e)| kind_filter.is_none_or(|k| k == e.kind))
        .filter(|(_, e)| outcome_filter.is_none_or(|o| o == e.outcome))
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(16.))
                .gap_y(px(6.))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(filter_chip(
                            "All",
                            kind_filter.is_none(),
                            cx.listener(|this, _, _, cx| {
                                this.history_kind_filter = None;
                                cx.notify();
                            }),
                        ))
                        .children(KINDS.iter().map(|(kind, label)| {
                            let kind = *kind;
                            filter_chip(
                                label,
                                kind_filter == Some(kind),
                                cx.listener(move |this, _, _, cx| {
                                    this.history_kind_filter = Some(kind);
                                    cx.notify();
                                }),
                            )
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(filter_chip(
                            "Any outcome",
                            outcome_filter.is_none(),
                            cx.listener(|this, _, _, cx| {
                                this.history_outcome_filter = None;
                                cx.notify();
                            }),
                        ))
                        .children(OUTCOMES.iter().map(|(outcome, label)| {
                            let outcome = *outcome;
                            filter_chip(
                                label,
                                outcome_filter == Some(outcome),
                                cx.listener(move |this, _, _, cx| {
                                    this.history_outcome_filter = Some(outcome);
                                    cx.notify();
                                }),
                            )
                        })),
                ),
        )
        .child(if rows.is_empty() {
            empty_state("No history matches this filter.").into_any_element()
        } else {
            chamber_with(
                "History",
                div().flex().flex_col().gap_1().children(
                    rows.into_iter()
                        .map(|(i, entry)| history_row(i, entry, expanded == Some(i), cx)),
                ),
            )
            .into_any_element()
        })
}

fn filter_chip(
    label: &str,
    active: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> impl IntoElement {
    div()
        .id(format!("history-filter-{label}"))
        .flex()
        .items_center()
        .h(px(22.))
        .px(px(8.))
        .when(active, |d| d.bg(palette::royal()).border_1().border_color(palette::gold_deep()))
        .when(!active, |d| d.bg(palette::night3()))
        .text_xs()
        .text_color(if active { palette::ink() } else { palette::mist2() })
        .child(label.to_owned())
        .on_click(move |ev, window, cx| on_click(ev, window, cx))
}

fn kind_label(kind: HistoryKind) -> &'static str {
    match kind {
        HistoryKind::Upload => "Upload",
        HistoryKind::ImportStaged => "Import staged",
        HistoryKind::AddonInstalled => "Addon installed",
        HistoryKind::AppUpdated => "App updated",
    }
}

fn history_row(
    index: usize,
    entry: &HistoryEntry,
    expanded: bool,
    cx: &mut Context<AppRoot>,
) -> Div {
    let tone = match entry.outcome {
        Outcome::Success => Tone::Good,
        Outcome::Skipped => Tone::Neutral,
        Outcome::Failed => Tone::Bad,
    };
    let has_detail = entry.detail.is_some();
    let detail = entry.detail.clone().unwrap_or_default();

    div()
        .flex()
        .flex_col()
        .when(index % 2 == 1, |d| d.bg(palette::night3().opacity(0.4)))
        .child(
            div()
                .id(("history-row", index))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .py_2()
                .px_3()
                .when(has_detail, |d| d.cursor_pointer())
                .when(has_detail, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.history_expanded =
                            if this.history_expanded == Some(index) { None } else { Some(index) };
                        cx.notify();
                    }))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(crate::ui::widgets::quest_mark(entry.outcome == Outcome::Success))
                        .child(
                            div()
                                .font_family(crate::ui::theme::DISPLAY_FONT)
                                .text_size(px(14.))
                                .text_color(palette::ink())
                                .child(crate::ui::widgets::tracked_caps(&entry.subject)),
                        )
                        .child(state_tag(kind_label(entry.kind), tone))
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::mist2())
                                .child(entry.summary.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_xs().text_color(palette::mist2()).child(ago(entry.at)))
                        .when(has_detail, |d| {
                            d.child(caret(
                                if expanded { CaretDir::Up } else { CaretDir::Down },
                                px(10.),
                                palette::mist2(),
                            ))
                        }),
                ),
        )
        .when(expanded && has_detail, |d| {
            d.child(
                div()
                    .mx_3()
                    .mb_2()
                    .p_2()
                    .bg(palette::night())
                    .border_1()
                    .border_color(palette::stone())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_xs().text_color(palette::mist()).child(detail.clone()))
                    .child(Clipboard::new(("copy-detail", index)).value(detail.clone()).on_copied(
                        |_, window, cx| window.push_notification("Copied to clipboard.", cx),
                    )),
            )
        })
}
