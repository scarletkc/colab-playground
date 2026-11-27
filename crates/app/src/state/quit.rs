//! Quitting while runtimes still bill. Quit (⌘Q / the app menu, and the window close on
//! Windows and Linux) calls [`AppState::request_quit`]; when something is billing, or the app
//! cannot tell, it opens the quit prompt instead of quitting. On macOS the traffic lights
//! close the window and leave the process running.
//!
//! "Leave running" is only described as watched by a watchdog for runtimes that have one;
//! runtimes opened elsewhere, or whose watchdog on the runtime is not armed (it failed, was
//! cancelled or could not be restarted), have no automatic stop and the prompt says so,
//! listing every runtime by name and account.

use colab_playground_core::{Error, ErrorKind, backend::RuntimeRef};
use gpui::Context;

use super::{AppState, Load, Op, Overlay};

/// What is still running or starting when the window is closed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Billing {
    /// Runtimes whose watchdog will stop them (deadline set, not cancelled).
    pub watched: Vec<RuntimeRef>,
    /// Runtimes with no automatic stop.
    pub unwatched: Vec<RuntimeRef>,
    pub provisioning: bool,
    pub running_plugin: bool,
    /// The runtime list, or an account in it, could not be loaded, so there may be more.
    pub unknown: bool,
    /// The app is still finding out what runs (checking the CLI, listing accounts or
    /// runtimes).
    pub checking: bool,
}

impl Billing {
    pub fn any(&self) -> bool {
        self.known() || self.unknown || self.checking
    }

