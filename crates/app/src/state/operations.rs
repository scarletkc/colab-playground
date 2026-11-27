//! What `colab-playground ctl` may change: starting and stopping runtimes, extending their
//! auto-stop, forwarding their ports, and the operations that follow those changes.
//!
//! A command that changes something changes nothing without `yes`: it answers with what it
//! would do, from what the window's own confirmation shows, or is refused with the code the
//! change would get. A start goes through the New session dialog's provision, and the rest
//! through what the Sessions page's buttons call, so the window shows them, the quit prompt
//! knows about them and the same limits hold. No argument loosens a cost guard; those stay with
//! the person at the window, and an extension adds what the window's Extend adds. A change that
//! would get in the way of what the window is doing (closing, updating, changing accounts, a
//! start under way, the New session dialog holding what someone entered) is refused, not queued.
//!
//! Each change gets an operation id: every one waits on a backend call, which the owner, which
//! answers one request at a time, never waits for. The record keeps the outcome itself, because
//! the window drops or replaces what showed it. An id starts with a part drawn at random once per
//! run, so an id from before a restart is unknown here instead of naming another operation.
//! Nothing ends an operation when the client that began it goes away: cancelling a start gives
//! the runtime back, so only its own command does that.

use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

use colab_playground_core::{
    Error, ErrorKind,
    accounts::AccountId,
    backend::{Extension, Runtime, RuntimeRef, accelerator_prefix, generate_session_name},
    clock::{serde_dur, serde_secs},
    guard::GuardPolicy,
    provision::{ProvisionEvent, ProvisionOutcome, ProvisionSpec},
    sessions::Accelerator,
    tunnel::{TunnelId, TunnelInfo, TunnelSpec, TunnelState},
    usage::published_rate,
};
use ely_gpui_component::{feedback::Toast, primitives::Severity};
use gpui::Context;
use serde::Serialize;

use super::{
    AccountDialog, AppState, Load, Op, Overlay, ProvisionPhase, SignInStatus, UpdatePhase,
    control::RuntimeAnswer,
    ops::key_of,
    runtime_label,
    tunnels::{FORWARD_START_PORT, local_url},
};
use crate::{
    control::{
        CancelArgs, CloseForwardArgs, Code, ExtendArgs, ForwardArgs, OperationState, StartArgs,
        StopArgs,
    },
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
    Extend {
        target: RuntimeRef,
        label: String,
        by: Duration,
        outcome: Option<ExtendOutcome>,
    },
    Forward {
        spec: TunnelSpec,
        /// Known once the backend has opened it.
        tunnel: Option<TunnelId>,
    },
    CloseForward {
        spec: TunnelSpec,
    },
}

/// Where an extension put the auto-stop.
#[derive(Serialize)]
struct ExtendOutcome {
    #[serde(serialize_with = "serde_secs::serialize")]
    auto_stop_at: SystemTime,
    /// When the runtime stops itself, also with the app closed. `None`: nothing on the runtime
    /// stops it, so the stop holds only while the app runs.
    #[serde(serialize_with = "serde_secs::option::serialize")]
    auto_stop_on_runtime_at: Option<SystemTime>,
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
    Extend(ExtendPreview<'a>),
    Forward(ForwardPreview<'a>),
    CloseForward(CloseForwardPreview<'a>),
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

/// What the card's "Extend by …" says. Not the new time: the backend reads the time left when
/// it extends, and the listing can be older than the last extension.
#[derive(Serialize)]
struct ExtendPreview<'a> {
    runtime: RuntimeAnswer<'a>,
    #[serde(serialize_with = "serde_dur::serialize")]
    extend_by: Duration,
}

#[derive(Serialize)]
struct ForwardPreview<'a> {
    runtime: RuntimeAnswer<'a>,
    forward: ForwardAnswer<'a>,
}

#[derive(Serialize)]
struct CloseForwardPreview<'a> {
    forward: ShownForward<'a>,
    /// Always false: the runtime keeps running and billing, as the window says when it closes
    /// one.
    stops_billing: bool,
}

/// A forward as `sessions` lists it, with its address.
#[derive(Serialize)]
struct ShownForward<'a> {
    #[serde(flatten)]
    info: &'a TunnelInfo,
    url: String,
}

