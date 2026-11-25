//! Overview: the first-run checklist until everything is set up, then the compute-unit
//! summary.

use colab_playground_core::host::Stream;
use ely_gpui_component::{
    buttons::{Button, ButtonVariant, IconButton},
    data_display::{Badge, Tone},
    feedback::{Alert, InlineMessage},
    forms::{Choice, Select},
    layout::{Card, CardHeader, Page, ScrollArea, Well},
    lists::List,
    motion::Spinner,
    primitives::{Icon, IconName, Severity},
    terminal::AnsiText,
    theme::{ActiveTheme, ControlSize, IconSize, TextSize},
    typography::Caption,
};
use gpui::{
    AnyElement, App, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::FluentBuilder as _,
};

use crate::{
    format,
    i18n::{keys, tr, tr_args},
    platform,
    route::Route,
    state::{
        AppState, EnvironmentStep, InstallPhase, Op, SignInStatus, Step, StepState,
        install_methods, usage_is_stale, usage_stale_note,
    },
    ui::{
        error_view,
        parts::{captioned, figure, step_icon, step_spinner},
        tagged,
        usage::{accounts_missing, time_left, unmanaged_warning},
    },
};

pub const CHECKLIST_SELECTOR: &str = "overview-checklist";
/// Selector of the summary card shown once set up.
pub const SUMMARY_SELECTOR: &str = "overview-summary";
/// Selector of the mark on a runtime count that leaves accounts out.
pub const RUNTIMES_INCOMPLETE_SELECTOR: &str = "summary runtimes-incomplete";
/// Selector of the line naming the accounts a runtime count leaves out.
pub const RUNTIMES_UNCOUNTED_SELECTOR: &str = "summary runtimes-uncounted";
pub const INSTALL_SELECTOR: &str = "install-start";
/// Selector around the Cancel button of a running install.
pub const INSTALL_CANCEL_SELECTOR: &str = "install-cancel";
pub const INSTALL_LOG_SELECTOR: &str = "install-log";
/// Selector of the alert shown when an install finished but `colab` is still not found.
pub const INSTALL_NOT_FOUND_SELECTOR: &str = "install-not-found";
/// Selector around that alert's button that opens Settings.
pub const INSTALL_OPEN_SETTINGS_SELECTOR: &str = "install-open-settings";
/// Selector around the sign-in button of the checklist.
pub const SIGN_IN_SELECTOR: &str = "checklist-sign-in";
pub const CHECK_AGAIN_SELECTOR: &str = "checklist-check-again";
pub const QUICK_DEPLOY_SELECTOR: &str = "overview-quick-deploy";

/// The selector of a checklist step, with its state: `step <name> <state>`.
pub fn step_selector(step: EnvironmentStep, state: &StepState) -> String {
    let name = match step {
        EnvironmentStep::Wsl => "wsl",
        EnvironmentStep::ShellTools => "shell",
        EnvironmentStep::Cli => "cli",
        EnvironmentStep::SignIn => "signin",
    };
    let state = match state {
        StepState::Checking => "checking",
        StepState::Done => "done",
        StepState::Todo => "todo",
        StepState::Waiting => "waiting",
        StepState::Failed(_) => "failed",
    };
    format!("step {name} {state}")
}

pub fn render(state: &AppState, handle: &Entity<AppState>, cx: &App) -> Page {
    let mut page = super::page(Route::Overview, keys::OVERVIEW_SUBTITLE, cx);
    if let Some(error) = state.backend_error() {
        return page.child(error_view::alert(
            "backend",
            Some(tr(cx, keys::BACKEND_START_FAILED)),
            error,
            None,
            handle,
            cx,
        ));
    }
    let checklist = state.checklist();
    page = if checklist.complete() {
        page.child(summary(state, handle, cx))
    } else {
        page.child(
            tagged(
                CHECKLIST_SELECTOR,
                Card::new()
                    .header(
                        CardHeader::new(tr(cx, keys::CHECKLIST_TITLE))
                            .description(tr(cx, keys::CHECKLIST_DESCRIPTION)),
                    )
                    .child(
                        List::new().divided().children(
                            checklist
                                .steps
                                .iter()
                                .enumerate()
                                .map(|(ix, step)| step_row(ix, step, state, handle, cx)),
                        ),
                    ),
            )
            .w_full(),
        )
    };
    page.children(unmanaged_warning(state, handle, cx))
        .children(quick_deploy(state, handle, cx))
}

