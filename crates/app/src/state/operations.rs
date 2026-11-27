//! What `colab-playground ctl` may change: starting and stopping runtimes, and the operations
//! that follow them.
//!
//! A command that changes something changes nothing without `yes`: it answers with what it
//! would do, from what the window's own confirmation shows, or is refused with the code the
//! change would get. A start goes through the New session dialog's provision and a stop through
//! the Sessions page's, so the window shows them, the quit prompt knows about them and the same
//! limits hold. No argument loosens a cost guard; those stay with the person at the window. A
//! change that would get in the way of what the window is doing (closing, updating, changing
//! accounts, a start under way, the New session form being filled in) is refused, not queued.
//!
//! Each change gets an operation id. The record keeps the outcome itself, because the window
//! drops or replaces the dialog that showed it. An id starts with a part drawn at random once
//! per run, so an id from before a restart is unknown here instead of naming another operation.
//! Nothing ends an operation when the client that began it goes away: cancelling a start gives
//! the runtime back, so only its own command does that.

use std::{collections::HashMap, time::SystemTime};

use colab_playground_core::{
    Error, ErrorKind,
    accounts::AccountId,
    backend::{Runtime, RuntimeRef, accelerator_prefix, generate_session_name},
    clock::serde_secs,
    guard::GuardPolicy,
    provision::{ProvisionEvent, ProvisionOutcome, ProvisionSpec},
    sessions::Accelerator,
    usage::published_rate,
};
use ely_gpui_component::{feedback::Toast, primitives::Severity};
use gpui::Context;
use serde::Serialize;

use super::{
    AccountDialog, AppState, Load, Op, Overlay, ProvisionPhase, SignInStatus, UpdatePhase,
    control::RuntimeAnswer, runtime_label,
};
use crate::{
    control::{CancelArgs, Code, OperationState, StartArgs, StopArgs},
    i18n::{keys, tr_args},
};

/// Operations that ended and can still be read; older ones are forgotten.
pub const ENDED_OPERATIONS_KEPT: usize = 100;

/// Why a command is not done, with a message for people.
pub(super) struct Refusal {
    pub(super) code: Code,
    pub(super) message: String,
}

impl Refusal {
    fn new(code: Code, message: impl Into<String>) -> Self {
        Refusal {
            code,
            message: message.into(),
        }
    }
}

pub(crate) struct Operations {
    /// The random part of this run's ids.
    run: String,
    count: u64,
    /// Oldest first.
    list: Vec<Operation>,
    /// The name a preview of a start without a name showed, by hardware: the start after it
    /// uses that name, as the dialog starts under the name it shows.
    suggested: HashMap<Accelerator, String>,
}

impl Default for Operations {
    fn default() -> Self {
        // The process id stands in should the OS have no randomness to give: it too differs
        // from the run before.
        let run = getrandom::u32().unwrap_or_else(|_| std::process::id());
        Operations {
            run: format!("{run:08x}"),
            count: 0,
            list: Vec::new(),
            suggested: HashMap::new(),
        }
    }
}

impl Operations {
    fn begin(&mut self, operation: impl FnOnce(String) -> Operation) -> String {
        self.count += 1;
        let id = format!("{}-{}", self.run, self.count);
        self.list.push(operation(id.clone()));
        id
    }