#[derive(Serialize)]
struct ForwardAnswer<'a> {
    session: &'a str,
    remote_port: u16,
    local_port: u16,
    health_path: Option<&'a str>,
    url: String,
}

impl<'a> ForwardAnswer<'a> {
    fn of(spec: &'a TunnelSpec) -> Self {
        ForwardAnswer {
            session: &spec.session,
            remote_port: spec.remote_port,
            local_port: spec.local_port,
            health_path: spec.health_path.as_deref(),
            url: local_url(spec),
        }
    }
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
        runtime: TargetRuntime<'a>,
    },
    Extend {
        runtime: TargetRuntime<'a>,
        #[serde(serialize_with = "serde_dur::serialize")]
        extend_by: Duration,
        outcome: Option<&'a ExtendOutcome>,
    },
    Forward {
        forward: ForwardAnswer<'a>,
    },
    CloseForward {
        forward: ForwardAnswer<'a>,
    },
}

#[derive(Serialize)]
struct NewRuntime<'a> {
    name: &'a str,
    accelerator: Accelerator,
    high_mem: bool,
}

/// A listed runtime an operation acts on.
#[derive(Serialize)]
struct TargetRuntime<'a> {
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

/// The forward on `local_port` of this computer: the open one there (an open forward holds its
/// port alone), else one that failed there and is still shown. Never a guess between two.
fn pick_forward<'a>(
    tunnels: impl Iterator<Item = &'a TunnelInfo>,
    local_port: u16,
) -> Result<&'a TunnelInfo, Refusal> {
    let on_port: Vec<&TunnelInfo> = tunnels
        .filter(|tunnel| tunnel.spec.local_port == local_port)
        .collect();
    let open: Vec<&TunnelInfo> = on_port
        .iter()
        .copied()
        .filter(|tunnel| !tunnel.state.is_terminal())
        .collect();
    let candidates = if open.is_empty() { on_port } else { open };
    match candidates.as_slice() {
        [tunnel] => Ok(tunnel),
        [] => Err(Refusal::new(
            Code::NoSuchForward,
            format!("no forward listens on localhost:{local_port}"),
        )),
        many => {
            let named: Vec<String> = many
                .iter()
                .map(|tunnel| {
                    format!(
                        "port {} of {}",
                        tunnel.spec.remote_port, tunnel.spec.session
                    )
                })
                .collect();
            Err(Refusal::new(
                Code::AmbiguousForward,
                format!(
                    "{} forwards are listed on localhost:{local_port}: {}. Close them in the \
                     window",
                    many.len(),
                    named.join(", ")
                ),
            ))
        }
    }
}