/// The selector of a quick-deploy card's button: `deploy <plugin id>`.
pub fn deploy_selector(id: &str) -> String {
    format!("deploy {id}")
}

/// Built-in plugins as "Deploy a model" cards; one click opens the run form. Shown once the
/// checklist is complete.
fn quick_deploy(state: &AppState, handle: &Entity<AppState>, cx: &App) -> Option<AnyElement> {
    if !state.checklist().complete() {
        return None;
    }
    let plugins = state.quick_deploys();
    if plugins.is_empty() {
        return None;
    }
    let theme = cx.theme();
    let tag = state.language().tag();
    let cards = plugins.into_iter().map(|info| {
        let id = info.manifest.id.clone();
        let open = handle.clone();
        let open_id = id.clone();
        Card::new()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Rocket)
                            .size(IconSize::Sm)
                            .color(theme.colors.accent),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Base))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(state.plugin_name(info)),
                    ),
            )
            .children(
                info.manifest
                    .description
                    .get(tag)
                    .map(|d| Caption::new(d.to_owned())),
            )
            .child(
                div().flex().child(tagged(
                    deploy_selector(&id),
                    Button::new(
                        SharedString::from(format!("deploy-{id}")),
                        tr(cx, keys::DEPLOY_ACTION),
                    )
                    .primary()
                    .icon(IconName::Rocket)
                    .on_click(move |_, _, cx| {
                        open.update(cx, |state, cx| state.open_run_form(open_id.clone(), cx))
                    }),
                )),
            )
            .w_full()
    });
    Some(
        tagged(
            QUICK_DEPLOY_SELECTOR,
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(theme.text_size(TextSize::Lg))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(tr(cx, keys::DEPLOY_TITLE)),
                        )
                        .child(Caption::new(tr(cx, keys::DEPLOY_SUBTITLE))),
                )
                .children(cards),
        )
        .w_full()
        .into_any_element(),
    )
}

fn step_row(
    ix: usize,
    step: &Step,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let colors = &theme.colors;
    let mark = match &step.state {
        StepState::Checking => step_spinner(("step-spinner", ix)),
        StepState::Done => step_icon(IconName::CircleCheck, colors.success),
        StepState::Failed(_) => step_icon(IconName::CircleAlert, colors.danger),
        StepState::Todo => step_icon(IconName::Circle, colors.accent),
        StepState::Waiting => step_icon(IconName::Circle, colors.fg_subtle),
    };
    let title = tr(
        cx,
        match step.step {
            EnvironmentStep::Wsl => keys::STEP_WSL_TITLE,
            EnvironmentStep::ShellTools => keys::STEP_SHELL_TITLE,
            EnvironmentStep::Cli => keys::STEP_CLI_TITLE,
            EnvironmentStep::SignIn => keys::STEP_SIGNIN_TITLE,
        },
    );
    let (body, extra) = step_body(step, state, handle, cx);
    tagged(
        step_selector(step.step, &step.state),
        div()
            .flex()
            .items_start()
            .gap_3()
            .py_3()
            .child(div().flex_none().pt_0p5().child(mark))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(match step.state {
                                StepState::Waiting => colors.fg_muted,
                                _ => colors.fg,
                            })
                            .child(title),
                    )
                    .children(body.map(|body| {
                        div()
                            .text_size(theme.text_size(TextSize::Sm))
                            .text_color(colors.fg_muted)
                            .child(body)
                    }))
                    .children(extra),
            ),
    )
    .into_any_element()
}