    fn find(&self, id: &str) -> Option<&Operation> {
        self.list.iter().find(|operation| operation.id == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Operation> {
        self.list.iter_mut().find(|operation| operation.id == id)
    }

    /// Forgets the oldest operations that ended beyond [`ENDED_OPERATIONS_KEPT`].
    fn forget_old(&mut self) {
        let ended = self
            .list
            .iter()
            .filter(|operation| operation.state != OperationState::Running)
            .count();
        let mut excess = ended.saturating_sub(ENDED_OPERATIONS_KEPT);
        self.list.retain(|operation| {
            let forget = excess > 0 && operation.state != OperationState::Running;
            excess -= usize::from(forget);
            !forget
        });
    }
}

struct Operation {
    id: String,
    account: AccountId,
    email: Option<String>,
    /// Backend clock.
    started_at: Option<SystemTime>,
    ended_at: Option<SystemTime>,
    state: OperationState,
    error: Option<Error>,
    kind: Kind,
}

enum Kind {
    Start {
        name: String,
        accelerator: Accelerator,
        high_mem: bool,
        progress: Vec<ProvisionEvent>,
        outcome: Option<ProvisionOutcome>,
    },
    Stop {
        target: RuntimeRef,
        label: String,
    },
}

#[derive(Serialize)]
struct Answer<'a> {
    #[serde(serialize_with = "serde_secs::option::serialize")]
    now: Option<SystemTime>,
    #[serde(flatten)]
    body: Body<'a>,
}

/// `{"preview": …}` for a command asked without `yes`, `{"operation": …}` for one under way.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Body<'a> {
    Preview(Preview<'a>),
    Operation(OperationAnswer<'a>),
}

#[derive(Serialize)]
#[serde(untagged)]
enum Preview<'a> {
    Start(StartPreview<'a>),
    Stop(StopPreview<'a>),
    Cancel { operation: OperationAnswer<'a> },
}

#[derive(Serialize)]
struct AccountAnswer<'a> {
    id: &'a AccountId,
    email: Option<&'a str>,
}

/// What the New session dialog shows before Start.
#[derive(Serialize)]
struct StartPreview<'a> {
    account: AccountAnswer<'a>,
    name: &'a str,
    accelerator: Accelerator,
    high_mem: bool,
    min_vram_mib: Option<u32>,
    attempts: u32,
    /// An estimate (`≈` in the window).
    estimated_rate_cu_per_hour: f64,
    rate_source: RateSource,
    #[serde(serialize_with = "serde_secs::option::serialize")]
    rate_measured_at: Option<SystemTime>,
    /// When it stops on its own.
    policy: &'a GuardPolicy,
    session_limit: LimitAnswer,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RateSource {
    /// What a runtime on this hardware burned when the app last measured one.
    Measured,
    /// What other users published for this hardware.
    Published,
}

#[derive(Serialize)]
struct LimitAnswer {
    limit: u32,
    /// The runtimes that count against it; `null` while the runtimes are not listed.
    counted: Option<Vec<String>>,
    /// `null` while the runtimes are not listed (the start is checked again as it begins).
    allows: Option<bool>,
}

/// What the stop confirmation says.
#[derive(Serialize)]
struct StopPreview<'a> {
    runtime: RuntimeAnswer<'a>,
    /// What ends with it.
    ends: Vec<&'static str>,
}

#[derive(Serialize)]
struct OperationAnswer<'a> {
    id: &'a str,
    #[serde(flatten)]
    kind: KindAnswer<'a>,
    state: OperationState,
    /// A cancel was asked and the start is still winding down.
    cancelling: bool,
    #[serde(serialize_with = "serde_secs::option::serialize")]
    started_at: Option<SystemTime>,
    #[serde(serialize_with = "serde_secs::option::serialize")]
    ended_at: Option<SystemTime>,
    account: AccountAnswer<'a>,
    error: Option<&'a Error>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum KindAnswer<'a> {
    Start {
        runtime: NewRuntime<'a>,
        progress: &'a [ProvisionEvent],
        outcome: Option<&'a ProvisionOutcome>,
    },
    Stop {
        runtime: StoppedRuntime<'a>,
    },
}

#[derive(Serialize)]
struct NewRuntime<'a> {
    name: &'a str,
    accelerator: Accelerator,
    high_mem: bool,
}

#[derive(Serialize)]
struct StoppedRuntime<'a> {
    id: &'a RuntimeRef,
    label: &'a str,
}

fn unknown_operation(id: &str) -> Refusal {
    Refusal::new(
        Code::UnknownOperation,
        format!(
            "this run of the app has no operation {id:?}: an id lasts until the app quits, and \
             only the {ENDED_OPERATIONS_KEPT} most recent that ended are kept. That says nothing \
             about how the operation went; `sessions` shows what runs"
        ),
    )
}

