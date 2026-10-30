//! Brand widgets shared by every page: the "chamber" panel (citadel.css geometry: night gradient
//! fill, 1px stone outer line, 1px gold-inner line 5px inset, bracketed gold corners with a cut
//! corner and a small solid diamond, a notched ribbon centred on the top border), drawn marks
//! (quest "!"/"?", diamond, caret), checks, diamond state tags, quest-log rows, an item tooltip
//! card, a freshness line and an empty state. Everything is painted with styled `div`s and
//! `canvas` paths, not images or a stock icon font, per `DESIGN.md`.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Div, InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, canvas, div, linear_color_stop,
    linear_gradient, point, px,
};

use crate::ui::theme::{DISPLAY_FONT, palette};

/// Upper-cases a heading and joins its letters with a thin space, approximating tracked caps.
pub fn tracked_caps(text: &str) -> SharedString {
    let upper = text.to_uppercase();
    let mut out = String::with_capacity(upper.len() * 2);
    for (i, ch) in upper.chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    out.into()
}

/// A chamber: the only container. Night gradient fill, 1px stone outer line, 1px gold-inner line
/// 5px inside it, bracketed gold corners (an L with a cut corner plus a small solid diamond), and
/// a ribbon centred on the top edge carrying the title. Chambers never nest.
pub fn chamber_with(title: impl Into<SharedString>, content: impl IntoElement) -> Div {
    div()
        .relative()
        .h_full()
        .bg(linear_gradient(
            180.,
            linear_color_stop(palette::night3(), 0.),
            linear_color_stop(palette::night(), 1.),
        ))
        .border_1()
        .border_color(palette::stone())
        .child(
            div()
                .relative()
                .h_full()
                .m(px(5.))
                .border_1()
                .border_color(palette::gold_inner())
                .pt(px(20.))
                .pb(px(16.))
                .px(px(16.))
                .child(content)
                .child(corner(true, true))
                .child(corner(true, false))
                .child(corner(false, true))
                .child(corner(false, false)),
        )
        .child(
            div()
                .absolute()
                .top(px(-12.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(ribbon(title)),
        )
}

/// A gold corner bracket: an L with a cut corner and a small solid diamond at the joint, 20px.
fn corner(top: bool, left: bool) -> Div {
    let size = px(20.);
    let mut wrap = div().absolute().w(size).h(size);
    wrap = if top { wrap.top(px(-1.)) } else { wrap.bottom(px(-1.)) };
    wrap = if left { wrap.left(px(-1.)) } else { wrap.right(px(-1.)) };
    wrap.child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let w = bounds.size.width;
                let h = bounds.size.height;
                let cut = px(6.);
                // An L shape with the inner corner cut at 45 degrees.
                let (ox, oy) = (o.x, o.y);
                let pts: Vec<_> = if top && left {
                    vec![
                        point(ox, oy + h),
                        point(ox, oy + cut),
                        point(ox + cut, oy),
                        point(ox + w, oy),
                    ]
                } else if top && !left {
                    vec![
                        point(ox, oy),
                        point(ox + w - cut, oy),
                        point(ox + w, oy + cut),
                        point(ox + w, oy + h),
                    ]
                } else if !top && left {
                    vec![
                        point(ox, oy),
                        point(ox, oy + h - cut),
                        point(ox + cut, oy + h),
                        point(ox + w, oy + h),
                    ]
                } else {
                    vec![
                        point(ox + w, oy),
                        point(ox + w, oy + h - cut),
                        point(ox + w - cut, oy + h),
                        point(ox, oy + h),
                    ]
                };
                let mut stroke = PathBuilder::stroke(px(1.2));
                stroke.move_to(pts[0]);
                for p in &pts[1..] {
                    stroke.line_to(*p);
                }
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, palette::gold());
                }
                // The small solid diamond at the inner joint.
                let joint = if top && left {
                    point(ox + cut + px(2.), oy + cut + px(2.))
                } else if top && !left {
                    point(ox + w - cut - px(2.), oy + cut + px(2.))
                } else if !top && left {
                    point(ox + cut + px(2.), oy + h - cut - px(2.))
                } else {
                    point(ox + w - cut - px(2.), oy + h - cut - px(2.))
                };
                let d = px(3.);
                let mut diamond = PathBuilder::fill();
                diamond.move_to(point(joint.x, joint.y - d));
                diamond.line_to(point(joint.x + d, joint.y));
                diamond.line_to(point(joint.x, joint.y + d));
                diamond.line_to(point(joint.x - d, joint.y));
                diamond.close();
                if let Ok(path) = diamond.build() {
                    window.paint_path(path, palette::gold());
                }
            },
        )
        .absolute()
        .inset_0(),
    )
}

