//! The gate: the only reachable screen until `AccountState::SignedIn`. Full window, the gate
//! painting scrimmed, centred crest, wordmark, one line of body copy, the gold plaque "Sign in
//! with sidequest-guild.com". While waiting: the code, a countdown, and quiet buttons to reopen
//! the page or cancel. A signed-in account that isn't in the guild sees a plain-English message
//! and a Discord link. Nothing else is reachable from here.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{
    Div, InteractiveElement as _, IntoElement, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, div, img, linear_color_stop,
    linear_gradient, px,
};

use crate::engine::EngineHandle;
use crate::state::{AccountState, Command, LoginState, Snapshot};
use crate::ui::theme::{BODY_FONT, DISPLAY_FONT, palette};
use crate::ui::widgets::tracked_caps;

pub fn render(snapshot: &Snapshot, engine: &EngineHandle) -> impl IntoElement {
    let body: Div = match (&snapshot.account, &snapshot.login) {
        (AccountState::NotInGuild, _) => not_in_guild(),
        (_, LoginState::Waiting { user_code, expires_at, .. }) => {
            waiting(engine, user_code, *expires_at)
        }
        (_, LoginState::Starting) => starting(),
        (_, LoginState::Failed(message)) => failed(engine, message),
        (AccountState::Unverified(message), _) => unverified(engine, message),
        (AccountState::Checking, _) => checking(),
        _ => sign_in(engine),
    };

    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .bg(palette::night())
        .child(
            img("plates/hero-art.webp")
                .absolute()
                .inset_0()
                .w_full()
                .h_full()
                .object_fit(gpui_kit::ObjectFit::Cover),
        )
        .child(div().absolute().inset_0().bg(linear_gradient(
            180.,
            linear_color_stop(palette::night().opacity(0.35), 0.),
            linear_color_stop(palette::night(), 1.),
        )))
        .child(
            div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_4()
                .px(px(32.))
                .child(img("plates/nav-crest.png").w(px(48.)).h(px(69.)))
                .child(
                    div()
                        .font_family(DISPLAY_FONT)
                        .text_size(px(20.))
                        .text_color(palette::ink())
                        .child(tracked_caps("SideQuest Companion")),
                )
                .child(body),
        )
}

fn sign_in(engine: &EngineHandle) -> Div {
    let engine = engine.clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_5()
        .child(
            div()
                .font_family(BODY_FONT)
                .text_size(px(17.))
                .text_color(palette::mist())
                .child("Sign in with your SideQuest account to start."),
        )
        .child(plaque("Sign in with sidequest-guild.com", move |_, _, _| {
            engine.send(Command::BeginBrowserLogin)
        }))
        .child(use_token_link())
}

fn checking() -> Div {
    div().flex().flex_col().items_center().gap_3().child(
        div()
            .font_family(BODY_FONT)
            .text_size(px(17.))
            .text_color(palette::mist())
            .child("Checking your sign-in\u{2026}"),
    )
}

fn starting() -> Div {
    div().flex().flex_col().items_center().gap_3().child(
        div()
            .font_family(BODY_FONT)
            .text_size(px(17.))
            .text_color(palette::mist())
            .child("Opening sidequest-guild.com\u{2026}"),
    )
}

fn waiting(
    engine: &EngineHandle,
    user_code: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Div {
    let remaining = (expires_at - chrono::Utc::now()).num_seconds().max(0);
    let countdown = format!("{}:{:02}", remaining / 60, remaining % 60);
    let engine_reopen = engine.clone();
    let engine_cancel = engine.clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_4()
        .child(
            div()
                .font_family(DISPLAY_FONT)
                .text_size(px(32.))
                .text_color(palette::gold())
                .child(tracked_caps(user_code)),
        )
        .child(
            div()
                .font_family(BODY_FONT)
                .text_size(px(15.))
                .text_color(palette::mist())
                .text_center()
                .child("Check that the website shows the same code, then click Authorize."),
        )
        .child(div().text_sm().text_color(palette::mist2()).child(countdown))
        .child(
            div()
                .flex()
                .gap_3()
                .child(
                    Button::new("gate-reopen")
                        .ghost()
                        .label("Open the page again")
                        .on_click(move |_, _, _| engine_reopen.send(Command::ReopenBrowserLogin)),
                )
                .child(
                    Button::new("gate-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(move |_, _, _| engine_cancel.send(Command::CancelBrowserLogin)),
                ),
        )
        .child(use_token_link())
}

fn failed(engine: &EngineHandle, message: &str) -> Div {
    let engine = engine.clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_4()
        .child(
            div()
                .text_sm()
                .text_color(palette::warm_error())
                .text_center()
                .child(message.to_owned()),
        )
        .child(plaque("Sign in with sidequest-guild.com", move |_, _, _| {
            engine.send(Command::BeginBrowserLogin)
        }))
        .child(use_token_link())
}

fn unverified(engine: &EngineHandle, message: &str) -> Div {
    let engine = engine.clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_4()
        .child(
            div()
                .text_sm()
                .text_color(palette::warm_error())
                .text_center()
                .child(message.to_owned()),
        )
        .child(
            Button::new("gate-retry")
                .ghost()
                .label("Try again")
                .on_click(move |_, _, _| engine.send(Command::TestConnection)),
        )
        .child(use_token_link())
}

fn not_in_guild() -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_4()
        .child(
            div()
                .font_family(BODY_FONT)
                .text_size(px(17.))
                .text_color(palette::mist())
                .text_center()
                .child("This app is for SideQuest members."),
        )
        .child(
            Button::new("gate-discord")
                .ghost()
                .label("Join the Discord")
                .on_click(|_, _, cx| cx.open_url("https://discord.gg/sidequest")),
        )
}

fn use_token_link() -> Stateful<Div> {
    div()
        .id("gate-token-link")
        .text_xs()
        .text_color(palette::mist2())
        .cursor_pointer()
        .child("Use a sync token instead")
}

/// The gold plaque: `cta-plate.png` art behind a near-black caps label. The one primary action
/// per screen.
fn plaque(
    label: impl Into<gpui_kit::SharedString>,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> Stateful<Div> {
    div()
        .id("gate-plaque")
        .relative()
        .w(px(320.))
        .h(px(54.))
        .cursor_pointer()
        .on_click(on_click)
        .child(
            img("plates/cta-plate.webp")
                .absolute()
                .inset_0()
                .w_full()
                .h_full()
                .object_fit(gpui_kit::ObjectFit::Fill),
        )
        .child(
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .font_family(BODY_FONT)
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_size(px(14.))
                .text_color(gpui_kit::rgb(0x17130c))
                .child(tracked_caps(&label.into())),
        )
}