impl AppState {
    /// `start`: a runtime on the active account under `args`' hardware and name, everything
    /// else from the Defaults as the dialog fills them in.
    pub(super) fn control_start(
        &mut self,
        args: StartArgs,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, Refusal> {
        let (account, email) = self.start_refusal()?;
        let defaults = self.defaults();
        let accelerator = args.accelerator.unwrap_or(defaults.accelerator);
        let name = match args.name {
            Some(name) => name,
            None => self.suggested_name(accelerator),
        };
        let mut spec = self.defaults().provision_spec(name);
        spec.accelerator = accelerator;
        // As the form does: a GPU memory minimum belongs to GPUs only.
        if !accelerator.is_gpu() {
            spec.min_vram_mib = None;
        }
        spec.validate()
            .map_err(|error| Refusal::new(Code::InvalidRequest, error.to_string()))?;
        if !args.yes {
            let preview = self.start_preview(&account, email.as_deref(), &spec);
            return Ok(self.reply(Body::Preview(Preview::Start(preview))));
        }
        self.operations.suggested.remove(&spec.accelerator);
        let started_at = self.backend_now();
        let name = spec.name.clone();
        let id = self.operations.begin(|id| Operation {
            id,
            account,
            email,
            started_at,
            ended_at: None,
            state: OperationState::Running,
            error: None,
            kind: Kind::Start {
                name: spec.name.clone(),
                accelerator: spec.accelerator,
                high_mem: spec.high_mem && spec.accelerator.supports_high_mem(),
                progress: Vec::new(),
                outcome: None,
            },
        });
        log::info!("control: starting {name} as operation {id}");
        self.start_provision_for(spec, id.clone(), cx);
        self.toast_from_command_line(keys::CTL_STARTING_TOAST, &name, cx);
        self.operation_reply(&id)
    }

    /// The active account and its email, or why a start cannot begin now.
    fn start_refusal(&self) -> Result<(AccountId, Option<String>), Refusal> {
        self.changes_refusal()?;
        // A start is tied to the active account, so the window opens neither of these while one
        // runs; one opened before the start could then switch or sign in under it.
        if matches!(
            self.dialog,
            Some(AccountDialog::Switch { .. } | AccountDialog::Add(_))
        ) {
            return Err(Refusal::new(
                Code::AccountChanging,
                "an account switch or sign-in is open in the window; ask again once it is done \
                 or closed",
            ));
        }
        if let Some(error) = self.backend_error() {
            return Err(Refusal::new(
                Code::NotReady,
                format!("the backend could not start: {error}"),
            ));
        }
        if self.ops.is_running(Op::Rebuild) {
            return Err(Refusal::new(
                Code::NotReady,
                "the app is setting up the Colab CLI again after a change in Settings",
            ));
        }
        let email = match self.sign_in_status() {
            SignInStatus::SignedIn { email } => email,
            SignInStatus::Checking => {
                return Err(Refusal::new(
                    Code::NotReady,
                    "the app is still checking the Colab CLI and the accounts",
                ));
            }
            SignInStatus::KeyringUnavailable(error) | SignInStatus::Failed(error) => {
                return Err(Refusal::new(
                    Code::NotReady,
                    format!("the accounts could not be loaded: {error}"),
                ));
            }
            SignInStatus::NeedsCli => {
                return Err(Refusal::new(
                    Code::NoActiveAccount,
                    "the Colab CLI is not set up yet",
                ));
            }
            SignInStatus::SignedOut => {
                return Err(Refusal::new(
                    Code::NoActiveAccount,
                    "no account is signed in",
                ));
            }
            SignInStatus::Unadopted { email } => {
                return Err(Refusal::new(
                    Code::NoActiveAccount,
                    format!(
                        "the Colab CLI is signed in to {}, which this app has not saved; save it \
                         in the window first",
                        email.as_deref().unwrap_or("an account")
                    ),
                ));
            }
        };
        let account = self
            .accounts
            .ready()
            .and_then(|view| view.active.clone())
            .ok_or_else(|| Refusal::new(Code::NoActiveAccount, "no account is active"))?;
        if self.is_provisioning() {
            let name = self
                .new_session
                .as_ref()
                .map_or("a runtime", |session| session.spec.name.as_str());
            return Err(Refusal::new(
                Code::AlreadyStarting,
                format!("{name} is being started already; wait for it, or cancel it, first"),
            ));
        }
        // The open dialog holds what someone entered: the form, or after a start that failed or
        // was cancelled, the values to try again with and why it ended. One that shows a
        // runtime it started holds nothing more to act on.
        let holds_a_form = matches!(self.overlay, Some(Overlay::NewSession))
            && self
                .new_session
                .as_ref()
                .is_some_and(|session| !matches!(session.phase, ProvisionPhase::Ready(_)));
        if holds_a_form {
            return Err(Refusal::new(
                Code::FormOpen,
                "the New session dialog is open in the window with what was entered there; it \
                 stays as it is until it is started or closed there",
            ));
        }
        if let Some(reached) = self.session_limit_reached() {
            return Err(Refusal::new(
                Code::SessionLimitReached,
                format!(
                    "this app starts at most {} runtimes per account, and {} has {}. It is this \
                     app's own limit, set in the window's Settings; stopping one makes room",
                    reached.limit,
                    reached.email.as_deref().unwrap_or("the active account"),
                    reached.counted.join(", ")
                ),
            ));
        }
        Ok((account, email))
    }

    /// Why nothing may start or stop now: the app is closing or updating (the quit prompt shows
    /// what billed when it opened and "Stop all" stops from a listing of its own, and a start
    /// would hold up a restart or be cut off by the quit), or an account change is under way
    /// (core would refuse the call with `busy` after a wait).
    fn changes_refusal(&self) -> Result<(), Refusal> {
        let updating = matches!(
            self.update_prompt().map(|prompt| &prompt.phase),
            Some(UpdatePhase::Working(_) | UpdatePhase::Restarting)
        );
        if matches!(self.overlay, Some(Overlay::Quit(_))) || self.is_stopping_all() || updating {
            return Err(Refusal::new(
                Code::Quitting,
                "the app is about to close, or to restart into an update; ask again in a moment",
            ));
        }
        let changing = [
            Op::Login,
            Op::LoginRollback,
            Op::Switch,
            Op::Adopt,
            Op::Remove,
        ]
        .into_iter()
        .any(|op| self.ops.is_running(op))
            || self.login_flow.is_some();
        if changing {
            return Err(Refusal::new(
                Code::AccountChanging,
                "an account is being added, switched or removed in the window; ask again once \
                 that is done",
            ));
        }
        Ok(())
    }

    /// A name for a start without one, with the hardware's prefix. A preview remembers it, so
    /// the start that follows gets the name the preview showed.
    fn suggested_name(&mut self, accelerator: Accelerator) -> String {
        let taken: Vec<String> = self
            .sessions
            .runtimes
            .ready()
            .map(|view| {
                view.runtimes
                    .iter()
                    .filter_map(|runtime| runtime.id.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(name) = self.operations.suggested.get(&accelerator)
            && !taken.contains(name)
        {
            return name.clone();
        }
        let name = generate_session_name(&accelerator_prefix(accelerator), &taken);
        self.operations.suggested.insert(accelerator, name.clone());
        name
    }

    fn start_preview<'a>(
        &'a self,
        account: &'a AccountId,
        email: Option<&'a str>,
        spec: &'a ProvisionSpec,
    ) -> StartPreview<'a> {
        // The figure the form shows, marked there as an estimate.
        let (rate, source, measured_at) = match self.measured_rate(spec) {
            Some((reading, _)) => (reading.cu_per_hour, RateSource::Measured, Some(reading.at)),
            None => (
                published_rate(spec.accelerator, spec.high_mem),
                RateSource::Published,
                None,
            ),
        };
        let limit = self.defaults().max_concurrent_sessions.max(1);
        // Counted as the dialog counts: named runtimes on the active account.
        let counted: Option<Vec<String>> = self.sessions.runtimes.ready().map(|view| {
            view.runtimes
                .iter()
                .filter(|runtime| runtime.on_active_account)
                .filter_map(|runtime| runtime.id.name.clone())
                .collect()
        });
        StartPreview {
            account: AccountAnswer { id: account, email },
            name: &spec.name,
            accelerator: spec.accelerator,
            high_mem: spec.high_mem && spec.accelerator.supports_high_mem(),
            min_vram_mib: spec.min_vram_mib,
            attempts: spec.attempts(),
            estimated_rate_cu_per_hour: rate,
            rate_source: source,
            rate_measured_at: measured_at,
            policy: &spec.guard,
            session_limit: LimitAnswer {
                limit,
                allows: counted
                    .as_ref()
                    .map(|counted| (counted.len() as u32) < limit),
                counted,
            },
        }
    }

    /// `stop`: a runtime on any saved account, named as `sessions` lists it.
    pub(super) fn control_stop(
        &mut self,
        args: StopArgs,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, Refusal> {
        self.changes_refusal()?;
        let runtime = self.find_runtime(&args.runtime, args.account.as_deref())?;
        if self.is_stopping(&runtime.id) {
            return Err(Refusal::new(
                Code::AlreadyStopping,
                format!("{} is being stopped already", runtime_label(runtime)),
            ));
        }
        if !args.yes {
            let mut ends = vec!["billing"];
            if !self.tunnels_for(&runtime.id).is_empty() {
                ends.push("port_forwards");
            }
            ends.push("files");
            let preview = StopPreview {
                runtime: self.runtime_answer(runtime),
                ends,
            };
            return Ok(self.reply(Body::Preview(Preview::Stop(preview))));
        }
        let (target, label) = (runtime.id.clone(), runtime_label(runtime));
        let email = runtime.account_email.clone();
        let started_at = self.backend_now();
        let id = self.operations.begin(|id| Operation {
            id,
            account: target.account.clone(),
            email,
            started_at,
            ended_at: None,
            state: OperationState::Running,
            error: None,
            kind: Kind::Stop {
                target: target.clone(),
                label: label.clone(),
            },
        });
        log::info!("control: stopping {label} as operation {id}");
        self.stop_runtime_for(target, Some(id.clone()), cx);
        self.toast_from_command_line(keys::CTL_STOPPING_TOAST, &label, cx);
        self.operation_reply(&id)
    }

    /// The one listed runtime with `wanted` as its session name or endpoint, on the account
    /// `account` names (by email or id) when given. Never a guess: none, or more than one, is
    /// refused.
    fn find_runtime(&self, wanted: &str, account: Option<&str>) -> Result<&Runtime, Refusal> {
        let view = match &self.sessions.runtimes {
            Load::Ready(view) => view,
            Load::Failed(error) => {
                return Err(Refusal::new(
                    Code::NotReady,
                    format!("the runtimes could not be listed: {error}"),
                ));
            }
            Load::Idle | Load::Loading => {
                return Err(Refusal::new(
                    Code::NotReady,
                    "the runtimes are not listed yet",
                ));
            }
        };
        let on_account = |runtime: &Runtime| {
            account.is_none_or(|account| {
                runtime.id.account.as_str() == account
                    || runtime
                        .account_email
                        .as_deref()
                        .is_some_and(|email| email.eq_ignore_ascii_case(account))
            })
        };
        let found: Vec<&Runtime> = view
            .runtimes
            .iter()
            .filter(|runtime| on_account(runtime))
            .filter(|runtime| {
                !wanted.is_empty()
                    && (runtime.id.name.as_deref() == Some(wanted) || runtime.id.endpoint == wanted)
            })
            .collect();
        match found.as_slice() {
            [runtime] => Ok(runtime),
            [] => {
                let mut message = format!("no listed runtime has the name or endpoint {wanted:?}");
                if let Some(account) = account {
                    message.push_str(&format!(" on {account}"));
                }
                if !view.errors.is_empty() {
                    message.push_str(&format!(
                        "; {} account(s) could not be listed, so their runtimes are unknown",
                        view.errors.len()
                    ));
                }
                Err(Refusal::new(Code::NoSuchRuntime, message))
            }
            many => {
                let named: Vec<String> = many
                    .iter()
                    .map(|runtime| {
                        format!(
                            "{} ({}) on {}",
                            runtime_label(runtime),
                            runtime.id.endpoint,
                            runtime
                                .account_email
                                .as_deref()
                                .unwrap_or(runtime.id.account.as_str())
                        )
                    })
                    .collect();
                Err(Refusal::new(
                    Code::AmbiguousRuntime,
                    format!(
                        "{} runtimes match {wanted:?}: {}. Name one by its endpoint, or give its \
                         account (--account)",
                        many.len(),
                        named.join(", ")
                    ),
                ))
            }
        }
    }

    /// `operation`: where an operation stands.
    pub(super) fn control_operation(&self, id: &str) -> Result<serde_json::Value, Refusal> {
        self.operation_reply(id)
    }

    /// `cancel`: stops a start that is under way; it gives back any runtime it got.
    pub(super) fn control_cancel(
        &mut self,
        args: CancelArgs,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, Refusal> {
        let operation = self
            .operations
            .find(&args.id)
            .ok_or_else(|| unknown_operation(&args.id))?;
        if operation.state != OperationState::Running {
            return Err(Refusal::new(
                Code::OperationEnded,
                format!(
                    "operation {:?} has ended: {}",
                    args.id,
                    format!("{:?}", operation.state).to_lowercase()
                ),
            ));
        }
        // While its record runs, the provision running is the one it follows: only one runs at
        // a time, and its end ends the record.
        let Kind::Start { name, .. } = &operation.kind else {
            return Err(Refusal::new(
                Code::NotCancellable,
                "a stop cannot be cancelled: once asked, it ends the runtime's billing",
            ));
        };
        if !args.yes {
            let operation = self.operation_answer(operation);
            return Ok(self.reply(Body::Preview(Preview::Cancel { operation })));
        }
        let name = name.clone();
        log::info!("control: cancelling operation {}", args.id);
        self.ops.cancel(Op::Provision);
        self.toast_from_command_line(keys::CTL_CANCELLING_TOAST, &name, cx);
        cx.notify();
        self.operation_reply(&args.id)
    }

    /// The provision that operation `id` follows reported `event`.
    pub(super) fn operation_progress(&mut self, id: &str, event: &ProvisionEvent) {
        if let Some(Operation {
            kind: Kind::Start { progress, .. },
            ..
        }) = self.operations.find_mut(id)
        {
            progress.push(event.clone());
        }
    }

    /// The provision or stop that operation `id` follows ended with `result`.
    pub(super) fn operation_ended(
        &mut self,
        id: &str,
        result: Result<Option<&ProvisionOutcome>, &Error>,
    ) {
        let now = self.backend_now();
        let Some(operation) = self.operations.find_mut(id) else {
            return;
        };
        operation.ended_at = now;
        match result {
            Ok(outcome) => {
                operation.state = OperationState::Succeeded;
                if let (Some(outcome), Kind::Start { outcome: kept, .. }) =
                    (outcome, &mut operation.kind)
                {
                    *kept = Some(outcome.clone());
                }
            }
            Err(error) if error.is(ErrorKind::Cancelled) => {
                operation.state = OperationState::Cancelled;
            }
            Err(error) => {
                operation.state = OperationState::Failed;
                operation.error = Some(error.clone());
            }
        }
        log::info!("control: operation {id} ended ({:?})", operation.state);
        self.operations.forget_old();
    }

    fn toast_from_command_line(&self, key: &str, name: &str, cx: &mut Context<Self>) {
        let toast = Toast::new(tr_args(cx, key, &[("name", name)])).severity(Severity::Info);
        self.toast(toast, cx);
    }

    fn operation_answer<'a>(&'a self, operation: &'a Operation) -> OperationAnswer<'a> {
        let kind = match &operation.kind {
            Kind::Start {
                name,
                accelerator,
                high_mem,
                progress,
                outcome,
            } => KindAnswer::Start {
                runtime: NewRuntime {
                    name,
                    accelerator: *accelerator,
                    high_mem: *high_mem,
                },
                progress,
                outcome: outcome.as_ref(),
            },
            Kind::Stop { target, label } => KindAnswer::Stop {
                runtime: StoppedRuntime { id: target, label },
            },
        };
        let running = operation.state == OperationState::Running;
        OperationAnswer {
            id: &operation.id,
            cancelling: running
                && matches!(operation.kind, Kind::Start { .. })
                && self.is_cancelling_provision(),
            kind,
            state: operation.state,
            started_at: operation.started_at,
            ended_at: operation.ended_at,
            account: AccountAnswer {
                id: &operation.account,
                email: operation.email.as_deref(),
            },
            error: operation.error.as_ref(),
        }
    }

    fn operation_reply(&self, id: &str) -> Result<serde_json::Value, Refusal> {
        let operation = self
            .operations
            .find(id)
            .ok_or_else(|| unknown_operation(id))?;
        Ok(self.reply(Body::Operation(self.operation_answer(operation))))
    }

    fn reply(&self, body: Body<'_>) -> serde_json::Value {
        let answer = Answer {
            now: self.backend_now(),
            body,
        };
        serde_json::to_value(answer).expect("answers serialise to JSON")
    }
}