/// A notched ribbon (royal-2 to royal to royal-deep gradient, gold-deep top/bottom lines, ends
/// notched inward) centred on a chamber's top border, carrying the chamber's title.
fn ribbon(title: impl Into<SharedString>) -> Div {
    let title = title.into();
    div()
        .relative()
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .px(px(22.))
        .child(ribbon_shape())
        .child(
            div()
                .relative()
                .font_family(DISPLAY_FONT)
                .text_xs()
                .text_color(palette::ink())
                .child(tracked_caps(&title)),
        )
}

/// Paints the ribbon's gradient fill and gold-deep top/bottom lines, notched inward at both ends.
fn ribbon_shape() -> impl IntoElement {
    canvas(
        |_bounds, _window, _cx| {},
        |bounds, _, window, _cx| {
            let notch = (bounds.size.height / 2.).min(px(10.));
            let o = bounds.origin;
            let w = bounds.size.width;
            let h = bounds.size.height;
            let pts = [
                point(o.x, o.y),
                point(o.x + notch, o.y + h / 2.),
                point(o.x, o.y + h),
                point(o.x + w, o.y + h),
                point(o.x + w - notch, o.y + h / 2.),
                point(o.x + w, o.y),
            ];

            let mut fill = PathBuilder::fill();
            fill.move_to(pts[0]);
            for p in &pts[1..] {
                fill.line_to(*p);
            }
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, palette::royal());
            }

            for (a, b) in [(pts[0], pts[5]), (pts[2], pts[3])] {
                let mut line = PathBuilder::stroke(px(1.));
                line.move_to(a);
                line.line_to(b);
                if let Ok(path) = line.build() {
                    window.paint_path(path, palette::gold_deep());
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Tracked-caps sub-heading followed by a gold-deep line that fades to the right.
pub fn sub_heading(text: impl Into<SharedString>) -> Div {
    let text = text.into();
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .font_family(DISPLAY_FONT)
                .text_xs()
                .text_color(palette::gold_deep())
                .child(tracked_caps(&text)),
        )
        .child(div().flex_1().h(px(1.)).bg(linear_gradient(
            90.,
            linear_color_stop(palette::gold_deep(), 0.),
            linear_color_stop(palette::gold_deep().opacity(0.), 1.),
        )))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Warn,
    Bad,
}

/// A diamond state tag (never a filled pill): a 5px gold/mist/error diamond followed by tracked
/// caps. Gold = needs you / pending; mist = done / quiet; error = failed.
pub fn state_tag(label: impl Into<SharedString>, tone: Tone) -> Div {
    let color = match tone {
        Tone::Neutral | Tone::Good => palette::mist(),
        Tone::Warn => palette::gold(),
        Tone::Bad => palette::warm_error(),
    };
    div().flex().items_center().gap_2().child(diamond_mark(px(5.), color)).child(
        div()
            .font_family(DISPLAY_FONT)
            .text_size(px(12.5))
            .text_color(color)
            .child(tracked_caps(&label.into())),
    )
}

/// A small solid diamond (rotated square), the system's one recurring drawn glyph.
pub fn diamond_mark(size: gpui_kit::Pixels, color: gpui_kit::Hsla) -> Div {
    div().relative().w(size).h(size).child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let w = bounds.size.width;
                let h = bounds.size.height;
                let mut fill = PathBuilder::fill();
                fill.move_to(point(o.x + w / 2., o.y));
                fill.line_to(point(o.x + w, o.y + h / 2.));
                fill.line_to(point(o.x + w / 2., o.y + h));
                fill.line_to(point(o.x, o.y + h / 2.));
                fill.close();
                if let Ok(path) = fill.build() {
                    window.paint_path(path, color);
                }
            },
        )
        .absolute()
        .inset_0(),
    )
}

/// A gold "!" for open/needs-action, a dim "?" for done: the quest marks.
pub fn quest_mark(done: bool) -> Div {
    let color = if done { palette::mist2() } else { palette::gold() };
    div()
        .w(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .font_family(DISPLAY_FONT)
        .text_color(color)
        .child(if done { "?" } else { "!" })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaretDir {
    Up,
    Down,
    Right,
}

/// A drawn caret chevron (never a stock icon font), 1.2px mist/gold line: points up, down, or
/// right. Used for disclosures and expand/collapse rows.
pub fn caret(dir: CaretDir, size: gpui_kit::Pixels, color: gpui_kit::Hsla) -> Div {
    div().relative().w(size).h(size).flex_shrink_0().child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let w = bounds.size.width;
                let h = bounds.size.height;
                let pts = match dir {
                    CaretDir::Down => [
                        point(o.x, o.y + h * 0.3),
                        point(o.x + w / 2., o.y + h * 0.7),
                        point(o.x + w, o.y + h * 0.3),
                    ],
                    CaretDir::Up => [
                        point(o.x, o.y + h * 0.7),
                        point(o.x + w / 2., o.y + h * 0.3),
                        point(o.x + w, o.y + h * 0.7),
                    ],
                    CaretDir::Right => [
                        point(o.x + w * 0.3, o.y),
                        point(o.x + w * 0.7, o.y + h / 2.),
                        point(o.x + w * 0.3, o.y + h),
                    ],
                };
                let mut stroke = PathBuilder::stroke(px(1.2));
                stroke.move_to(pts[0]);
                stroke.line_to(pts[1]);
                stroke.line_to(pts[2]);
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, color);
                }
            },
        )
        .absolute()
        .inset_0(),
    )
}