/// The line under a step's title and anything it needs below (buttons, the install).
fn step_body(
    step: &Step,
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> (Option<SharedString>, Vec<AnyElement>) {
    let report = state.probe().ready();
    match (&step.state, step.step) {
        (StepState::Checking, _) => (Some(tr(cx, keys::STEP_CHECKING)), vec![]),
        (StepState::Waiting, _) => (Some(tr(cx, keys::STEP_WAITING)), vec![]),
        (StepState::Failed(error), _) => {
            let retry = check_again(handle);
            (
                None,
                vec![error_view::alert(
                    "step",
                    None,
                    error,
                    Some(retry),
                    handle,
                    cx,
                )],
            )
        }
        (StepState::Done, EnvironmentStep::Wsl) => {
            let distro = state
                .settings()
                .wsl_distro
                .clone()
                .or_else(|| report.and_then(|r| r.wsl_distros.first().cloned()))
                .unwrap_or_default();
            (
                Some(tr_args(cx, keys::STEP_WSL_DONE, &[("distro", &distro)])),
                vec![],
            )
        }
        (StepState::Done, EnvironmentStep::ShellTools) => {
            let key = if report.is_some_and(|r| r.ssh.is_some()) {
                keys::STEP_SHELL_DONE
            } else {
                keys::STEP_SHELL_NO_SSH
            };
            (Some(tr(cx, key)), vec![])
        }
        (StepState::Done, EnvironmentStep::Cli) => {
            let body = match report.and_then(|r| r.colab_version.clone()) {
                Some(version) => tr_args(cx, keys::STEP_CLI_DONE, &[("version", &version)]),
                None => tr(cx, keys::STEP_CLI_DONE_UNKNOWN),
            };
            (Some(body), vec![])
        }
        (StepState::Done, EnvironmentStep::SignIn) => {
            let email = format::email(cx, state.active_email().as_deref());
            (
                Some(tr_args(cx, keys::STEP_SIGNIN_DONE, &[("email", &email)])),
                vec![],
            )
        }
        (StepState::Todo, EnvironmentStep::Wsl) => {
            let buttons = div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("install-wsl", tr(cx, keys::ACTION_INSTALL_WSL))
                        .icon(IconName::ExternalLink)
                        .size(ControlSize::Sm)
                        .on_click(|_, _, cx| cx.open_url(platform::WSL_INSTALL_URL)),
                )
                .child(check_again_button(handle, cx));
            (
                Some(tr(cx, keys::STEP_WSL_TODO)),
                vec![buttons.into_any_element()],
            )
        }
        (StepState::Todo, EnvironmentStep::ShellTools) => (
            Some(tr(cx, keys::STEP_SHELL_TODO)),
            vec![
                div()
                    .flex()
                    .child(check_again_button(handle, cx))
                    .into_any_element(),
            ],
        ),
        (StepState::Todo, EnvironmentStep::Cli) => install(state, handle, cx),
        (StepState::Todo, EnvironmentStep::SignIn) => sign_in(state, handle, cx),
    }
}

fn check_again(handle: &Entity<AppState>) -> error_view::Retry {
    error_view::retry(handle, AppState::check_environment)
}

fn check_again_button(handle: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let handle = handle.clone();
    tagged(
        CHECK_AGAIN_SELECTOR,
        Button::new("check-again", tr(cx, keys::ACTION_CHECK_AGAIN))
            .icon(IconName::RefreshCw)
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .on_click(move |_, _, cx| handle.update(cx, |state, cx| state.check_environment(cx))),
    )
}

