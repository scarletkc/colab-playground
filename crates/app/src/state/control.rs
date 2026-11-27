//! What `colab-playground ctl` is told: what the app shows now, as JSON with stable names.
//!
//! Unknown is never reported as empty or zero (`docs/DESIGN.md`, "Invariants"). A list the
//! app has not loaded is left out and its `state` says why; a total over accounts whose usage
//! is not known says how many it misses; stale values carry the marker the window shows.

use std::time::{Duration, SystemTime};

use colab_playground_core::{
    Error,
    accounts::{AccountId, AccountsView},
    backend::{Runtime, RuntimeRef},
    clock::{serde_dur, serde_secs},
    guard::{Concern, GuardPolicy},
    sessions::Accelerator,
    tunnel::TunnelInfo,
    usage::{BalancePoint, UsageSnapshot},
};
use gpui::{App, Context};
use serde::Serialize;

use super::{AppState, EndReason, Load, Op, runtime_label, usage::is_stale};
use crate::{
    control::{Code, Command, Response},
    i18n::{keys, tr, tr_args},
};

/// A value the app loads, as `{"state": ...}` with the value's fields beside it when ready.
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum LoadState<'a, T> {
    /// Not asked for yet (the CLI is not set up, or no account is saved).
    NotLoaded,
    /// Being fetched for the first time.
    Loading,
    Ready(T),
    Failed {
        error: &'a Error,
    },
}

fn load_state<'a, T, U>(load: &'a Load<T>, ready: impl FnOnce(&'a T) -> U) -> LoadState<'a, U> {
    match load {
        Load::Idle => LoadState::NotLoaded,
        Load::Loading => LoadState::Loading,
        Load::Ready(value) => LoadState::Ready(ready(value)),
        Load::Failed(error) => LoadState::Failed { error },
    }
}

#[derive(Serialize)]
struct StatusAnswer<'a> {
    #[serde(serialize_with = "serde_secs::option::serialize")]
    now: Option<SystemTime>,
    version: &'static str,
    /// The backend could not be built; nothing else can load.
    backend_error: Option<&'a Error>,
    environment: LoadState<'a, Environment<'a>>,
    accounts: LoadState<'a, AccountsSummary<'a>>,
    runtimes: LoadState<'a, RuntimesSummary>,
    usage: LoadState<'a, UsageTotal>,
    activity: Vec<&'static str>,
}

#[derive(Serialize)]
struct Environment<'a> {
    cli_installed: bool,
    cli_version: Option<&'a str>,
}

#[derive(Serialize)]
struct AccountsSummary<'a> {
    saved: usize,
    active: Option<ActiveAccount<'a>>,
    /// The CLI is signed in to an account the app has not saved.
    unadopted: bool,
}

#[derive(Serialize)]
struct ActiveAccount<'a> {
    id: &'a AccountId,
    email: Option<&'a str>,
}

#[derive(Serialize)]
struct RuntimesSummary {
    running: usize,
    /// Accounts whose runtimes could not be listed: `running` does not count theirs.
    unlisted_accounts: usize,
}

#[derive(Serialize)]
struct AccountsAnswer<'a> {
    #[serde(serialize_with = "serde_secs::option::serialize")]
    now: Option<SystemTime>,
    #[serde(flatten)]
    accounts: LoadState<'a, &'a AccountsView>,
    refreshing: bool,
}

#[derive(Serialize)]
struct SessionsAnswer<'a> {
    #[serde(serialize_with = "serde_secs::option::serialize")]
    now: Option<SystemTime>,
    #[serde(flatten)]
    listing: LoadState<'a, Listing<'a>>,
    /// A listing is under way: what is shown may change when it is back.
    refreshing: bool,
    /// The last cost-guard round failed, so no limit was checked then.
    guard_error: Option<&'a Error>,
    /// Runtimes that ended while the app watched, newest first.
    ended: Vec<EndedAnswer<'a>>,
}

