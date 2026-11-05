//! Settings: edits are staged on `SettingsPage` and sent as one [`Command::SaveSettings`] on Save;
//! Revert discards them back to the engine's last known [`Settings`].

use chrono::{DateTime, Utc};
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    collapsible::Collapsible,
    form::{Field, Form},
    input::{Input, InputState},
    radio::RadioGroup,
    spinner::Spinner,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, px,
};

use crate::engine::EngineHandle;
use crate::state::{
    AccountState, AppUpdate, Command, LoginState, ReminderScope, Settings, Snapshot,
};
use crate::ui::format::{EVENT_KINDS, event_kind_label, until};
use crate::ui::theme::{DISPLAY_FONT, palette};
use crate::ui::widgets::{
    CaretDir, Tone, caret, chamber_with, checkbox, danger_button, state_tag, tracked_caps,
};

pub struct SettingsPage {
    engine: EngineHandle,
    draft: Settings,
    dirty: bool,
    site_url: Entity<InputState>,
    token_input: Entity<InputState>,
    calendar_input: Entity<InputState>,
    token_disclosure_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(
        initial: &Snapshot,
        engine: EngineHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = initial.settings.clone();
        let site_url =
            cx.new(|cx| InputState::new(window, cx).default_value(draft.site_url.clone()));
        let token_input = cx.new(|cx| InputState::new(window, cx).placeholder("sqs_\u{2026}"));
        let calendar_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://sidequest-guild.com/api/calendar/\u{2026}.ics")
        });
        let site_url_sub = cx.subscribe_in(&site_url, window, |this, _, event, _, cx| {
            if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                this.mark_dirty(cx);
            }
        });
        Self {
            engine,
            draft,
            dirty: false,
            site_url,
            token_input,
            calendar_input,
            token_disclosure_open: false,
            _subscriptions: vec![site_url_sub],
        }
    }

    fn sync_from_snapshot(
        &mut self,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dirty {
            self.draft = snapshot.settings.clone();
            Self::set_input_value(&self.site_url, snapshot.settings.site_url.clone(), window, cx);
        }
    }

    /// `InputState::set_value` needs `&mut Window` via `VisualContext`, which only an async window
    /// context provides; a sibling entity's synchronous `render` can't call it directly.
    fn set_input_value(
        entity: &Entity<InputState>,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = entity.clone();
        cx.spawn_in(window, async move |_, cx| {
            let _ = entity.update_in(cx, |state, window, cx| state.set_value(value, window, cx));
        })
        .detach();
    }

    fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.draft.site_url = self.site_url.read(cx).value().to_string();
        self.engine.send(Command::SaveSettings(self.draft.clone()));
        self.dirty = false;
        cx.notify();
    }

    fn revert(&mut self, snapshot: &Settings, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = snapshot.clone();
        Self::set_input_value(&self.site_url, snapshot.site_url.clone(), window, cx);
        self.dirty = false;
        cx.notify();
    }

    pub fn render(
        &mut self,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.sync_from_snapshot(snapshot, window, cx);
        let is_officer = matches!(&snapshot.account, AccountState::SignedIn(m) if m.is_officer);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .child(self.account_section(snapshot, cx))
            .child(self.wow_folders_section(snapshot, cx))
            .when(is_officer, |d| d.child(self.sync_section(cx)))
            .child(self.addon_updates_section(cx))
            .child(self.reminders_section(snapshot, cx))
            .child(self.app_section(snapshot, cx))
            .child(self.about_section(snapshot))
            .child(save_bar(self.dirty, snapshot.settings.clone(), cx))
    }

    fn account_section(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) -> Div {
        let show_disclosure =
            !matches!(snapshot.login, LoginState::Starting | LoginState::Waiting { .. });
        let primary = match &snapshot.login {
            LoginState::Starting => login_starting_view(),
            LoginState::Waiting { user_code, verification_uri_complete, expires_at } => {
                login_waiting_view(&self.engine, user_code, verification_uri_complete, *expires_at)
            }
            LoginState::Failed(message) => login_failed_view(&self.engine, message),
            LoginState::Idle | LoginState::Done => match &snapshot.account {
                AccountState::SignedIn(member) => account_identity_view(&self.engine, member),
                _ => sign_in_prompt(&self.engine),
            },
        };

        section(
            "Account",
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    Form::new()
                        .child(Field::new().label("Site URL").child(Input::new(&self.site_url))),
                )
                .child(primary)
                .when(show_disclosure, |d| {
                    d.child(self.token_disclosure(&snapshot.settings.site_url, cx))
                }),
        )
    }

    /// A collapsed "Use a token instead" fallback for sites without browser sign-in, or for
    /// pasting a token saved elsewhere.
    fn token_disclosure(&mut self, site_url: &str, cx: &mut Context<Self>) -> Div {
        let open = self.token_disclosure_open;
        let engine_set = self.engine.clone();
        let engine_clear = self.engine.clone();
        let engine_test = self.engine.clone();
        let token_input = self.token_input.clone();
        let site_url = site_url.to_owned();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("token-disclosure-toggle")
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(palette::mist2())
                    .child(caret(
                        if open { CaretDir::Down } else { CaretDir::Right },
                        px(10.),
                        palette::mist2(),
                    ))
                    .child("Use a token instead")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.token_disclosure_open = !this.token_disclosure_open;
                        cx.notify();
                    })),
            )
            .child(
                Collapsible::new().open(open).motion_id("token-disclosure").content(
                    div()
                        .pt_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Input::new(&self.token_input).mask_toggle())
                        .child(Button::new("token-save").primary().label("Save & test").on_click({
                            let token_input = token_input.clone();
                            move |_, _, cx| {
                                let value = token_input.read(cx).value().to_string();
                                if !value.is_empty() {
                                    engine_set.send(Command::SetToken(value));
                                }
                            }
                        }))
                        .child(
                            Button::new("token-clear")
                                .ghost()
                                .label("Clear")
                                .on_click(move |_, _, _| engine_clear.send(Command::SignOut)),
                        )
                        .child(
                            Button::new("token-test")
                                .ghost()
                                .label("Test connection")
                                .on_click(move |_, _, _| engine_test.send(Command::TestConnection)),
                        )
                        .child(Button::new("get-token").link().label("Get a token").on_click(
                            move |_, _, cx| {
                                cx.open_url(&format!(
                                    "{}/officers/sync/",
                                    site_url.trim_end_matches('/')
                                ))
                            },
                        )),
                ),
            )
    }

    fn wow_folders_section(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) -> Div {
        let mut list = div().flex().flex_col().gap_2();
        for install in &snapshot.installs {
            let removable = matches!(install.source, crate::state::InstallSource::UserAdded);
            let path = install.root.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .text_color(palette::ink())
                            .child(install.root.display().to_string())
                            .child(
                                div().text_xs().text_color(palette::mist2()).child(if removable {
                                    "added"
                                } else {
                                    "detected"
                                }),
                            ),
                    )
                    .when(removable, |d| {
                        d.child(danger_button(
                            format!("remove-root-{}", path.display()),
                            "Remove",
                            cx.listener(move |this, _, _, cx| {
                                this.draft.wow_roots.retain(|r| r != &path);
                                this.mark_dirty(cx);
                            }),
                        ))
                    }),
            );
            for flavor in &install.flavors {
                for account in &flavor.accounts {
                    let enabled = account.enabled;
                    let sv = account.saved_variables.clone();
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl_4()
                            .py_1()
                            .text_xs()
                            .text_color(palette::mist())
                            .child(format!("{} \u{b7} {}", flavor.dir_name, account.name))
                            .child(checkbox(
                                format!("account-enabled-{}", account.name),
                                enabled,
                                cx.listener(move |this, _, _, cx| {
                                    this.draft.disabled_accounts.retain(|p| p != &sv);
                                    if enabled {
                                        this.draft.disabled_accounts.push(sv.clone());
                                    }
                                    this.mark_dirty(cx);
                                }),
                            )),
                    );
                }
            }
        }
        section(
            "WoW Folders",
            div().flex().flex_col().gap_2().child(list).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(palette::ink())
                                    .child("Auto-detect installs"),
                            )
                            .child(checkbox(
                                "auto-detect",
                                self.draft.auto_detect,
                                cx.listener(|this, _, _, cx| {
                                    this.draft.auto_detect = !this.draft.auto_detect;
                                    this.mark_dirty(cx);
                                }),
                            )),
                    )
                    .child(Button::new("add-folder").ghost().label("Add folder").on_click(
                        cx.listener(move |_this, _, _, cx| {
                            let task = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: None,
                            });
                            cx.spawn(async move |this, cx| {
                                if let Ok(Ok(Some(mut paths))) = task.await
                                    && let Some(root) = paths.pop()
                                {
                                    this.update(cx, |this, cx| {
                                        if !this.draft.wow_roots.contains(&root) {
                                            this.draft.wow_roots.push(root);
                                        }
                                        this.mark_dirty(cx);
                                    })
                                    .ok();
                                }
                            })
                            .detach();
                        }),
                    )),
            ),
        )
    }

    fn sync_section(&mut self, cx: &mut Context<Self>) -> Div {
        section(
            "Sync",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(labeled_switch(
                    "auto-sync",
                    "Upload new exports automatically",
                    self.draft.auto_sync,
                    cx.listener(|this, v, _, cx| {
                        this.draft.auto_sync = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "stage-imports",
                    "Stage game imports back to the addon",
                    self.draft.stage_imports,
                    cx.listener(|this, v, _, cx| {
                        this.draft.stage_imports = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "notify-sync",
                    "Notify on successful sync",
                    self.draft.notify_on_sync,
                    cx.listener(|this, v, _, cx| {
                        this.draft.notify_on_sync = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "notify-error",
                    "Notify on sync error",
                    self.draft.notify_on_error,
                    cx.listener(|this, v, _, cx| {
                        this.draft.notify_on_error = *v;
                        this.mark_dirty(cx);
                    }),
                )),
        )
    }

    fn addon_updates_section(&mut self, cx: &mut Context<Self>) -> Div {
        let engine_check = self.engine.clone();
        section(
            "Addon Updates",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(labeled_switch(
                    "check-addon",
                    "Check for addon updates",
                    self.draft.check_addon_updates,
                    cx.listener(|this, v, _, cx| {
                        this.draft.check_addon_updates = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "auto-update-addon",
                    "Install updates automatically while WoW is closed",
                    self.draft.auto_update_addon,
                    cx.listener(|this, v, _, cx| {
                        this.draft.auto_update_addon = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(
                    Button::new("check-addon-now")
                        .ghost()
                        .label("Check now")
                        .on_click(move |_, _, _| engine_check.send(Command::CheckAddonRelease)),
                ),
        )
    }

    fn reminders_section(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) -> Div {
        let scope_index = match self.draft.reminders.scope {
            ReminderScope::SignedUp => 0,
            ReminderScope::AllEvents => 1,
        };
        let calendar_input = self.calendar_input.clone();
        let engine_cal_set = self.engine.clone();
        let engine_cal_clear = self.engine.clone();
        section(
            "Reminders",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(labeled_switch(
                    "reminders-enabled",
                    "Remind me about events",
                    self.draft.reminders.enabled,
                    cx.listener(|this, v, _, cx| {
                        this.draft.reminders.enabled = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(
                    Field::new().label("Remind me for").child(
                        RadioGroup::new("reminder-scope")
                            .children(["Events I'm signed up for", "Every scheduled event"])
                            .selected_index(Some(scope_index))
                            .on_change(cx.listener(|this, v, _, cx| {
                                this.draft.reminders.scope = if *v == 0 {
                                    ReminderScope::SignedUp
                                } else {
                                    ReminderScope::AllEvents
                                };
                                this.mark_dirty(cx);
                            })),
                    ),
                )
                .child(div().flex().flex_wrap().gap_2().children(EVENT_KINDS.iter().map(|kind| {
                    let checked = self.draft.reminders.kinds.iter().any(|k| k == kind);
                    let kind = kind.to_string();
                    Checkbox::new(format!("reminder-kind-{kind}"))
                        .label(event_kind_label(&kind))
                        .checked(checked)
                        .on_change(cx.listener(move |this, checked, _, cx| {
                            if *checked {
                                if !this.draft.reminders.kinds.contains(&kind) {
                                    this.draft.reminders.kinds.push(kind.clone());
                                }
                            } else {
                                this.draft.reminders.kinds.retain(|k| k != &kind);
                            }
                            this.mark_dirty(cx);
                        }))
                })))
                .when(!snapshot.calendar_linked, |d| {
                    d.child(
                        Field::new().label("Private calendar link").child(
                            div()
                                .flex()
                                .gap_2()
                                .child(Input::new(&self.calendar_input))
                                .child(
                                    Button::new("calendar-save").primary().label("Save").on_click(
                                        {
                                            let calendar_input = calendar_input.clone();
                                            move |_, _, cx| {
                                                let value =
                                                    calendar_input.read(cx).value().to_string();
                                                if !value.is_empty() {
                                                    engine_cal_set
                                                        .send(Command::SetCalendarUrl(Some(value)));
                                                }
                                            }
                                        },
                                    ),
                                )
                                .child(
                                    Button::new("calendar-clear").ghost().label("Clear").on_click(
                                        move |_, _, _| {
                                            engine_cal_clear.send(Command::SetCalendarUrl(None))
                                        },
                                    ),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette::mist2())
                            .child("Find your link on the site under Account \u{2192} Calendar."),
                    )
                }),
        )
    }

    fn app_section(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) -> Div {
        let engine_check = self.engine.clone();
        let engine_restart = self.engine.clone();
        let (update_label, update_action): (SharedString, Option<&'static str>) = match &snapshot
            .app_update
        {
            AppUpdate::NotChecked => ("Not checked".into(), None),
            AppUpdate::Checking => ("Checking\u{2026}".into(), None),
            AppUpdate::UpToDate { .. } => ("Up to date".into(), None),
            AppUpdate::Available(r) => {
                (format!("v{} available", r.version).into(), Some("install"))
            }
            AppUpdate::Downloading { progress } => {
                (format!("Downloading\u{2026} {:.0}%", progress * 100.0).into(), None)
            }
            AppUpdate::ReadyToRestart(r) => {
                (format!("v{} ready \u{2014} restart to apply", r.version).into(), Some("restart"))
            }
            AppUpdate::Failed(e) => (format!("Failed: {e}").into(), None),
        };
        section(
            "App",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(labeled_switch(
                    "launch-at-login",
                    "Launch at login",
                    self.draft.launch_at_login,
                    cx.listener(|this, v, _, cx| {
                        this.draft.launch_at_login = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "start-hidden",
                    "Start hidden in the tray",
                    self.draft.start_hidden,
                    cx.listener(|this, v, _, cx| {
                        this.draft.start_hidden = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "close-to-tray",
                    "Closing the window keeps the app running",
                    self.draft.close_to_tray,
                    cx.listener(|this, v, _, cx| {
                        this.draft.close_to_tray = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "check-app-updates",
                    "Check for app updates",
                    self.draft.check_app_updates,
                    cx.listener(|this, v, _, cx| {
                        this.draft.check_app_updates = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(labeled_switch(
                    "reduce-motion",
                    "Reduce motion",
                    self.draft.reduce_motion,
                    cx.listener(|this, v, _, cx| {
                        this.draft.reduce_motion = *v;
                        this.mark_dirty(cx);
                    }),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_sm().text_color(palette::ink()).child("App update"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::mist2())
                                        .child(update_label),
                                )
                                .child(
                                    Button::new("app-update-check")
                                        .ghost()
                                        .xsmall()
                                        .label("Check now")
                                        .on_click(move |_, _, _| {
                                            engine_check.send(Command::CheckAppUpdate)
                                        }),
                                )
                                .when(update_action.is_some(), |d| {
                                    d.child(
                                        Button::new("app-update-install")
                                            .primary()
                                            .xsmall()
                                            .label("Install")
                                            .on_click(move |_, _, _| {
                                                engine_restart.send(Command::InstallAppUpdate)
                                            }),
                                    )
                                }),
                        ),
                ),
        )
    }

    fn about_section(&mut self, snapshot: &Snapshot) -> Div {
        section(
            "About",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().text_color(palette::ink()).child(format!("SideQuest Companion v{}", snapshot.app_version)))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Button::new("open-logs").ghost().label("Open logs").on_click(|_, _, _| {
                            if let Ok(dir) = crate::paths::log_dir() {
                                let _ = open::that(dir);
                            }
                        }))
                        .child(Button::new("open-config").ghost().label("Open config folder").on_click(|_, _, _| {
                            if let Ok(dir) = crate::paths::config_dir() {
                                let _ = open::that(dir);
                            }
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::mist2())
                        .child("Fonts: Libertinus Serif Display and EB Garamond, SIL Open Font License 1.1 (see assets/fonts/OFL.txt)."),
                ),
        )
    }
}

fn section(title: impl Into<SharedString>, body: impl IntoElement) -> Div {
    chamber_with(title, body)
}

fn labeled_switch(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_change: impl Fn(&bool, &mut Window, &mut gpui_kit::App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_sm().text_color(palette::ink()).child(label.into()))
        .child(checkbox(id, checked, move |_, window, cx| on_change(&!checked, window, cx)))
}

fn sign_in_prompt(engine: &EngineHandle) -> Div {
    let engine = engine.clone();
    div().child(
        Button::new("begin-browser-login")
            .primary()
            .large()
            .label("Sign in with sidequest-guild.com")
            .on_click(move |_, _, _| engine.send(Command::BeginBrowserLogin)),
    )
}

fn account_identity_view(engine: &EngineHandle, member: &crate::state::Member) -> Div {
    let engine = engine.clone();
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div().text_sm().text_color(palette::ink()).child(member.display_name.clone()),
                )
                .child(state_tag(
                    if member.is_officer { "Officer" } else { "Member" },
                    if member.is_officer { Tone::Warn } else { Tone::Neutral },
                )),
        )
        .child(danger_button("sign-out", "Sign out", move |_, _, _| engine.send(Command::SignOut)))
}

fn login_starting_view() -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(Spinner::new().small())
        .child(div().text_sm().text_color(palette::mist()).child("Opening your browser\u{2026}"))
}

fn login_waiting_view(
    engine: &EngineHandle,
    user_code: &str,
    _verification_uri_complete: &str,
    expires_at: DateTime<Utc>,
) -> Div {
    let engine_reopen = engine.clone();
    let engine_cancel = engine.clone();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(palette::well())
        .border_1()
        .border_color(palette::stone())
        .child(
            div()
                .font_family(DISPLAY_FONT)
                .text_2xl()
                .text_color(palette::gold())
                .child(tracked_caps(user_code)),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette::mist())
                .child("Check that the website shows the same code, then click Authorize."),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::mist2())
                .child(format!("Expires {}", until(expires_at))),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("reopen-login")
                        .link()
                        .label("Open the page again")
                        .on_click(move |_, _, _| engine_reopen.send(Command::ReopenBrowserLogin)),
                )
                .child(
                    Button::new("cancel-login")
                        .ghost()
                        .label("Cancel")
                        .on_click(move |_, _, _| engine_cancel.send(Command::CancelBrowserLogin)),
                ),
        )
}

fn login_failed_view(engine: &EngineHandle, message: &str) -> Div {
    let engine = engine.clone();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().text_color(palette::warm_error()).child(message.to_owned()))
        .child(
            Button::new("retry-login")
                .label("Try again")
                .on_click(move |_, _, _| engine.send(Command::BeginBrowserLogin)),
        )
}

fn save_bar(dirty: bool, saved: Settings, cx: &mut Context<SettingsPage>) -> Div {
    div().flex().items_center().justify_end().gap_2().pt_2().when(dirty, |d| {
        d.child(div().text_xs().text_color(palette::gold()).child("Unsaved changes"))
            .child(
                Button::new("settings-revert").ghost().label("Revert").on_click(
                    cx.listener(move |this, _, window, cx| this.revert(&saved, window, cx)),
                ),
            )
            .child(
                Button::new("settings-save")
                    .primary()
                    .label("Save")
                    .on_click(cx.listener(move |this, _, _, cx| this.save(cx))),
            )
    })
}