/// The CLI step: choose a method, install, follow the log, cancel, see why it failed.
fn install(
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> (Option<SharedString>, Vec<AnyElement>) {
    let theme = cx.theme();
    let install = state.install();
    let running = state.is_busy(Op::Install);
    // A finished install is followed by a check for `colab`; until it ends, the step still
    // shows the report from before the install.
    let checking = state.is_busy(Op::Environment);
    let mut extra: Vec<AnyElement> = Vec::new();
    let body = match &install.phase {
        InstallPhase::Running if running => None,
        InstallPhase::Done if checking => Some(tr(cx, keys::STEP_CHECKING)),
        InstallPhase::Done => None,
        InstallPhase::Cancelled => Some(tr(cx, keys::INSTALL_CANCELLED)),
        _ => Some(tr(cx, keys::STEP_CLI_TODO)),
    };
    if running {
        let cancelling = state.is_cancelling(Op::Install);
        let cancel = handle.clone();
        extra.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(Spinner::new("install-spinner").size(IconSize::Sm))
                .child(
                    div()
                        .flex_1()
                        .text_size(theme.text_size(TextSize::Sm))
                        .child(tr(cx, keys::INSTALL_RUNNING)),
                )
                .child(tagged(
                    INSTALL_CANCEL_SELECTOR,
                    Button::new(
                        "install-cancel",
                        tr(
                            cx,
                            if cancelling {
                                keys::INSTALL_CANCELLING
                            } else {
                                keys::ACTION_CANCEL
                            },
                        ),
                    )
                    .size(ControlSize::Sm)
                    .disabled(cancelling)
                    .on_click(move |_, _, cx| {
                        cancel.update(cx, |state, cx| state.cancel_install(cx))
                    }),
                ))
                .into_any_element(),
        );
    } else {
        if let InstallPhase::Failed(error) = &install.phase {
            let retry = error_view::retry(handle, AppState::start_install);
            extra.push(error_view::alert(
                "install",
                Some(tr(cx, keys::INSTALL_FAILED)),
                error,
                Some(retry),
                handle,
                cx,
            ));
        }
        if install.phase == InstallPhase::Done && !checking {
            extra.push(install_not_found(handle, cx));
        }
        let methods = install_methods(state.probe().ready());
        let chosen = state.chosen_install_method();
        let start = handle.clone();
        let mut controls = div().flex().flex_wrap().items_center().gap_2();
        if methods.len() > 1 {
            let (pick, offered) = (handle.clone(), methods.clone());
            let choices = methods.iter().map(|method| {
                Choice::new(
                    format::install_method_value(*method),
                    tr(cx, format::install_method_key(*method)),
                )
            });
            controls = controls
                .child(Caption::new(tr(cx, keys::INSTALL_METHOD_LABEL)))
                .child(
                    tagged(
                        "install-method",
                        Select::new("install-method-select", choices)
                            .label(tr(cx, keys::INSTALL_METHOD_LABEL))
                            .selected(format::install_method_value(chosen))
                            .size(ControlSize::Sm)
                            .on_change(move |value, _, cx| {
                                let method = offered.iter().copied().find(|method| {
                                    format::install_method_value(*method) == value.as_ref()
                                });
                                if let Some(method) = method {
                                    pick.update(cx, |state, cx| {
                                        state.set_install_method(method, cx)
                                    });
                                }
                            }),
                    )
                    .w(theme.menu_width()),
                );
        } else {
            controls = controls.child(Caption::new(tr(cx, format::install_method_key(chosen))));
        }
        controls = controls.child(tagged(
            INSTALL_SELECTOR,
            Button::new("install-start", tr(cx, keys::INSTALL_START))
                .icon(IconName::Download)
                .primary()
                .size(ControlSize::Sm)
                .on_click(move |_, _, cx| start.update(cx, |state, cx| state.start_install(cx))),
        ));
        extra.push(controls.into_any_element());
    }
    if !install.lines.is_empty() {
        extra.push(install_log(state, cx));
    }
    (body, extra)
}