#[derive(Serialize)]
struct Listing<'a> {
    #[serde(serialize_with = "serde_secs::option::serialize")]
    listed_at: Option<SystemTime>,
    runtimes: Vec<RuntimeAnswer<'a>>,
    /// Accounts that could not be listed: their runtimes are unknown, not absent.
    unlisted_accounts: Vec<UnlistedAccount<'a>>,
}

#[derive(Serialize)]
struct RuntimeAnswer<'a> {
    id: &'a RuntimeRef,
    label: String,
    account_email: Option<&'a str>,
    on_active_account: bool,
    managed: bool,
    accelerator: Option<Accelerator>,
    hardware: &'a str,
    gpu: Option<&'a str>,
    vram_mib: Option<u32>,
    high_mem: bool,
    region: Option<&'a str>,
    #[serde(serialize_with = "serde_secs::option::serialize")]
    started_at: Option<SystemTime>,
    estimated_rate_cu_per_hour: Option<f64>,
    estimated_spend_cu: Option<f64>,
    #[serde(serialize_with = "serde_secs::option::serialize")]
    auto_stop_at: Option<SystemTime>,
    /// The runtime enforces the auto-stop itself; otherwise it holds only while the app is open.
    auto_stop_on_runtime: bool,
    policy: Option<&'a GuardPolicy>,
    stopping: bool,
    warnings: &'a [Concern],
    /// The cost guard tried to stop it and failed: it is still billing.
    stop_failure: Option<&'a Error>,
    tunnels: Vec<&'a TunnelInfo>,
}

#[derive(Serialize)]
struct UnlistedAccount<'a> {
    account: &'a AccountId,
    email: Option<&'a str>,
    error: &'a Error,
}

#[derive(Serialize)]
struct EndedAnswer<'a> {
    id: &'a RuntimeRef,
    label: &'a str,
    email: Option<&'a str>,
    reason: &'static str,
    concern: Option<&'a Concern>,
    #[serde(serialize_with = "serde_secs::option::serialize")]
    at: Option<SystemTime>,
}

#[derive(Serialize)]
struct UsageAnswer<'a> {
    #[serde(serialize_with = "serde_secs::option::serialize")]
    now: Option<SystemTime>,
    #[serde(flatten)]
    usage: LoadState<'a, UsageReady<'a>>,
    refreshing: bool,
}

#[derive(Serialize)]
struct UsageReady<'a> {
    /// The last refresh failed as a whole.
    error: Option<&'a Error>,
    total: UsageTotal,
    accounts: Vec<AccountUsageAnswer<'a>>,
}

/// Sums over the accounts whose usage is known; `null` while none is.
#[derive(Serialize)]
struct UsageTotal {
    balance_cu: Option<f64>,
    rate_cu_per_hour: Option<f64>,
    runtimes: Option<u32>,
    known_accounts: usize,
    /// Accounts left out of the sums.
    unknown_accounts: usize,
    stale: bool,
}

#[derive(Serialize)]
struct AccountUsageAnswer<'a> {
    account: &'a AccountId,
    email: Option<&'a str>,
    active: bool,
    /// `null`: not known (not fetched yet, or every fetch failed).
    snapshot: Option<&'a UsageSnapshot>,
    /// The window marks these values stale: a later fetch failed, or they are old.
    stale: Option<bool>,
    #[serde(serialize_with = "serde_dur::option::serialize")]
    eta_secs: Option<Duration>,
    /// Runtimes on the account the app does not manage; `null` when that is not known.
    unmanaged_runtimes: Option<u32>,
    error: Option<&'a Error>,
    history: &'a [BalancePoint],
}

impl AppState {
    pub fn allows_cli_control(&self) -> bool {
        self.settings.allow_cli_control
    }

    pub fn set_cli_control(&mut self, on: bool, cx: &mut Context<Self>) {
        log::info!("control: from the command line -> {on}");
        self.settings.allow_cli_control = on;
        self.persist_settings(cx);
        cx.notify();
    }

