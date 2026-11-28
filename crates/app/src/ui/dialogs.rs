//! The account dialogs: add (sign in with a pasted code), switch (with what it affects) and
//! remove. Drawn by the root view so they open from any page or the sidebar switcher. Every
//! button carries a selector (`dialog-cancel`, `add-continue`, `add-submit`,
//! `add-start-again`, `add-open-again`, `switch-confirm`, `remove-confirm`).

use std::time::SystemTime;

use colab_playground_core::backend::SwitchImpact;
use ely_gpui_component::{
    buttons::{Button, ButtonVariant, CopyButton},
    feedback::InlineMessage,
    forms::Input,
    overlays::Dialog,
    primitives::{IconName, Severity},
    theme::{ActiveTheme, TextSize},
    typography::{Caption, MiddleEllipsis, Paragraph},
};
use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, SharedString, Styled, div};

use crate::{
    format,
    i18n::{keys, tr, tr_args},
    state::{AccountDialog, AddPhase, AppState, Load, Op},
    ui::{
        Inputs, error_view,
        parts::{Layer, busy_line, field},
        tagged,
    },
};

pub const SWITCH_NOTHING_SELECTOR: &str = "switch-nothing";
pub const SWITCH_SESSIONS_WATCHED_SELECTOR: &str = "switch-sessions-watched";
pub const SWITCH_SESSIONS_UNWATCHED_SELECTOR: &str = "switch-sessions-unwatched";
pub const SWITCH_UNMANAGED_UNKNOWN_SELECTOR: &str = "switch-unmanaged-unknown";

pub fn switch_session_selector(name: &str) -> String {
    format!("switch-session {name}")
}

pub fn switch_session_deadline_selector(name: &str) -> String {
    format!("switch-session-deadline {name}")
}

pub fn switch_session_app_only_selector(name: &str) -> String {
    format!("switch-session-app-only {name}")
}

pub fn render(
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> Option<AnyElement> {
    let dialog = state.dialog()?;
    Some(match dialog {
        AccountDialog::Add(phase) => add(phase, handle, inputs, cx),
        AccountDialog::Switch {
            email,
            impact,
            error,
            ..
        } => switch(email.as_deref(), impact, error.as_ref(), state, handle, cx),
        AccountDialog::Remove { email, error, .. } => {
            remove(email.as_deref(), error.as_ref(), state, handle, cx)
        }
    })
}

fn add(phase: &AddPhase, handle: &Entity<AppState>, inputs: &Inputs, cx: &App) -> AnyElement {
    let dialog = Dialog::new(
        "add-account",
        tr(cx, keys::ADD_TITLE),
        Layer::Dialog.close(handle),
    );
    let cancel_label = tr(cx, keys::ACTION_CANCEL);
    let continue_label = tr(cx, keys::ADD_CONTINUE);
    let submit_label = tr(cx, keys::ADD_SUBMIT);
    let again_label = tr(cx, keys::ADD_START_AGAIN);
    let dialog = match phase {
        AddPhase::Intro => {
            let start = handle.clone();
            dialog
                .child(Paragraph::new(tr(cx, keys::ADD_INTRO)))
                .child(InlineMessage::new(Severity::Info, tr(cx, keys::ADD_IMPACT)))
                .action({
                    let handle = handle.clone();
                    move |_| Layer::Dialog.cancel_button(cancel_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "add-continue",
                        Button::new("add-continue", continue_label)
                            .primary()
                            .icon(IconName::ExternalLink)
                            .on_click(move |_, _, cx| {
                                start.update(cx, |state, cx| state.begin_sign_in(cx))
                            }),
                    )
                })
        }
        AddPhase::Starting => dialog
            .child(busy_line("add-starting", tr(cx, keys::ADD_STARTING)))
            .action({
                let handle = handle.clone();
                move |_| Layer::Dialog.cancel_button(cancel_label, handle)
            }),
        AddPhase::Code { url } | AddPhase::Submitting { url } => {
            let submitting = matches!(phase, AddPhase::Submitting { .. });
            let theme = cx.theme();
            let open = url.clone();
            let link = div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.mono_family.clone())
                        .text_size(theme.text_size(TextSize::Xs))
                        .text_color(theme.colors.fg_muted)
                        .child(MiddleEllipsis::new(url.clone())),
                )
                .child(CopyButton::new("add-copy-link", url.clone()))
                .child(tagged(
                    "add-open-again",
                    Button::new("add-open-again", tr(cx, keys::ADD_OPEN_AGAIN))
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| cx.open_url(&open)),
                ));
            let field = field(
                tr(cx, keys::ADD_CODE_LABEL),
                tagged("add-code-field", Input::new(&inputs.code)),
                cx,
            );
            let (submit, code) = (handle.clone(), inputs.code.clone());
            let mut dialog = dialog
                .child(Paragraph::new(tr(cx, keys::ADD_OPENED)))
                .child(link)
                .child(field);
            if submitting {
                dialog = dialog.child(busy_line("add-submitting", tr(cx, keys::ADD_SUBMITTING)));
            }
            dialog
                .action({
                    let handle = handle.clone();
                    move |_| Layer::Dialog.cancel_button(cancel_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "add-submit",
                        Button::new("add-submit", submit_label)
                            .primary()
                            .disabled(submitting)
                            .on_click(move |_, _, cx| {
                                let text = code.read(cx).text().trim().to_owned();
                                if text.is_empty() {
                                    return;
                                }
                                code.update(cx, |input, cx| input.set_text("", cx));
                                submit.update(cx, |state, cx| state.submit_code(text, cx));
                            }),
                    )
                })
        }
        AddPhase::Failed { error } => {
            let again = handle.clone();
            let close_label = tr(cx, keys::ACTION_CLOSE);
            let retry = error_view::retry(handle, AppState::begin_sign_in);
            dialog
                .child(error_view::alert(
                    "add",
                    None,
                    error,
                    Some(retry),
                    handle,
                    cx,
                ))
                .child(Caption::new(tr(cx, keys::ADD_ROLLED_BACK)))
                .action({
                    let handle = handle.clone();
                    move |_| Layer::Dialog.cancel_button(close_label, handle)
                })
                .action(move |_| {
                    tagged(
                        "add-start-again",
                        Button::new("add-start-again", again_label)
                            .primary()
                            .on_click(move |_, _, cx| {
                                again.update(cx, |state, cx| state.begin_sign_in(cx))
                            }),
                    )
                })
        }
    };
    dialog.into_any_element()
}