/// The installer succeeded, yet the check after it misses `colab`: it went to a folder the probe
/// does not search, where the same install would put it again.
fn install_not_found(handle: &Entity<AppState>, cx: &App) -> AnyElement {
    let open = handle.clone();
    let alert = Alert::new(
        INSTALL_NOT_FOUND_SELECTOR,
        Severity::Warning,
        tr(cx, keys::INSTALL_NOT_FOUND_TITLE),
    )
    .body(tr(cx, keys::INSTALL_NOT_FOUND_BODY))
    .action(tagged(
        INSTALL_OPEN_SETTINGS_SELECTOR,
        Button::new("install-open-settings", tr(cx, keys::NEW_OPEN_SETTINGS))
            .size(ControlSize::Sm)
            .on_click(move |_, _, cx| {
                open.update(cx, |state, cx| state.navigate(Route::Settings, cx))
            }),
    ))
    .action(check_again_button(handle, cx));
    tagged(INSTALL_NOT_FOUND_SELECTOR, alert).into_any_element()
}

/// The install's output: monospace, colours kept, following the newest line.
fn install_log(state: &AppState, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let colors = &theme.colors;
    let install = state.install();
    let lines = install.lines.iter().enumerate().map(|(ix, line)| {
        div()
            .id(("install-line", ix))
            .when(line.stream == Stream::Stderr, |row| {
                row.text_color(colors.danger)
            })
            .child(AnsiText::new(line.text.clone()))
    });
    tagged(
        INSTALL_LOG_SELECTOR,
        Well::new()
            .h(theme.list_max_height())
            .w_full()
            .font_family(theme.mono_family.clone())
            .text_size(theme.text_size(TextSize::Xs))
            .child(
                ScrollArea::new("install-log-scroll")
                    .vertical()
                    .size_full()
                    .track_scroll(&install.scroll)
                    .children(lines),
            ),
    )
    .into_any_element()
}

/// The sign-in step: save the CLI's account, or add one.
fn sign_in(
    state: &AppState,
    handle: &Entity<AppState>,
    cx: &App,
) -> (Option<SharedString>, Vec<AnyElement>) {
    let handle = handle.clone();
    match state.sign_in_status() {
        SignInStatus::Unadopted { .. } => (
            Some(tr(cx, keys::STEP_SIGNIN_UNADOPTED)),
            vec![
                tagged(
                    SIGN_IN_SELECTOR,
                    Button::new("checklist-adopt", tr(cx, keys::ACCOUNTS_ADOPT))
                        .primary()
                        .size(ControlSize::Sm)
                        .disabled(state.is_busy(Op::Adopt))
                        .on_click(move |_, _, cx| {
                            handle.update(cx, |state, cx| state.adopt_account(cx))
                        }),
                )
                .self_start()
                .into_any_element(),
            ],
        ),
        _ => (
            Some(tr(cx, keys::STEP_SIGNIN_TODO)),
            vec![
                tagged(
                    SIGN_IN_SELECTOR,
                    Button::new("checklist-sign-in", tr(cx, keys::ACTION_SIGN_IN))
                        .icon(IconName::LogIn)
                        .primary()
                        .size(ControlSize::Sm)
                        .on_click(move |_, _, cx| {
                            handle.update(cx, |state, cx| state.open_add_account(cx))
                        }),
                )
                .self_start()
                .into_any_element(),
            ],
        ),
    }
}