/// How an operation that opens a forward ends when the forward reaches `state`: once it is up,
/// or once it failed or was closed before that. `None` while it is still coming up.
fn forward_outcome(state: &TunnelState) -> Option<Result<(), Error>> {
    match state {
        TunnelState::Up => Some(Ok(())),
        TunnelState::Failed { error } => Some(Err(error.clone())),
        // Closed by hand, by a stop of its runtime or by a switch of account.
        TunnelState::Stopped => Some(Err(Error::new(ErrorKind::Cancelled))),
        TunnelState::Starting | TunnelState::Reconnecting { .. } => None,
    }
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
        self.toast_from_command_line(keys::CTL_STARTING_TOAST, &[("name", &name)], cx);
        self.operation_reply(&id)
    }

    /// The active account and its email, or why a start cannot begin now.
    fn start_refusal(&self) -> Result<(AccountId, Option<String>), Refusal> {
        let active = self.active_account()?;
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
        Ok(active)
    }

    /// The active account and its email, or why nothing tied to it can happen now: a start, or
    /// extending or forwarding one of its runtimes.
    fn active_account(&self) -> Result<(AccountId, Option<String>), Refusal> {
        self.changes_refusal()?;
        // What is tied to the active account would run under a switch or sign-in opened before
        // it, so the window opens neither while a start runs, and the switch dialog lists the
        // forwards it will close as they were when it opened.
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
        Ok((account, email))
    }

    /// The one listed runtime `wanted` names, for a change the window's card offers only on the
    /// active account. Checked against the account that is active now, not the listing's mark,
    /// which is out of date from a switch until the next listing.
    fn runtime_on_active_account(
        &self,
        wanted: &str,
        account: Option<&str>,
    ) -> Result<&Runtime, Refusal> {
        let (active, active_email) = self.active_account()?;
        let runtime = self.find_runtime(wanted, account)?;
        let label = runtime_label(runtime);
        if self.is_stopping(&runtime.id) {
            return Err(Refusal::new(
                Code::AlreadyStopping,
                format!("{label} is being stopped"),
            ));
        }
        if runtime.id.account != active {
            return Err(Refusal::new(
                Code::AccountNotActive,
                format!(
                    "{label} is on {}, and the active account is {}. Extending and forwarding \
                     need the runtime's account to be the active one; switch to it in the window",
                    runtime
                        .account_email
                        .as_deref()
                        .unwrap_or(runtime.id.account.as_str()),
                    active_email.as_deref().unwrap_or(active.as_str())
                ),
            ));
        }
        Ok(runtime)
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
        self.toast_from_command_line(keys::CTL_STOPPING_TOAST, &[("name", &label)], cx);
        self.operation_reply(&id)
    }

    /// `extend`: moves a runtime's auto-stop later by what the card's Extend adds.
    pub(super) fn control_extend(
        &mut self,
        args: ExtendArgs,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, Refusal> {
        let runtime = self.runtime_on_active_account(&args.runtime, args.account.as_deref())?;
        let label = runtime_label(runtime);
        // Where the card offers Extend. The backend gives a runtime without an auto-stop a new
        // one, which is the card's "Set auto-stop", not an extension.
        if runtime.id.name.is_none() || runtime.auto_stop_at.is_none() {
            let why = if runtime.managed {
                "its auto-stop is off (it was started without one, or it was cancelled), so it \
                 bills until it is stopped; the window can set one"
            } else {
                "it was not started by this app, which has no auto-stop on it"
            };
            return Err(Refusal::new(
                Code::NoAutoStop,
                format!("{label} has no auto-stop to extend: {why}"),
            ));
        }
        let key = key_of(&runtime.id);
        if self.ops.is_running(Op::Extend(key)) || self.ops.is_running(Op::CancelAutoStop(key)) {
            return Err(Refusal::new(
                Code::AutoStopChanging,
                format!("the auto-stop of {label} is being changed already; ask again once it is"),
            ));
        }
        let Some(by) = self.rearm_duration() else {
            return Err(Refusal::new(
                Code::NoAutoStop,
                "no runtime limit is set to extend by",
            ));
        };
        if !args.yes {
            let preview = ExtendPreview {
                runtime: self.runtime_answer(runtime),
                extend_by: by,
            };
            return Ok(self.reply(Body::Preview(Preview::Extend(preview))));
        }
        let target = runtime.id.clone();
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
            kind: Kind::Extend {
                target: target.clone(),
                label: label.clone(),
                by,
                outcome: None,
            },
        });
        log::info!("control: extending {label} as operation {id}");
        self.extend_session_for(target, by, Some(id.clone()), cx);
        self.toast_from_command_line(keys::CTL_EXTENDING_TOAST, &[("name", &label)], cx);
        self.operation_reply(&id)
    }

    /// `forward`: forwards a port of a runtime on the active account to this computer, as the
    /// card's "Forward a port" does.
    pub(super) fn control_forward(
        &mut self,
        args: ForwardArgs,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, Refusal> {
        let runtime = self.runtime_on_active_account(&args.runtime, args.account.as_deref())?;
        let label = runtime_label(runtime);
        // The CLI reaches a runtime by its session name.
        let Some(session) = runtime.id.name.clone() else {
            return Err(Refusal::new(
                Code::UnnamedRuntime,
                format!(
                    "{label} was opened in the browser or another tool and has no session name, \
                     which a forward needs"
                ),
            ));
        };
        let remote_port = args.remote_port.unwrap_or(FORWARD_START_PORT);
        let spec = TunnelSpec {
            account: runtime.id.account.clone(),
            session,
            local_port: args.local_port.unwrap_or(remote_port),
            remote_port,
            health_path: args.health_path,
        };
        spec.validate()
            .map_err(|error| Refusal::new(Code::InvalidRequest, error.to_string()))?;
        let taken = self.tunnels().find(|tunnel| {
            tunnel.spec.local_port == spec.local_port && !tunnel.state.is_terminal()
        });
        if let Some(taken) = taken {
            return Err(Refusal::new(
                Code::PortInUse,
                format!(
                    "{} forwards port {} of {} already; close that forward first \
                     (close-forward {}), or choose another local port",
                    local_url(&taken.spec),
                    taken.spec.remote_port,
                    taken.spec.session,
                    taken.spec.local_port
                ),
            ));
        }
        if self.is_opening_forward(&spec) {
            return Err(Refusal::new(
                Code::AlreadyForwarding,
                format!(
                    "port {} of {label} is being forwarded already",
                    spec.remote_port
                ),
            ));
        }
        if !args.yes {
            let preview = ForwardPreview {
                runtime: self.runtime_answer(runtime),
                forward: ForwardAnswer::of(&spec),
            };
            return Ok(self.reply(Body::Preview(Preview::Forward(preview))));
        }
        let email = runtime.account_email.clone();
        let started_at = self.backend_now();
        let port = spec.remote_port.to_string();
        let id = self.operations.begin(|id| Operation {
            id,
            account: spec.account.clone(),
            email,
            started_at,
            ended_at: None,
            state: OperationState::Running,
            error: None,
            kind: Kind::Forward {
                spec: spec.clone(),
                tunnel: None,
            },
        });
        log::info!("control: forwarding port {port} of {label} as operation {id}");
        self.open_tunnel_for(spec, false, Some(id.clone()), cx);
        self.toast_from_command_line(
            keys::CTL_FORWARDING_TOAST,
            &[("name", &label), ("port", &port)],
            cx,
        );
        self.operation_reply(&id)
    }

    /// `close-forward`: closes the forward on a port of this computer, as its button in the
    /// window does. The runtime keeps running and billing.
    pub(super) fn control_close_forward(
        &mut self,
        args: CloseForwardArgs,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, Refusal> {
        self.changes_refusal()?;
        let tunnel = pick_forward(self.tunnels(), args.local_port)?;
        if self.ops.is_running(Op::CloseForward(tunnel.id.0)) {
            return Err(Refusal::new(
                Code::AlreadyClosing,
                format!(
                    "the forward at {} is being closed already",
                    local_url(&tunnel.spec)
                ),
            ));
        }
        if !args.yes {
            let preview = CloseForwardPreview {
                forward: ShownForward {
                    info: tunnel,
                    url: local_url(&tunnel.spec),
                },
                stops_billing: false,
            };
            return Ok(self.reply(Body::Preview(Preview::CloseForward(preview))));
        }
        let (tunnel_id, spec) = (tunnel.id, tunnel.spec.clone());
        let failed = tunnel.state.is_terminal();
        let email = self.account_email(&spec.account).map(str::to_owned);
        let started_at = self.backend_now();
        let port = spec.local_port.to_string();
        let id = self.operations.begin(|id| Operation {
            id,
            account: spec.account.clone(),
            email,
            started_at,
            ended_at: None,
            state: OperationState::Running,
            error: None,
            kind: Kind::CloseForward { spec },
        });
        log::info!("control: closing the forward at localhost:{port} as operation {id}");
        if failed {
            // The window shows a failed forward until it is dismissed; nothing is left to close.
            self.dismiss_forward(tunnel_id, cx);
            self.operation_ended(&id, Ok(None));
        } else {
            self.close_forward_for(tunnel_id, Some(id.clone()), cx);
        }
        self.toast_from_command_line(keys::CTL_CLOSING_FORWARD_TOAST, &[("port", &port)], cx);
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
        let name = match &operation.kind {
            Kind::Start { name, .. } => name,
            Kind::Stop { .. } => {
                return Err(Refusal::new(
                    Code::NotCancellable,
                    "a stop cannot be cancelled: once asked, it ends the runtime's billing",
                ));
            }
            Kind::Forward { spec, .. } => {
                return Err(Refusal::new(
                    Code::NotCancellable,
                    format!(
                        "only a start can be cancelled; a forward that is coming up ends when it \
                         is closed (close-forward {})",
                        spec.local_port
                    ),
                ));
            }
            Kind::Extend { .. } | Kind::CloseForward { .. } => {
                return Err(Refusal::new(
                    Code::NotCancellable,
                    "only a start can be cancelled; this operation ends on its own in a moment",
                ));
            }
        };
        if !args.yes {
            let operation = self.operation_answer(operation);
            return Ok(self.reply(Body::Preview(Preview::Cancel { operation })));
        }
        let name = name.clone();
        log::info!("control: cancelling operation {}", args.id);
        self.ops.cancel(Op::Provision);
        self.toast_from_command_line(keys::CTL_CANCELLING_TOAST, &[("name", &name)], cx);
        cx.notify();
        self.operation_reply(&args.id)
    }

    /// The extension that operation `id` follows ended with `result`.
    pub(super) fn operation_extended(&mut self, id: &str, result: Result<&Extension, &Error>) {
        if let (
            Ok(set),
            Some(Operation {
                kind: Kind::Extend { outcome, .. },
                ..
            }),
        ) = (result, self.operations.find_mut(id))
        {
            *outcome = Some(ExtendOutcome {
                auto_stop_at: set.auto_stop_at(),
                auto_stop_on_runtime_at: set.watchdog.enforced_deadline(),
            });
        }
        self.operation_ended(id, result.map(|_| None));
    }

    /// The forward that operation `id` opens got `tunnel` as its id.
    pub(super) fn operation_follows_tunnel(&mut self, id: &str, tunnel: TunnelId) {
        if let Some(Operation {
            kind: Kind::Forward {
                tunnel: followed, ..
            },
            ..
        }) = self.operations.find_mut(id)
        {
            *followed = Some(tunnel);
        }
    }

    /// Forward `tunnel` reached `state`. Every backend reports a forward's last state before it
    /// stops reporting, so an operation that opens one always ends here.
    pub(super) fn operation_tunnel_state(&mut self, tunnel: TunnelId, state: &TunnelState) {
        let opening = self.operations.list.iter().find(|operation| {
            operation.state == OperationState::Running
                && matches!(operation.kind, Kind::Forward { tunnel: Some(followed), .. } if followed == tunnel)
        });
        let Some(id) = opening.map(|operation| operation.id.clone()) else {
            return;
        };
        match forward_outcome(state) {
            Some(Ok(())) => self.operation_ended(&id, Ok(None)),
            Some(Err(error)) => self.operation_ended(&id, Err(&error)),
            None => {}
        }
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

    /// What operation `id` follows ended with `result`; only a start has an outcome to keep.
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

    fn toast_from_command_line(&self, key: &str, args: &[(&str, &str)], cx: &mut Context<Self>) {
        let toast = Toast::new(tr_args(cx, key, args)).severity(Severity::Info);
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
                runtime: TargetRuntime { id: target, label },
            },
            Kind::Extend {
                target,
                label,
                by,
                outcome,
            } => KindAnswer::Extend {
                runtime: TargetRuntime { id: target, label },
                extend_by: *by,
                outcome: outcome.as_ref(),
            },
            Kind::Forward { spec, .. } => KindAnswer::Forward {
                forward: ForwardAnswer::of(spec),
            },
            Kind::CloseForward { spec } => KindAnswer::CloseForward {
                forward: ForwardAnswer::of(spec),
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

#[cfg(test)]
mod tests {
    use colab_playground_core::{
        Error, ErrorKind,
        accounts::AccountId,
        tunnel::{TunnelId, TunnelInfo, TunnelSpec, TunnelState},
    };

    use super::{forward_outcome, pick_forward};
    use crate::control::Code;

    fn forward(id: u64, local_port: u16, state: TunnelState) -> TunnelInfo {
        TunnelInfo {
            id: TunnelId(id),
            spec: TunnelSpec {
                account: AccountId::new("ada").expect("an account id"),
                session: format!("t4-{id}"),
                local_port,
                remote_port: 8000,
                health_path: None,
            },
            state,
            healthy: None,
        }
    }

    fn failed() -> TunnelState {
        TunnelState::Failed {
            error: Error::new(ErrorKind::SshBusy),
        }
    }

    #[test]
    fn a_forward_is_named_by_its_port_and_never_guessed() {
        let picked = |tunnels: &[TunnelInfo], port| {
            pick_forward(tunnels.iter(), port)
                .map(|tunnel| tunnel.id.0)
                .map_err(|refusal| refusal.code)
        };
        let open = forward(1, 8000, TunnelState::Up);
        assert_eq!(picked(std::slice::from_ref(&open), 8000), Ok(1));
        assert_eq!(
            picked(std::slice::from_ref(&open), 8001).unwrap_err(),
            Code::NoSuchForward
        );
        assert_eq!(
            picked(&[forward(2, 8000, failed()), open.clone()], 8000),
            Ok(1),
            "the open one, not one that failed on the same port before"
        );
        assert_eq!(
            picked(&[forward(2, 8000, failed())], 8000),
            Ok(2),
            "one that failed is cleared as the window's button does"
        );
        assert_eq!(
            picked(
                &[forward(2, 8000, failed()), forward(3, 8000, failed())],
                8000
            )
            .unwrap_err(),
            Code::AmbiguousForward
        );
        let reconnecting = TunnelState::Reconnecting {
            attempt: 1,
            delay_secs: 1,
        };
        assert_eq!(
            picked(&[open, forward(4, 8000, reconnecting)], 8000).unwrap_err(),
            Code::AmbiguousForward,
            "two open ones, as a listing a moment behind can show, are not guessed between"
        );
    }

    #[test]
    fn opening_a_forward_ends_once_it_is_up_or_gone() {
        assert_eq!(forward_outcome(&TunnelState::Starting), None);
        assert_eq!(
            forward_outcome(&TunnelState::Reconnecting {
                attempt: 2,
                delay_secs: 2
            }),
            None,
            "still trying to come up"
        );
        assert_eq!(forward_outcome(&TunnelState::Up), Some(Ok(())));
        assert_eq!(
            forward_outcome(&failed()),
            Some(Err(Error::new(ErrorKind::SshBusy)))
        );
        assert!(
            forward_outcome(&TunnelState::Stopped)
                .is_some_and(|ended| ended.is_err_and(|error| error.is(ErrorKind::Cancelled))),
            "closed before it was up"
        );
    }
}