fn switch(
    email: Option<&str>,
    impact: &Load<(SwitchImpact, SystemTime)>,
    error: Option<&colab_playground_core::Error>,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let email = format::email(cx, email);
    let switching = state.is_busy(Op::Switch);
    let mut dialog = Dialog::new(
        "switch-account",
        tr_args(cx, keys::SWITCH_TITLE, &[("email", &email)]),
        Layer::Dialog.close(handle),
    );
    let ready = matches!(impact, Load::Ready(_));
    dialog = match impact {
        Load::Idle | Load::Loading => {
            dialog.child(busy_line("switch-loading", tr(cx, keys::SWITCH_LOADING)))
        }
        Load::Failed(error) => dialog.child(error_view::alert(
            "switch-impact",
            None,
            error,
            None,
            handle,
            cx,
        )),
        Load::Ready((impact, now)) => dialog.child(impact_list(impact, *now, state, cx)),
    };
    if let Some(error) = error {
        dialog = dialog.child(error_view::alert("switch", None, error, None, handle, cx));
    }
    let confirm = handle.clone();
    let cancel_label = tr(cx, keys::ACTION_CANCEL);
    let confirm_label = tr(cx, keys::SWITCH_CONFIRM);
    dialog
        .action({
            let handle = handle.clone();
            move |_| Layer::Dialog.cancel_button(cancel_label, handle)
        })
        .action(move |_| {
            tagged(
                "switch-confirm",
                Button::new("switch-confirm", confirm_label)
                    .primary()
                    .loading(switching)
                    .disabled(!ready || switching)
                    .on_click(move |_, _, cx| {
                        confirm.update(cx, |state, cx| state.confirm_switch(cx))
                    }),
            )
        })
        .into_any_element()
}

