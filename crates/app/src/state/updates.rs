//! Updates: whether a newer release is published, and installing it in place when the user
//! asks.
//!
//! The automatic check runs at every start, from the usage poller (so never before the first
//! frame or the setup checks), then once a day on the backend clock while the app runs. Nothing
//! about it is saved, so a restart checks again. A check that gets no answer is tried again
//! hourly until one does. A failed automatic check is only shown in Settings → About; a manual
//! one also says so in a toast. A found release is announced once per version (a banner the
//! user can dismiss for that version) and stays marked in the sidebar and in About until this
//! build is at least that version.
//!
//! Installing is one action, "Download and restart", confirmed once with what a restart
//! affects. The backend checks the download before anything changes. Once the new version is in
//! place the app restarts as soon as no call that must finish is running, through the quit path,
//! which also gives a sign-in's credential back. A release whose download failed its check is
//! remembered, and the app never points at its files again. A copy that cannot replace itself
//! says why and offers the download page instead.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use colab_playground_core::{
    Error, ErrorKind, Result,
    update::{self, Release, UpdateStatus, Version},
    updater::{Installed, SelfUpdate, UpdateProgress},
};
use ely_gpui_component::{feedback::Toast, primitives::Severity};
use gpui::Context;

use super::{AppState, Op, Overlay};
use crate::{
    errors,
    i18n::{keys, tr},
    settings::OnByDefault,
};

/// How often the automatic check asks GitHub again while the app runs.
pub const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 3600);
// Offline at start-up still gets a check once the network is back, without asking GitHub at
// every POLL_CHECK meanwhile.
const UPDATE_RETRY_INTERVAL: Duration = Duration::from_secs(3600);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum UpdateCheck {
    /// Not checked in this run.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Failed(Error),
}

/// The update dialog: from the confirmation to the restart.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdatePrompt {
    pub version: String,
    pub phase: UpdatePhase,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdatePhase {
    /// What happens and what a restart affects, before anything starts.
    Confirm,
    /// Downloading, checking or installing (the latest progress).
    Working(UpdateProgress),
    /// The new version is in place; the app restarts once no call that must finish runs.
    Restarting,
    /// Nothing changed (or a failed step was undone).
    Failed(Error),
    /// The new version is in place but could not be started.
    RestartFailed(Error),
}

pub(super) fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn is_newer(version: &str) -> bool {
    match (
        Version::parse(version),
        Version::parse(update::CURRENT_VERSION),
    ) {
        (Some(found), Some(current)) => found > current,
        _ => false,
    }
}

impl AppState {
    pub fn update_check(&self) -> &UpdateCheck {
        &self.update
    }

    /// The newer release to point at (found now or by an earlier check), while this build is
    /// older than it: `(version, page)`.
    pub fn update_available(&self) -> Option<(String, String)> {
        if let UpdateCheck::Available(release) = &self.update {
            return Some((release.version.clone(), release.page.clone()));
        }
        let version = self.settings.update_latest.clone()?;
        if !is_newer(&version) {
            return None;
        }
        let page = update::this_app_release_page(&version)?;
        Some((version, page))
    }

    /// The release to install: the one found now, or the one an earlier check saved.
    fn available_release(&self) -> Option<Release> {
        if let UpdateCheck::Available(release) = &self.update {
            return Some(release.clone());
        }
        let (version, page) = self.update_available()?;
        Some(Release {
            tag: format!("v{version}"),
            version,
            page,
        })
    }

    /// The banner's release. Shown once the app knows which action to offer, and never for a
    /// release whose download failed its check.
    pub fn update_notice(&self) -> Option<(String, String)> {
        self.self_update.as_ref()?;
        self.update_available().filter(|(version, _)| {
            self.settings.update_dismissed.as_deref() != Some(version)
                && !self.update_unverified(version)
        })
    }

    /// Whether `version`'s download failed its signature or checksum check.
    pub fn update_unverified(&self, version: &str) -> bool {
        self.settings.update_unverified.as_deref() == Some(version)
    }

    /// Whether this copy can replace itself; `None` until the backend has said.
    pub fn self_update(&self) -> Option<&SelfUpdate> {
        self.self_update.as_ref()
    }

    pub fn can_self_update(&self) -> bool {
        matches!(self.self_update, Some(SelfUpdate::Supported))
    }