/// Balance, burn rate, time left and running runtimes, once everything is set up.
fn summary(state: &AppState, handle: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let usage = state.active_usage();
    let snapshot = usage.and_then(|usage| usage.snapshot);
    let unknown = || tr(cx, keys::SUMMARY_UNKNOWN);
    let balance = snapshot.map_or_else(unknown, |s| format::cu(cx, s.balance_cu));
    let rate = snapshot.map_or_else(unknown, |s| format::cu_rate(cx, s.rate_cu_per_hour));
    let left = time_left(usage, cx);
    // Unknown is never shown as zero: with no account's usage known the count is unknown, and
    // a sum that leaves accounts out is marked and says which.
    let totals = state.usage_summary();
    let uncounted = state.uncounted_accounts();
    let incomplete = totals.known > 0 && !uncounted.is_empty();
    let runtimes = if totals.known == 0 {
        unknown()
    } else {
        SharedString::from(totals.runtimes.to_string())
    };
    let runtimes_mark = incomplete.then(|| {
        tagged(
            RUNTIMES_INCOMPLETE_SELECTOR,
            Badge::new(tr(cx, keys::SUMMARY_INCOMPLETE)).tone(Tone::Warning),
        )
        .into_any_element()
    });
    let runtimes_note = if incomplete {
        tagged(
            RUNTIMES_UNCOUNTED_SELECTOR,
            Caption::new(not_counted(&uncounted, cx)),
        )
        .into_any_element()
    } else {
        Caption::new(tr(cx, keys::SUMMARY_RUNTIMES_ALL)).into_any_element()
    };
    let stale_note = snapshot
        .filter(|s| usage_is_stale(s, state.usage_now()))
        .map(|s| usage_stale_note(&s));
    let stale = stale_note.is_some();
    let refresh = handle.clone();
    let email = format::email(cx, state.active_email().as_deref());
    let mut header = CardHeader::new(tr(cx, keys::SUMMARY_TITLE)).description(tr_args(
        cx,
        keys::SUMMARY_ACTIVE,
        &[("email", &email)],
    ));
    header = header.action(
        div()
            .flex()
            .items_center()
            .gap_2()
            .children(stale.then(|| Badge::new(tr(cx, keys::USAGE_OUTDATED)).tone(Tone::Warning)))
            .child(tagged(
                "summary-refresh",
                IconButton::new("summary-refresh", IconName::RefreshCw)
                    .tooltip(tr(cx, keys::USAGE_REFRESH))
                    .variant(ButtonVariant::Ghost)
                    .disabled(state.is_busy(Op::Usage))
                    .on_click(move |_, _, cx| {
                        refresh.update(cx, |state, cx| state.refresh_usage(true, cx))
                    }),
            )),
    );
    let tile = |id: &str, label: &str, value: SharedString, mark: Option<AnyElement>| {
        tagged(
            format!("summary {id} {value}"),
            captioned(
                tr(cx, label),
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(figure(value, TextSize::Xl, cx))
                    .children(mark),
            )
            .flex_1()
            .min_w(theme.label_width())
            .gap_1(),
        )
    };
    let failure = state.usage_error().map(|error| {
        InlineMessage::new(
            Severity::Warning,
            crate::errors::explain(error, false, cx).message,
        )
    });
    tagged(
        SUMMARY_SELECTOR,
        Card::new()
            .header(header)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_6()
                    .child(tile("balance", keys::SUMMARY_BALANCE, balance, None))
                    .child(tile("rate", keys::SUMMARY_RATE, rate, None))
                    .child(tile("left", keys::SUMMARY_TIME_LEFT, left, None))
                    .child(
                        tile("runtimes", keys::SUMMARY_RUNTIMES, runtimes, runtimes_mark)
                            .child(runtimes_note),
                    ),
            )
            .children(stale_note.map(|note| InlineMessage::new(Severity::Warning, tr(cx, note))))
            .children(failure),
    )
    .w_full()
}

const MAX_EMAILS_ON_TILE: usize = 2;

/// The accounts a runtime count leaves out: by email while they fit on the tile (all with an
/// email), otherwise how many.
fn not_counted(uncounted: &[Option<String>], cx: &App) -> SharedString {
    let emails: Option<Vec<&str>> = uncounted.iter().map(Option::as_deref).collect();
    match emails {
        Some(emails) if emails.len() <= MAX_EMAILS_ON_TILE => tr_args(
            cx,
            keys::SUMMARY_NOT_COUNTED,
            &[("emails", &emails.join(", "))],
        ),
        _ => accounts_missing(uncounted.len(), cx),
    }
}
