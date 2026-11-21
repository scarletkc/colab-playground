//! The deployments list (see [`crate::deployments`]): recorded when a run starts, used again
//! from the plugins page, saved in the background like the settings.

use colab_playground_core::{backend::PluginRunSpec, plugins::PluginInfo};
use ely_gpui_component::{feedback::Toast, primitives::Severity};
use gpui::{AppContext, Context};

use super::AppState;
use crate::{
    deployments::{Deployment, record},
    i18n::{keys, tr},
};

impl AppState {
    /// Saved deployments, the most recently used first.
    pub fn deployments(&self) -> &[Deployment] {
        &self.deployments
    }

    pub(super) fn record_deployment(
        &mut self,
        spec: &PluginRunSpec,
        info: &PluginInfo,
        cx: &mut Context<Self>,
    ) {
        let now = self
            .backend_now()
            .unwrap_or_else(std::time::SystemTime::now);
        record(
            &mut self.deployments,
            Deployment::of(spec, &info.manifest, now),
        );
        self.persist_deployments(cx);
    }

    /// Removes a saved deployment from the list (nothing on a runtime changes).
    pub fn forget_deployment(&mut self, id: &str, cx: &mut Context<Self>) {
        let before = self.deployments.len();
        self.deployments.retain(|d| d.id != id);
        if self.deployments.len() != before {
            self.persist_deployments(cx);
            cx.notify();
        }
    }

    fn persist_deployments(&mut self, cx: &mut Context<Self>) {
        let store = self.deployment_store.clone();
        let list = self.deployments.clone();
        let previous = self.deployments_save.take();
        self.deployments_save = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let saved = cx.background_spawn(async move { store.save(&list) }).await;
            if let Err(error) = saved {
                log::error!("deployments: save failed: {error}");
                let detail = error.to_string();
                this.update(cx, |state, cx| {
                    let toast = Toast::new(tr(cx, keys::TOAST_SAVE_FAILED_TITLE))
                        .body(detail)
                        .severity(Severity::Danger);
                    state.toast(toast, cx);
                })
                .ok();
            }
        }));
    }
}