/// What switching affects: tunnels that close and runtimes that keep billing.
fn impact_list(impact: &SwitchImpact, now: SystemTime, state: &AppState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    // "Nothing is running" needs a list that came back empty: runtimes that could not be
    // listed may still be billing.
    if impact.tunnels_closing.is_empty()
        && impact.sessions_left_running.is_empty()
        && impact
            .unmanaged_left_running
            .as_ref()
            .is_some_and(Vec::is_empty)
    {
        return tagged(
            SWITCH_NOTHING_SELECTOR,
            Paragraph::new(tr(cx, keys::SWITCH_NOTHING)),
        )
        .into_any_element();
    }
    let item = |text: SharedString| {
        div()
            .pl_3()
            .text_size(theme.text_size(TextSize::Sm))
            .child(format!("• {text}"))
    };
    let mut list = div().flex().flex_col().gap_2();
    if !impact.tunnels_closing.is_empty() {
        list = list
            .child(Paragraph::new(tr(cx, keys::SWITCH_TUNNELS)))
            .children(impact.tunnels_closing.iter().map(|tunnel| {
                item(tr_args(
                    cx,
                    keys::SWITCH_TUNNEL,
                    &[
                        ("local", &tunnel.spec.local_port.to_string()),
                        ("session", &tunnel.spec.session),
                        ("remote", &tunnel.spec.remote_port.to_string()),
                    ],
                ))
            }));
    }
    // Switching leaves the app open, so the guard keeps enforcing app-side limits on every account.
    let mut watched = Vec::new();
    let mut unwatched = Vec::new();
    for session in &impact.sessions_left_running {
        if let Some(deadline) = session.auto_stop_at() {
            watched.push((session, deadline));
        } else {
            unwatched.push(session);
        }
    }
    if !watched.is_empty() {
        list = list
            .child(tagged(
                SWITCH_SESSIONS_WATCHED_SELECTOR,
                Paragraph::new(tr(cx, keys::SWITCH_SESSIONS)),
            ))
            .children(watched.into_iter().map(|(session, deadline)| {
                let hardware = tr(cx, format::accelerator_key(session.accelerator)).to_string();
                let left = deadline.duration_since(now).unwrap_or_default();
                let armed = session
                    .watchdog
                    .as_ref()
                    .and_then(|watchdog| watchdog.enforced_deadline())
                    .is_some();
                let key = if armed {
                    keys::SWITCH_SESSION_DEADLINE
                } else {
                    keys::SWITCH_SESSION_DEADLINE_APP_ONLY
                };
                let row = item(tr_args(
                    cx,
                    key,
                    &[
                        ("name", &session.name),
                        ("hardware", &hardware),
                        ("time", &format::duration(cx, left)),
                    ],
                ));
                let row = if armed {
                    row
                } else {
                    tagged(switch_session_app_only_selector(&session.name), row)
                };
                tagged(
                    switch_session_selector(&session.name),
                    tagged(switch_session_deadline_selector(&session.name), row),
                )
            }));
    }
    if !unwatched.is_empty() {
        list = list
            .child(tagged(
                SWITCH_SESSIONS_UNWATCHED_SELECTOR,
                Paragraph::new(tr(cx, keys::SWITCH_SESSIONS_UNWATCHED)),
            ))
            .children(unwatched.into_iter().map(|session| {
                let hardware = tr(cx, format::accelerator_key(session.accelerator)).to_string();
                let row = item(tr_args(
                    cx,
                    keys::SWITCH_SESSION,
                    &[("name", &session.name), ("hardware", &hardware)],
                ));
                tagged(switch_session_selector(&session.name), row)
            }));
    }
    let Some(unmanaged) = &impact.unmanaged_left_running else {
        return tagged(
            "switch-impact",
            list.child(tagged(
                SWITCH_UNMANAGED_UNKNOWN_SELECTOR,
                InlineMessage::new(Severity::Warning, tr(cx, keys::SWITCH_UNMANAGED_UNKNOWN)),
            )),
        )
        .into_any_element();
    };
    if !unmanaged.is_empty() {
        // Runtimes opened in the browser or elsewhere keep billing with no automatic stop.
        list = list
            .child(Paragraph::new(tr(cx, keys::SWITCH_UNMANAGED)))
            .children(unmanaged.iter().map(|runtime| {
                let label = state
                    .runtime(runtime)
                    .map(crate::state::runtime_label)
                    .or_else(|| runtime.name.clone())
                    .unwrap_or_else(|| runtime.endpoint.clone());
                item(tr_args(
                    cx,
                    keys::SWITCH_UNMANAGED_ITEM,
                    &[("name", &label)],
                ))
            }));
    }
    tagged("switch-impact", list).into_any_element()
}

fn remove(
    email: Option<&str>,
    error: Option<&colab_playground_core::Error>,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let email = format::email(cx, email);
    let removing = state.is_busy(Op::Remove);
    let mut dialog = Dialog::new(
        "remove-account",
        tr_args(cx, keys::REMOVE_TITLE, &[("email", &email)]),
        Layer::Dialog.close(handle),
    )
    .child(Paragraph::new(tr(cx, keys::REMOVE_BODY)));
    if let Some(error) = error {
        dialog = dialog.child(error_view::alert("remove", None, error, None, handle, cx));
    }
    let confirm = handle.clone();
    let cancel_label = tr(cx, keys::ACTION_CANCEL);
    let confirm_label = tr(cx, keys::REMOVE_CONFIRM);
    dialog
        .action({
            let handle = handle.clone();
            move |_| Layer::Dialog.cancel_button(cancel_label, handle)
        })
        .action(move |_| {
            tagged(
                "remove-confirm",
                Button::new("remove-confirm", confirm_label)
                    .variant(ButtonVariant::Danger)
                    .loading(removing)
                    .disabled(removing)
                    .on_click(move |_, _, cx| {
                        confirm.update(cx, |state, cx| state.confirm_remove(cx))
                    }),
            )
        })
        .into_any_element()
}