    pub fn checks_updates(&self) -> bool {
        self.settings.check_updates.0
    }

    pub fn set_check_updates(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.check_updates = OnByDefault(on);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Hides the banner for `version`; the sidebar and About keep pointing at it.
    pub fn dismiss_update(&mut self, version: String, cx: &mut Context<Self>) {
        self.settings.update_dismissed = Some(version);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Opens the release page in the browser, except for a release whose download failed its
    /// check: downloading it by hand would skip the check.
    pub fn open_update_page(&self, cx: &mut Context<Self>) {
        if let Some((version, page)) = self.update_available()
            && !self.update_unverified(&version)
        {
            cx.open_url(&page);
        }
    }

    pub fn check_updates_now(&mut self, cx: &mut Context<Self>) {
        self.run_update_check(true, cx);
    }

    /// At start-up: whether this copy can replace itself, and deleting what an earlier update
    /// left (on Windows the replaced program, once it has exited).
    pub(crate) fn prepare_updates(&mut self, cx: &mut Context<Self>) {
        self.run_backend(
            Op::UpdateCleanup,
            cx,
            |backend, _| {
                backend.remove_update_leftovers();
                Ok(())
            },
            |_, _, _| {},
        );
        self.refresh_self_update(cx);
    }

    fn refresh_self_update(&mut self, cx: &mut Context<Self>) {
        self.run_backend(
            Op::UpdateSupport,
            cx,
            |backend, _| Ok(backend.self_update()),
            |state, result, cx| {
                if let Ok(support) = result {
                    if let SelfUpdate::Unsupported(reason) = &support {
                        log::info!("updates: this copy cannot replace itself: {reason:?}");
                    }
                    state.self_update = Some(support);
                    cx.notify();
                }
            },
        );
    }

    /// One poller wake: checks when automatic checks are on, no check has got an answer in
    /// this run or the last one is a day old, and an hour has passed since the last try (on the
    /// backend clock).
    pub(crate) fn maybe_check_updates(&mut self, cx: &mut Context<Self>) {
        if self.checks_updates() && !self.ops.is_running(Op::UpdateCheck) {
            self.run_update_check(false, cx);
        }
    }

    fn run_update_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.ops.is_running(Op::UpdateCheck) {
            return;
        }
        if !manual {
            match &self.backend {
                Ok(backend) => {
                    let now = secs(backend.now());
                    let due_answer = self
                        .update_answered_at
                        .is_none_or(|last| now >= last + UPDATE_INTERVAL.as_secs());
                    let due_retry = self
                        .update_attempted_at
                        .is_none_or(|last| now >= last + UPDATE_RETRY_INTERVAL.as_secs());
                    if !(due_answer && due_retry) {
                        return;
                    }
                }
                // No backend to check with and no clock to space the tries: its failure is
                // reported once, not at every poller wake.
                Err(_) if matches!(self.update, UpdateCheck::Failed(_)) => return,
                Err(_) => {}
            }
        }
        if manual {
            self.update = UpdateCheck::Checking;
        }
        self.run_backend(
            Op::UpdateCheck,
            cx,
            move |backend, cancel| {
                let now = secs(backend.now());
                Ok((backend.check_for_update(cancel), now))
            },
            move |state, result, cx| {
                let (outcome, now) = match result {
                    Ok(checked) => checked,
                    Err(error) => return state.update_failed(error, manual, cx),
                };
                state.update_attempted_at = Some(now);
                match outcome {
                    Ok(UpdateStatus::UpToDate) => {
                        state.update_answered_at = Some(now);
                        state.update = UpdateCheck::UpToDate;
                        if state
                            .settings
                            .update_latest
                            .as_deref()
                            .is_some_and(|v| !is_newer(v))
                        {
                            state.settings.update_latest = None;
                            state.persist_settings(cx);
                        }
                    }
                    Ok(UpdateStatus::Available(release)) => {
                        state.update_answered_at = Some(now);
                        state.settings.update_latest = Some(release.version.clone());
                        state.update = UpdateCheck::Available(release);
                        // Where the app is may have changed since start-up.
                        state.refresh_self_update(cx);
                        state.persist_settings(cx);
                    }
                    Err(error) => state.update_failed(error, manual, cx),
                }
                cx.notify();
            },
        );
        if manual {
            cx.notify();
        }
    }

    fn update_failed(&mut self, error: Error, manual: bool, cx: &mut Context<Self>) {
        log::warn!("updates: the check failed: {error}");
        if manual {
            let message = errors::explain(&error, false, cx).message;
            let toast = Toast::new(tr(cx, keys::UPDATE_FAILED_TITLE))
                .body(message)
                .severity(Severity::Warning);
            self.toast(toast, cx);
        }
        self.update = UpdateCheck::Failed(error);
        cx.notify();
    }

    /// The update dialog, if it is open.
    pub fn update_prompt(&self) -> Option<&UpdatePrompt> {
        match &self.overlay {
            Some(Overlay::Update(prompt)) => Some(prompt),
            _ => None,
        }
    }

    fn update_phase_mut(&mut self) -> Option<&mut UpdatePhase> {
        match &mut self.overlay {
            Some(Overlay::Update(prompt)) => Some(&mut prompt.phase),
            _ => None,
        }
    }

    /// "Download and restart": opens the confirmation, which says what a restart affects.
    pub fn begin_update(&mut self, cx: &mut Context<Self>) {
        let Some(release) = self.available_release() else {
            return;
        };
        // A cancelled download still winding down would close the new dialog when it returns
        // (usually at once).
        if !self.can_self_update() || self.overlay.is_some() || self.is_busy(Op::UpdateInstall) {
            return;
        }
        self.overlay = Some(Overlay::Update(UpdatePrompt {
            version: release.version,
            phase: UpdatePhase::Confirm,
        }));
        cx.notify();
    }

    /// Why installing must wait, if it must: a restart would cut off a runtime being started,
    /// a deployment, a sign-in or the CLI's installation.
    pub fn update_block(&self) -> Option<&'static str> {
        if self.ops.is_running(Op::Provision) {
            Some(keys::UPDATE_BLOCKED_PROVISION)
        } else if self.ops.is_running(Op::PluginRun) {
            Some(keys::UPDATE_BLOCKED_RUN)
        } else if self.login_flow.is_some()
            || self.ops.is_running(Op::Login)
            || self.ops.is_running(Op::LoginRollback)
        {
            Some(keys::UPDATE_BLOCKED_SIGN_IN)
        } else if self.ops.is_running(Op::Install) {
            Some(keys::UPDATE_BLOCKED_INSTALL)
        } else {
            None
        }
    }