    /// Whether something is known to bill or to be starting.
    pub fn known(&self) -> bool {
        !self.watched.is_empty()
            || !self.unwatched.is_empty()
            || self.provisioning
            || self.running_plugin
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuitPrompt {
    pub billing: Billing,
    /// Stops that failed during "Stop all".
    pub failures: Vec<(String, Error)>,
    /// Waiting on the check (opened before the app knew what runs, or a listing started since):
    /// once it is done, the app quits if nothing bills (the user already asked to close).
    pub waiting: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuitState {
    #[default]
    Ask,
    /// The user decided; the window may close.
    Approved,
}

impl AppState {
    pub fn billing(&self) -> Billing {
        let mut billing = Billing {
            provisioning: self.ops.is_running(Op::Provision),
            running_plugin: self.ops.is_running(Op::PluginRun),
            ..Billing::default()
        };
        match &self.sessions.runtimes {
            Load::Ready(view) => {
                for runtime in &view.runtimes {
                    // Stops on its own only if the watchdog on the runtime is armed with a deadline.
                    let watched = runtime
                        .tracked
                        .as_ref()
                        .and_then(|t| t.watchdog.as_ref())
                        .is_some_and(|w| w.enforced_deadline().is_some());
                    if watched {
                        billing.watched.push(runtime.id.clone());
                    } else {
                        billing.unwatched.push(runtime.id.clone());
                    }
                }
                billing.unknown = !view.errors.is_empty();
            }
            Load::Failed(_) => billing.unknown = self.has_saved_accounts(),
            Load::Idle | Load::Loading => billing.checking = self.may_list_runtimes(),
        }
        billing
    }

    /// Whether runtimes may still turn up while the list is not loaded: the CLI or the
    /// accounts are still being checked, or there are accounts to list.
    fn may_list_runtimes(&self) -> bool {
        match (&self.probe, &self.accounts) {
            (Load::Idle | Load::Loading, _) => true,
            _ if !self.cli_installed() => false,
            (_, Load::Idle | Load::Loading) => true,
            _ => self.has_saved_accounts(),
        }
    }

    /// Settles a quit prompt waiting on the check once it is done: nothing billing quits;
    /// otherwise the prompt stays and shows what bills, or that the app could not check.
    /// Called after every backend call returns.
    pub(crate) fn settle_waiting_quit(&mut self, cx: &mut Context<Self>) {
        if !matches!(&self.overlay, Some(Overlay::Quit(prompt)) if prompt.waiting) {
            return;
        }
        let billing = self.billing();
        if billing.checking || self.ops.is_running(Op::Runtimes) {
            return;
        }
        if !billing.any() {
            self.approve_quit(cx);
            return;
        }
        if let Some(Overlay::Quit(prompt)) = &mut self.overlay {
            prompt.waiting = false;
        }
        cx.notify();
    }

    /// Quit: with nothing billing the app quits, otherwise the quit prompt opens.
    /// The window itself never closes on request; quitting closes it (see `approve_quit`).
    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.quit == QuitState::Approved {
            // Already quitting.
            return;
        }
        // A download in progress is simply abandoned: nothing was changed yet. Once the new
        // version is being put in place, or the app is about to restart into it, the quit
        // waits: stopping halfway could leave no program where the app was. The install call
        // can pass that point before the dialog shows it, which the cancel reports.
        if self.update_committed() || !self.ops.cancel(Op::UpdateInstall) {
            return;
        }
        if matches!(self.overlay, Some(Overlay::Quit(_))) {
            // Already asking (maybe mid "Stop all"): keep that prompt as it is.
            return;
        }
        if !matches!(self.sessions.runtimes, Load::Ready(_)) {
            // The prompt names every runtime: list them now if they are not known yet (or the
            // last try failed), unless a listing is already on its way.
            self.poll_runtimes(cx);
        }
        let billing = self.billing();
        // The list shown was read before the listing under way began, so it can lack a runtime
        // that came up since (a start that just finished): only that listing can say that
        // nothing bills.
        let listing = self.ops.is_running(Op::Runtimes);
        if !billing.any() && !listing {
            self.approve_quit(cx);
            return;
        }
        self.close_dialog(cx);
        self.drop_install_preview(cx);
        self.overlay = Some(Overlay::Quit(QuitPrompt {
            waiting: billing.checking || listing,
            billing,
            failures: Vec::new(),
        }));
        cx.notify();
    }

    /// A listing is starting: an open quit prompt waits for it, so that it quits if the
    /// listing finds nothing billing (the user already asked to close). Not during "Stop all",
    /// whose own result decides.
    pub(crate) fn quit_waits_for_listing(&mut self) {
        let stopping = self.is_stopping_all();
        if let Some(Overlay::Quit(prompt)) = &mut self.overlay
            && !stopping
        {
            prompt.waiting = true;
        }
    }

    pub fn quit_approved(&self) -> bool {
        self.quit == QuitState::Approved
    }

    pub fn quit_leaving_running(&mut self, cx: &mut Context<Self>) {
        self.approve_quit(cx);
    }

    /// Quits so the version an update installed can take over. The user confirmed what that
    /// affects in the update dialog, so the quit prompt is not asked again.
    pub(super) fn quit_for_restart(&mut self, cx: &mut Context<Self>) {
        self.approve_quit(cx);
    }

    /// The app quits (one window: quitting closes it). Quitting rather than also letting the
    /// close remove the window keeps the platform from handling a window that is already gone
    /// (a drawn Close is followed by the system's own close message).
    fn approve_quit(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
        self.quit = QuitState::Approved;
        cx.notify();
        cx.quit();
    }

    /// Stops everything, then quits. In order: cancel provisioning and plugin runs and wait
    /// until they have returned (they release what they allocated); then list afresh and stop
    /// every runtime (one already gone counts as stopped); then list once more and quit only
    /// if nothing is left. Anything left, or any account that could not be listed, keeps the
    /// prompt open and says which.
    pub fn quit_stopping_all(&mut self, cx: &mut Context<Self>) {
        if self.stop_all_pending {
            return;
        }
        self.stop_all_pending = true;
        self.ops.cancel(Op::Provision);
        self.ops.cancel(Op::PluginRun);
        if let Some(Overlay::Quit(prompt)) = &mut self.overlay {
            prompt.failures.clear();
        }
        self.continue_stop_all(cx);
        cx.notify();
    }

    /// Runs the stopping part of "Stop all" once no cancelled provision or plugin run is still
    /// unwinding. Called again when each of them returns.
    pub(crate) fn continue_stop_all(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.overlay, Some(Overlay::Quit(_))) {
            // The prompt was dismissed while cancelled work unwound: stop nothing.
            self.stop_all_pending = false;
            return;
        }
        if self.stop_all_pending
            && !self.ops.is_running(Op::StopAll)
            && !self.ops.is_running(Op::Provision)
            && !self.ops.is_running(Op::PluginRun)
        {
            self.stop_everything(cx);
        }
    }

    fn stop_everything(&mut self, cx: &mut Context<Self>) {
        self.run_backend(
            Op::StopAll,
            cx,
            move |backend, cancel| {
                let view = backend.runtimes(cancel)?;
                let mut failures: Vec<(Failed, Error)> = Vec::new();
                for runtime in &view.runtimes {
                    match backend.stop_session(&runtime.id, Default::default(), cancel) {
                        Ok(()) => {}
                        // Gone already (stopped by its watchdog, a cancelled run, elsewhere).
                        Err(error) if error.is(ErrorKind::SessionGone) => {}
                        Err(error) => {
                            failures.push((Failed::Runtime(super::runtime_label(runtime)), error))
                        }
                    }
                }
                let after = backend.runtimes(cancel)?;
                // The final listing decides: a stop that "failed" on a runtime that is gone now
                // (already stopped by its watchdog, a cancelled run or elsewhere) is not a failure.
                let still: Vec<String> = after.runtimes.iter().map(super::runtime_label).collect();
                failures.retain(
                    |(what, _)| matches!(what, Failed::Runtime(label) if still.contains(label)),
                );
                for failure in after.errors {
                    failures.push((Failed::Account(failure.account), failure.error));
                }
                for runtime in after.runtimes {
                    let label = super::runtime_label(&runtime);
                    if !failures
                        .iter()
                        .any(|(what, _)| matches!(what, Failed::Runtime(l) if *l == label))
                    {
                        failures.push((
                            Failed::Runtime(label),
                            Error::new(ErrorKind::Other).detail("still listed after stopping"),
                        ));
                    }
                }
                Ok(failures)
            },
            |state, result, cx| {
                state.stop_all_pending = false;
                let failures: Vec<(String, Error)> = match result {
                    Ok(failures) => failures
                        .into_iter()
                        .map(|(what, error)| {
                            let label = match what {
                                Failed::Runtime(label) => label,
                                Failed::Account(account) => state
                                    .email_of(&account)
                                    .unwrap_or_else(|| format!("{account:?}")),
                            };
                            (label, error)
                        })
                        .collect(),
                    Err(error) => vec![(String::new(), error)],
                };
                let asking = matches!(state.overlay, Some(Overlay::Quit(_)));
                if failures.is_empty() && asking {
                    state.approve_quit(cx);
                    return;
                }
                if let Some(Overlay::Quit(prompt)) = &mut state.overlay {
                    prompt.failures = failures;
                }
                state.load_runtimes(cx);
                cx.notify();
            },
        );
    }

    /// Whether "Stop all" is under way (waiting for cancelled work, or stopping).
    pub fn is_stopping_all(&self) -> bool {
        self.stop_all_pending || self.ops.is_running(Op::StopAll)
    }
}

enum Failed {
    Runtime(String),
    /// An account whose runtimes could not be listed.
    Account(colab_playground_core::accounts::AccountId),
}