    /// The answer to a read command, or why there is none.
    pub fn control_answer(&self, command: Command, cx: &App) -> Response {
        let flavor = self.flavor();
        if !self.allows_cli_control() {
            let message = tr_args(
                cx,
                keys::CONTROL_OFF,
                &[
                    ("app", &tr(cx, keys::APP_NAME)),
                    ("page", &tr(cx, keys::NAV_SETTINGS)),
                    ("section", &tr(cx, keys::SETTINGS_CONTROL_SECTION_TITLE)),
                    ("setting", &tr(cx, keys::SETTINGS_CONTROL_TITLE)),
                ],
            );
            return Response::failure(Some(flavor), Code::ControlOff, Some(message.to_string()));
        }
        let data = match command {
            Command::Status => serde_json::to_value(self.status_answer()),
            Command::Accounts => serde_json::to_value(self.accounts_answer()),
            Command::Sessions => serde_json::to_value(self.sessions_answer()),
            Command::Usage => serde_json::to_value(self.usage_answer()),
            Command::Activate => Ok(serde_json::json!({})),
        };
        Response::success(flavor, data.expect("answers serialise to JSON"))
    }

    fn status_answer(&self) -> StatusAnswer<'_> {
        let usage = load_state(&self.accounts, |_| self.usage_total());
        StatusAnswer {
            now: self.backend_now(),
            version: env!("CARGO_PKG_VERSION"),
            backend_error: self.backend_error(),
            environment: load_state(&self.probe, |report| Environment {
                cli_installed: report.colab.is_some(),
                cli_version: report.colab_version.as_deref(),
            }),
            accounts: load_state(&self.accounts, |view| AccountsSummary {
                saved: view.accounts.len(),
                active: view.active_account().map(|account| ActiveAccount {
                    id: &account.id,
                    email: account.email.as_deref(),
                }),
                unadopted: view.unadopted,
            }),
            runtimes: load_state(&self.sessions.runtimes, |view| RuntimesSummary {
                running: view.runtimes.len(),
                unlisted_accounts: view.errors.len(),
            }),
            usage,
            activity: self.activity(),
        }
    }

    /// What the app is busy with, in the user's terms.
    fn activity(&self) -> Vec<&'static str> {
        let ops = &self.ops;
        [
            ("installing_cli", ops.is_running(Op::Install)),
            ("adding_account", ops.is_running(Op::Login)),
            ("switching_account", ops.is_running(Op::Switch)),
            ("starting_runtime", self.is_provisioning()),
            ("deploying", ops.is_running(Op::PluginRun)),
            (
                "stopping_runtimes",
                ops.any_of(|op| matches!(op, Op::Stop(_) | Op::StopAll)),
            ),
            (
                "updating",
                ops.any_of(|op| matches!(op, Op::UpdateInstall | Op::UpdateRestart)),
            ),
        ]
        .into_iter()
        .filter_map(|(name, busy)| busy.then_some(name))
        .collect()
    }

    fn accounts_answer(&self) -> AccountsAnswer<'_> {
        AccountsAnswer {
            now: self.backend_now(),
            accounts: load_state(&self.accounts, |view| view),
            refreshing: self.ops.is_running(Op::Accounts),
        }
    }

    fn sessions_answer(&self) -> SessionsAnswer<'_> {
        let listing = load_state(&self.sessions.runtimes, |view| Listing {
            listed_at: self.sessions.now,
            runtimes: view
                .runtimes
                .iter()
                .map(|runtime| self.runtime_answer(runtime))
                .collect(),
            unlisted_accounts: view
                .errors
                .iter()
                .map(|failed| UnlistedAccount {
                    account: &failed.account,
                    email: self.account_email(&failed.account),
                    error: &failed.error,
                })
                .collect(),
        });
        SessionsAnswer {
            now: self.backend_now(),
            listing,
            refreshing: self.sessions.runtimes.ready().is_some()
                && self.ops.is_running(Op::Runtimes),
            guard_error: self.guard_error(),
            ended: self
                .ended()
                .iter()
                .map(|ended| {
                    let (reason, concern) = match &ended.reason {
                        EndReason::User => ("user", None),
                        EndReason::Guard(concern) => ("guard", Some(concern)),
                        EndReason::Vanished => ("vanished", None),
                    };
                    EndedAnswer {
                        id: &ended.runtime,
                        label: &ended.label,
                        email: ended.email.as_deref(),
                        reason,
                        concern,
                        at: ended.at,
                    }
                })
                .collect(),
        }
    }

    fn runtime_answer<'a>(&'a self, runtime: &'a Runtime) -> RuntimeAnswer<'a> {
        let tracked = runtime.tracked.as_ref();
        RuntimeAnswer {
            id: &runtime.id,
            label: runtime_label(runtime),
            account_email: runtime.account_email.as_deref(),
            on_active_account: runtime.on_active_account,
            managed: runtime.managed,
            accelerator: runtime.accelerator,
            hardware: &runtime.hardware,
            gpu: tracked.and_then(|t| t.gpu.as_deref()),
            vram_mib: tracked.and_then(|t| t.vram_mib),
            high_mem: runtime.high_mem,
            region: runtime.region.as_deref(),
            started_at: tracked.map(|t| t.started_at()),
            estimated_rate_cu_per_hour: runtime.estimated_rate_cu_per_hour,
            estimated_spend_cu: runtime.estimated_spend_cu,
            auto_stop_at: runtime.auto_stop_at,
            auto_stop_on_runtime: tracked
                .and_then(|t| t.watchdog.as_ref())
                .and_then(|w| w.enforced_deadline())
                .is_some(),
            policy: tracked.map(|t| &t.policy),
            stopping: self.is_stopping(&runtime.id),
            warnings: self.guard_warnings(&runtime.id),
            stop_failure: self.guard_stop_failure(&runtime.id),
            tunnels: self.tunnels_for(&runtime.id),
        }
    }

    fn account_email(&self, id: &AccountId) -> Option<&str> {
        self.accounts
            .ready()?
            .accounts
            .iter()
            .find(|account| &account.id == id)?
            .email
            .as_deref()
    }

    fn usage_total(&self) -> UsageTotal {
        let summary = self.usage_summary();
        let known = summary.known > 0;
        // As the window marks it: old values count, not only a failed refresh.
        let now = self.usage_now();
        let stale = self.accounts.ready().is_some_and(|view| {
            view.accounts.iter().any(|account| {
                self.usage_for(&account.id)
                    .and_then(|usage| usage.snapshot.as_ref())
                    .is_some_and(|snapshot| is_stale(snapshot, now))
            })
        });
        UsageTotal {
            balance_cu: known.then_some(summary.balance_cu),
            rate_cu_per_hour: known.then_some(summary.rate_cu_per_hour),
            runtimes: known.then_some(summary.runtimes),
            known_accounts: summary.known,
            unknown_accounts: summary.unknown,
            stale,
        }
    }

    fn usage_answer(&self) -> UsageAnswer<'_> {
        let now = self.usage_now();
        let usage = load_state(&self.accounts, |view| UsageReady {
            error: self.usage_error(),
            total: self.usage_total(),
            accounts: view
                .accounts
                .iter()
                .map(|account| {
                    let usage = self.usage_for(&account.id);
                    let snapshot = usage.and_then(|usage| usage.snapshot.as_ref());
                    AccountUsageAnswer {
                        account: &account.id,
                        email: account.email.as_deref(),
                        active: view.active.as_ref() == Some(&account.id),
                        snapshot,
                        stale: snapshot.map(|snapshot| is_stale(snapshot, now)),
                        eta_secs: usage.and_then(|usage| usage.eta),
                        unmanaged_runtimes: usage.and_then(|usage| {
                            usage.ownership_verified.then_some(usage.unmanaged_runtimes)
                        }),
                        error: usage.and_then(|usage| usage.last_error.as_ref()),
                        history: usage.map_or(&[], |usage| usage.history.as_slice()),
                    }
                })
                .collect(),
        });
        UsageAnswer {
            now: self.backend_now(),
            usage,
            refreshing: self.ops.is_running(Op::Usage),
        }
    }
}