    /// The confirmed "Download and restart" (also "Try again" after a failure).
    pub fn confirm_update(&mut self, cx: &mut Context<Self>) {
        if self.update_block().is_some() || self.ops.is_running(Op::UpdateInstall) {
            return;
        }
        let Some(release) = self.available_release() else {
            return;
        };
        let Some(Overlay::Update(prompt)) = &mut self.overlay else {
            return;
        };
        if prompt.version != release.version {
            // A check found another release while the dialog was open: it is installed only
            // once confirmed by name.
            prompt.version = release.version;
            prompt.phase = UpdatePhase::Confirm;
            cx.notify();
            return;
        }
        prompt.phase = UpdatePhase::Working(UpdateProgress::Preparing);
        log::info!("updates: installing {}", release.version);
        let version = release.version.clone();
        self.run_backend_streaming(
            Op::UpdateInstall,
            cx,
            move |backend, events, cancel| backend.install_update(&release, events, cancel),
            |state, progress, cx| state.apply_update_progress(progress, cx),
            move |state, result, cx| state.update_installed(version, result, cx),
        );
        cx.notify();
    }

    /// A progress report from the install call: the dialog shows it while it works.
    pub fn apply_update_progress(&mut self, progress: UpdateProgress, cx: &mut Context<Self>) {
        if let Some(phase) = self.update_phase_mut()
            && matches!(phase, UpdatePhase::Working(_))
        {
            *phase = UpdatePhase::Working(progress);
            cx.notify();
        }
    }

