//! The Deployments page: every deployment started from the app, the most recently used first,
//! to start again with the same settings ("Deploy again" opens the run form on the Plugins
//! page) or to remove from the list.

use colab_playground_core::plugins::{InputKind, InputValue, PluginInfo};
use ely_gpui_component::{
    buttons::{Button, ButtonVariant},
    data_display::{Badge, Tone},
    layout::{Card, Page},
    lists::List,
    primitives::IconName,
    theme::{ActiveTheme, ControlSize},
    typography::Caption,
};
use gpui::{
    AnyElement, App, Entity, FontWeight, IntoElement, ParentElement, SharedString, Styled, div,
};

use crate::{
    deployments::Deployment,
    format,
    i18n::{keys, tr},
    route::Route,
    state::AppState,
    ui::tagged,
};

/// Selector of the deployments list.
pub const DEPLOYMENTS: &str = "deployments";
/// Selector of a saved deployment's row: `deployment <id>`.
pub fn deployment_selector(id: &str) -> String {
    format!("deployment {id}")
}
pub fn redeploy_selector(id: &str) -> String {
    format!("deployment-again {id}")
}
pub fn forget_deployment_selector(id: &str) -> String {
    format!("deployment-forget {id}")
}

/// Selector of the line shown when nothing was deployed yet.
pub const DEPLOYMENTS_EMPTY: &str = "deployments-empty";

pub fn render(state: &AppState, handle: &Entity<AppState>, cx: &App) -> Page {
    let page = super::page(Route::Deployments, keys::DEPLOYMENTS_HINT, cx);
    if state.deployments().is_empty() {
        page.child(tagged(
            DEPLOYMENTS_EMPTY,
            Caption::new(tr(cx, keys::DEPLOYMENTS_EMPTY)),
        ))
    } else {
        page.child(list_card(state, handle, cx))
    }
}

/// What a saved deployment is about in a few words: its model repository when the plugin
/// takes one, else its first text input, else the plugin's name.
fn deployment_title(record: &Deployment, info: Option<&PluginInfo>, plugin_name: &str) -> String {
    let Some(info) = info else {
        return plugin_name.to_owned();
    };
    let value = |kinds: &[InputKind]| {
        info.manifest
            .inputs
            .iter()
            .filter(|input| kinds.contains(&input.kind))
            .find_map(|input| match record.inputs.get(&input.id) {
                Some(InputValue::Text(text)) if !text.trim().is_empty() => Some(text.clone()),
                _ => None,
            })
    };
    value(&[InputKind::HfModel])
        .or_else(|| value(&[InputKind::Text]))
        .unwrap_or_else(|| plugin_name.to_owned())
}

fn list_card(state: &AppState, handle: &Entity<AppState>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let now = state
        .backend_now()
        .unwrap_or_else(std::time::SystemTime::now);
    let rows = state.deployments().iter().map(|record| {
        let info = state.plugin(&record.plugin);
        let plugin_name = info
            .map(|info| state.plugin_name(info))
            .unwrap_or_else(|| record.plugin.clone());
        let usable = info.is_some_and(|info| info.enabled) && !state.is_running_plugin();
        let mut details = vec![plugin_name.clone()];
        details.push(match &record.hardware {
            Some(hardware) => tr(cx, format::accelerator_key(hardware.accelerator)).to_string(),
            None => tr(cx, keys::DEPLOYMENT_EXISTING).to_string(),
        });
        let used = std::time::UNIX_EPOCH + std::time::Duration::from_secs(record.last_used);
        details.push(format::age(cx, used, now).to_string());
        let mut title = div().flex().items_center().gap_2().min_w_0().child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .child(deployment_title(record, info, &plugin_name)),
        );
        if record.drive {
            title = title.child(Badge::new(tr(cx, keys::DEPLOYMENT_DRIVE)).tone(Tone::Info));
        }
        let again = {
            let (handle, id) = (handle.clone(), record.id.clone());
            Button::new(
                SharedString::from(format!("deployment-again-{}", record.id)),
                tr(cx, keys::DEPLOYMENT_AGAIN),
            )
            .size(ControlSize::Sm)
            .icon(IconName::Rocket)
            .disabled(!usable)
            .on_click(move |_, _, cx| handle.update(cx, |state, cx| state.redeploy(&id, cx)))
        };
        let forget = {
            let (handle, id) = (handle.clone(), record.id.clone());
            Button::new(
                SharedString::from(format!("deployment-forget-{}", record.id)),
                tr(cx, keys::ACTION_REMOVE),
            )
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Sm)
            .on_click(move |_, _, cx| {
                handle.update(cx, |state, cx| state.forget_deployment(&id, cx))
            })
        };
        tagged(
            deployment_selector(&record.id),
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .py_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(theme.label_width())
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(title)
                        .child(Caption::new(format::list(cx, &details))),
                )
                .child(tagged(redeploy_selector(&record.id), again))
                .child(tagged(forget_deployment_selector(&record.id), forget)),
        )
    });
    tagged(
        DEPLOYMENTS,
        Card::new().child(List::new().divided().children(rows)),
    )
    .w_full()
    .into_any_element()
}
