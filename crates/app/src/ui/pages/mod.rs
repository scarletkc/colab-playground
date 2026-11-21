//! One function per page. A page that needs local state (text fields, scroll positions) keeps it
//! in `AppState` or in an entity the root view owns (see `ui::Inputs`).

pub mod accounts;
pub mod defaults;
pub mod deployments;
pub mod overview;
pub mod plugins;
pub mod sessions;
pub mod settings;

use colab_playground_core::updater::SelfUpdate;
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    feedback::Alert,
    layout::Page,
    primitives::Severity,
    theme::{ActiveTheme, ContainerSize, ControlSize},
};
use gpui::{
    AnyElement, App, Context, Entity, IntoElement, ParentElement, SharedString, Styled, div,
};

use crate::{
    i18n::{keys, tr, tr_args},
    route::Route,
    state::AppState,
    ui::{Inputs, overlays::unsupported_reason, tagged},
};

pub fn render(
    state: &AppState,
    handle: &Entity<AppState>,
    inputs: &Inputs,
    cx: &App,
) -> AnyElement {
    let route = state.route();
    let notice = update_banner(state, handle, cx);
    let page = match route {
        Route::Overview => overview::render(state, handle, cx),
        Route::Sessions => sessions::render(state, handle, cx),
        Route::Plugins => plugins::render(state, handle, inputs, cx),
        Route::Deployments => deployments::render(state, handle, cx),
        Route::Accounts => accounts::render(state, handle, cx),
        Route::Settings => settings::render(state, handle, inputs, cx),
    };
    let framed = frame(route, page, cx);
    match notice {
        Some(notice) => div()
            .w_full()
            .flex()
            .flex_col()
            .child(notice)
            .child(framed)
            .into_any_element(),
        None => framed,
    }
}

pub const UPDATE_NOTICE_SELECTOR: &str = "update-notice";
/// "Download and restart" on the banner.
pub const UPDATE_NOTICE_INSTALL: &str = "update-notice-install";
/// "What's new" (the release page) on the banner, beside "Download and restart".
pub const UPDATE_NOTICE_NOTES: &str = "update-notice-notes";
/// "Open download page" on the banner, when this copy cannot replace itself.
pub const UPDATE_NOTICE_OPEN: &str = "update-notice-open";
pub const UPDATE_NOTICE_DISMISS: &str = "update-notice-dismiss";

fn banner_button(
    id: &'static str,
    label: SharedString,
    primary: bool,
    handle: &Entity<AppState>,
    act: fn(&mut AppState, &mut Context<AppState>),
) -> impl IntoElement {
    let handle = handle.clone();
    let button = Button::new(id, label)
        .size(ControlSize::Sm)
        .on_click(move |_, _, cx| handle.update(cx, act));
    tagged(
        id,
        if primary {
            button.primary()
        } else {
            button.variant(ButtonVariant::Ghost)
        },
    )
}

/// A newer release, announced once per version above every page until dismissed for that
/// version. It offers "Download and restart" with the release notes beside it, or, when this
/// copy cannot replace itself, says why and offers the release page.
fn update_banner(state: &AppState, handle: &Entity<AppState>, cx: &App) -> Option<AnyElement> {
    let (version, _) = state.update_notice()?;
    let theme = cx.theme();
    let body = match state.self_update() {
        Some(SelfUpdate::Unsupported(reason)) => tr_args(
            cx,
            keys::UPDATE_NOTICE_BODY_MANUAL,
            &[("why", &unsupported_reason(reason, cx))],
        ),
        _ => tr(cx, keys::UPDATE_NOTICE_BODY),
    };
    let mut actions = div().flex().flex_wrap().gap_2();
    if state.can_self_update() {
        actions = actions
            .child(banner_button(
                UPDATE_NOTICE_INSTALL,
                tr(cx, keys::UPDATE_INSTALL),
                true,
                handle,
                AppState::begin_update,
            ))
            .child(banner_button(
                UPDATE_NOTICE_NOTES,
                tr(cx, keys::UPDATE_WHATS_NEW),
                false,
                handle,
                |state, cx| state.open_update_page(cx),
            ));
    } else {
        actions = actions.child(banner_button(
            UPDATE_NOTICE_OPEN,
            tr(cx, keys::UPDATE_OPEN_PAGE),
            true,
            handle,
            |state, cx| state.open_update_page(cx),
        ));
    }
    let dismiss = handle.clone();
    actions = actions.child(tagged(
        UPDATE_NOTICE_DISMISS,
        Button::new(UPDATE_NOTICE_DISMISS, tr(cx, keys::UPDATE_DISMISS))
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .on_click({
                let version = version.clone();
                move |_, _, cx| {
                    dismiss.update(cx, |state, cx| state.dismiss_update(version.clone(), cx))
                }
            }),
    ));
    Some(
        tagged(
            UPDATE_NOTICE_SELECTOR,
            div()
                .w_full()
                .max_w(theme.container_width(ContainerSize::Lg))
                .mx_auto()
                .px_6()
                .pt_4()
                .child(
                    Alert::new(
                        "update-notice",
                        Severity::Info,
                        tr_args(cx, keys::UPDATE_NOTICE_TITLE, &[("version", &version)]),
                    )
                    .body(body)
                    .action(actions),
                ),
        )
        .w_full()
        .into_any_element(),
    )
}

/// The page under its title, centred at a readable width. In test builds the frame carries the
/// selector `page <slug> <title>`, so tests can check which page shows in which language.
fn frame(route: Route, page: Page, cx: &App) -> AnyElement {
    let title = tr(cx, route.title_key());
    let theme = cx.theme();
    tagged(
        format!("page {} {title}", route.slug()),
        div()
            .w_full()
            .max_w(theme.container_width(ContainerSize::Lg))
            .mx_auto()
            .child(page),
    )
    .w_full()
    .into_any_element()
}

pub fn page(route: Route, subtitle_key: &str, cx: &App) -> Page {
    Page::new(tr(cx, route.title_key())).subtitle(tr(cx, subtitle_key))
}