    fn update_installed(
        &mut self,
        version: String,
        result: Result<Installed>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = &result
            && error.is(ErrorKind::UpdateUnverified)
        {
            // Remembered even when a quit prompt has taken the dialog's place meanwhile.
            self.settings.update_unverified = Some(version.clone());
            self.persist_settings(cx);
        }
        if self.update_prompt().is_none() {
            // The dialog gave way to quitting while the download was cancelled.
            if let Ok(installed) = &result {
                log::info!(
                    "updates: {} is installed; it runs from the next start",
                    installed.version
                );
            }
            return;
        };
        match result {
            Ok(installed) if installed.relaunch.is_none() => {
                // Demo data: nothing was installed, so nothing restarts.
                self.overlay = None;
                let toast = Toast::new(tr(cx, keys::UPDATE_SIMULATED_TITLE))
                    .body(tr(cx, keys::UPDATE_SIMULATED_BODY))
                    .severity(Severity::Info);
                self.toast(toast, cx);
            }
            Ok(installed) => {
                log::info!("updates: {} is installed; restarting", installed.version);
                self.installed = Some(installed);
                if let Some(phase) = self.update_phase_mut() {
                    *phase = UpdatePhase::Restarting;
                }
                self.settle_pending_restart(cx);
            }
            Err(error) if error.is(ErrorKind::Cancelled) => {
                log::info!("updates: cancelled; nothing changed");
                self.overlay = None;
            }
            Err(error) => {
                log::warn!("updates: installing {version} failed: {error}");
                if let Some(phase) = self.update_phase_mut() {
                    *phase = UpdatePhase::Failed(error);
                }
            }
        }
        cx.notify();
    }

    /// Whether Cancel still applies: until the new version starts being put in place. (The
    /// dialog is `Working` exactly while the install call runs; its result moves it on.)
    pub fn update_cancellable(&self) -> bool {
        matches!(
            self.update_prompt().map(|prompt| &prompt.phase),
            Some(UpdatePhase::Working(progress)) if *progress != UpdateProgress::Installing
        )
    }

    /// Whether the app is past the point where the update can be stopped: putting the new
    /// version in place, or restarting.
    pub fn update_committed(&self) -> bool {
        matches!(
            self.update_prompt().map(|prompt| &prompt.phase),
            Some(UpdatePhase::Working(UpdateProgress::Installing) | UpdatePhase::Restarting)
        )
    }

    pub fn cancel_update(&mut self, cx: &mut Context<Self>) {
        if self.update_cancellable() {
            self.ops.cancel(Op::UpdateInstall);
            cx.notify();
        }
    }

    /// The dialog's close (Escape, the scrim, its close button): closes it before anything
    /// starts or after a failure, cancels a download, and does nothing once installing.
    pub(crate) fn close_update_dialog(&mut self, cx: &mut Context<Self>) {
        match self.update_prompt().map(|prompt| &prompt.phase) {
            Some(UpdatePhase::Working(_)) if self.update_cancellable() => self.cancel_update(cx),
            Some(UpdatePhase::Working(_) | UpdatePhase::Restarting) => {}
            Some(_) => {
                self.overlay = None;
                cx.notify();
            }
            None => {}
        }
    }

    /// Restarts into the installed version once no call that must finish is running. Called
    /// after every backend call returns and on every poller wake.
    pub(crate) fn settle_pending_restart(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.update_prompt().map(|prompt| &prompt.phase),
            Some(UpdatePhase::Restarting)
        ) {
            return;
        }
        let Some(installed) = self.installed.clone() else {
            return;
        };
        if self.ops.delaying_restart() || self.ops.is_running(Op::UpdateRestart) {
            return;
        }
        self.run_backend(
            Op::UpdateRestart,
            cx,
            move |backend, _| backend.relaunch(&installed),
            |state, result, cx| match result {
                Ok(()) => {
                    log::info!("updates: started the new version; quitting");
                    state.installed = None;
                    state.quit_for_restart(cx);
                }
                Err(error) => {
                    log::error!("updates: starting the new version failed: {error}");
                    state.installed = None;
                    if let Some(phase) = state.update_phase_mut() {
                        *phase = UpdatePhase::RestartFailed(error);
                    }
                    cx.notify();
                }
            },
        );
    }

    /// Whether the dialog is waiting to restart, with a call that must finish first.
    pub fn restart_waiting(&self) -> bool {
        matches!(
            self.update_prompt().map(|prompt| &prompt.phase),
            Some(UpdatePhase::Restarting)
        ) && self.ops.delaying_restart()
    }
}