/// A quiet button whose edge and label warm to error on hover (Sign out, Remove folder): well
/// fill, 1px stone edge, mist caps label. Never a filled background.
pub fn danger_button(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(28.))
        .px(px(12.))
        .bg(palette::well())
        .border_1()
        .border_color(palette::stone())
        .cursor_pointer()
        .font_family(DISPLAY_FONT)
        .text_size(px(12.))
        .text_color(palette::mist())
        .hover(|s| s.border_color(palette::warm_error()).text_color(palette::warm_error()))
        .on_click(on_click)
        .child(tracked_caps(&label.into()))
}

/// A 16px square check well; checked = gold-deep edge with a 1px gold check stroke. No pill toggle.
pub fn checkbox(
    id: impl Into<gpui_kit::ElementId>,
    checked: bool,
    on_toggle: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .w(px(16.))
        .h(px(16.))
        .flex_shrink_0()
        .border_1()
        .border_color(if checked { palette::gold_deep() } else { palette::stone() })
        .bg(palette::well())
        .cursor_pointer()
        .on_click(on_toggle)
        .when(checked, |d| {
            d.child(
                canvas(
                    |_, _, _| {},
                    |bounds, _, window, _| {
                        let o = bounds.origin;
                        let w = bounds.size.width;
                        let h = bounds.size.height;
                        let mut stroke = PathBuilder::stroke(px(1.));
                        stroke.move_to(point(o.x + w * 0.2, o.y + h * 0.55));
                        stroke.line_to(point(o.x + w * 0.42, o.y + h * 0.78));
                        stroke.line_to(point(o.x + w * 0.82, o.y + h * 0.25));
                        if let Ok(path) = stroke.build() {
                            window.paint_path(path, palette::gold());
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
        })
}

/// A labeled check row: the 16px check well plus its label in body type to its right.
pub fn labeled_check(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_toggle: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .child(checkbox("check", checked, on_toggle))
        .child(div().text_sm().text_color(palette::mist()).child(label.into()))
}

/// A dim "synced 2h ago" style freshness line, italic mist.
pub fn freshness_line(text: impl Into<SharedString>) -> Div {
    div().italic().text_xs().text_color(palette::mist()).child(text.into())
}

/// A quiet placeholder for a list/section with nothing in it yet.
pub fn empty_state(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .py(px(28.))
        .text_color(palette::mist2())
        .child(diamond_mark(px(8.), palette::stone()))
        .child(div().text_sm().child(text.into()))
}

/// One row of a quest log (History, Raids, accounts): quest mark at left, title in display caps,
/// a mist meta line, time over day at the right in tabular numerals. Hover tints royal and turns
/// the title gold; rows divide with stone-2 hairlines above this element's caller.
pub fn quest_log_row(
    id: impl Into<gpui_kit::ElementId>,
    done: bool,
    title: impl Into<SharedString>,
    meta: impl Into<SharedString>,
    time: impl Into<SharedString>,
    day: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .group("quest-row")
        .flex()
        .items_center()
        .gap_3()
        .px(px(10.))
        .py(px(10.))
        .cursor_pointer()
        .child(quest_mark(done))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .font_family(DISPLAY_FONT)
                        .text_size(px(14.))
                        .text_color(palette::ink())
                        .group_hover("quest-row", |s| s.text_color(palette::gold()))
                        .child(tracked_caps(&title.into())),
                )
                .child(div().text_xs().text_color(palette::mist()).child(meta.into())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .flex_shrink_0()
                .child(
                    div()
                        .text_sm()
                        .text_color(palette::ink())
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .child(time.into()),
                )
                .child(div().text_xs().text_color(palette::mist2()).child(day.into())),
        )
}

/// An item tooltip card (Addon): name in epic quality color, a version line, label/value rows,
/// release notes as flavor text.
pub fn item_tooltip(
    name: impl Into<SharedString>,
    version_line: impl Into<SharedString>,
    rows: Vec<(SharedString, SharedString)>,
    flavor_text: Option<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .font_family(DISPLAY_FONT)
                .text_size(px(16.))
                .text_color(gpui_kit::rgb(0xa335ee))
                .child(tracked_caps(&name.into())),
        )
        .child(div().text_xs().text_color(palette::mist2()).child(version_line.into()))
        .child(div().flex().flex_col().gap_1().children(rows.into_iter().map(|(label, value)| {
            div()
                .flex()
                .justify_between()
                .text_sm()
                .child(div().text_color(palette::mist()).child(label))
                .child(div().text_color(palette::ink()).child(value))
        })))
        .when_some(flavor_text, |d, text| {
            d.child(div().italic().text_sm().text_color(palette::flavor()).mt(px(4.)).child(text))
        })
}
