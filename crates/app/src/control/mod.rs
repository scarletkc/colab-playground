//! Control from the command line: `colab-playground ctl <command>` asks the copy of the app
//! that owns the state, so what a script or an AI agent reads and does is what the window shows.
//!
//! Only one process may own a state: core keeps the credential slot lock, the ledger, the
//! session cap and the tunnels in memory, so two processes on the same data would corrupt each
//! other. The owner holds a lock the OS drops with the process, so an owner that crashed never
//! blocks the next start. A state is the real backend's data folder, or the mock's config
//! folder (the screenshot script runs several mocks side by side, each with its own folder).
//! On Windows and macOS the real data folder is the default config folder too, so every file
//! name here carries the backend kind: a mock owner and a real owner may run side by side, and
//! a client looking for one can never reach the other.
//!
//! The owner listens on a loopback port the OS picks. Every local user can reach a loopback
//! port, so the boundary is a token, new at every start, kept in a file only this user can
//! read; the request carries it in the body, never on a command line. One JSON object per
//! line each way, one request per connection, answered at once: a long operation answers with
//! an id that the client follows with further requests, so no connection waits on one.

mod ctl;
mod owner;

use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

use colab_playground_core::{
    backend::BackendKind as Flavor, paths::default_local_data_dir, sessions::Accelerator,
};
use serde::{Deserialize, Serialize};

pub use ctl::{CtlArgs, CtlError, Outcome, parse_args, run as run_ctl, usage as ctl_usage};
pub use owner::{Claim, Owner, Service, answer, claim, claim_within, listen, mark_quitting, serve};

/// The first argument that makes the program a client instead of the app. The same program,
/// because Windows ships one exe that self-update replaces whole.
pub const CTL_COMMAND: &str = "ctl";

/// Bumped when a request or a response changes in a way an older copy would misread. Not for a
/// new command: an older owner refuses a command it does not know as `invalid_request`, and a
/// bump would also keep a newer second launch from bringing an older owner's window forward.
pub const PROTOCOL_VERSION: u32 = 1;

/// The longest request line the owner reads; a request is a few short fields.
pub const MAX_REQUEST_BYTES: u64 = 16 * 1024;

/// What a client asks, with its arguments under `args` (left out for a command without any).
/// `Activate` is the second launch asking the owner to show its window; `ctl` offers the others.
/// The owner refuses an argument it does not know rather than ignore it, and a command that
/// changes something changes nothing unless its `yes` is true.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum Command {
    Activate,
    Status,
    Accounts,
    Sessions,
    Usage,
    Start(StartArgs),
    Stop(StopArgs),
    Operation(OperationArgs),
    Cancel(CancelArgs),
}

impl Command {
    /// The commands that read and take no arguments, in the order `ctl`'s usage lists them.
    pub const READ: [Command; 4] = [
        Command::Status,
        Command::Accounts,
        Command::Sessions,
        Command::Usage,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Command::Activate => "activate",
            Command::Status => "status",
            Command::Accounts => "accounts",
            Command::Sessions => "sessions",
            Command::Usage => "usage",
            Command::Start(_) => "start",
            Command::Stop(_) => "stop",
            Command::Operation(_) => "operation",
            Command::Cancel(_) => "cancel",
        }
    }

    /// Whether the command can change something, so it needs the user's second permission.
    /// Its preview is included: it is the same command asked without `yes`.
    pub fn changes(&self) -> bool {
        matches!(
            self,
            Command::Start(_) | Command::Stop(_) | Command::Cancel(_)
        )
    }
}

/// Starts a runtime on the active account. Everything else comes from the Defaults, as the
/// New session dialog fills it in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartArgs {
    /// `None`: the Defaults' hardware.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accelerator: Option<Accelerator>,
    /// `None`: a name made up for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub yes: bool,
}

/// Stops a runtime on any saved account and ends its billing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopArgs {
    /// A session name or an endpoint, as `sessions` lists them.
    pub runtime: String,
    /// An account's email or id, for a name that more than one account has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default)]
    pub yes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationArgs {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelArgs {
    pub id: String,
    #[serde(default)]
    pub yes: bool,
}

/// One request line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub token: Token,
    /// The backend the client looks for; an owner of the other kind refuses.
    pub backend: Flavor,
    #[serde(flatten)]
    pub command: Command,
}

/// Stable machine codes of a failed request. The `message` beside one is for people and may
/// change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    /// `ctl` was called wrongly (exit code 2).
    InvalidArguments,
    /// No copy of the app owns this state on this backend.
    NotRunning,
    /// The owner's port did not answer as expected.
    ConnectionFailed,
    /// The owner's answer was not a response.
    BadResponse,
    /// The request did not carry the owner's token.
    Unauthorized,
    /// The request was not one the owner understands.
    InvalidRequest,
    /// The client speaks another protocol version (an update replaced one of the two).
    UnsupportedVersion,
    /// The owner runs on the other backend.
    WrongBackend,
    /// The user has not allowed control from the command line.
    ControlOff,
    /// The user allows reading but not starting and stopping runtimes.
    ChangesOff,
    /// The owner is on its way out (closing, or restarting into an update).
    Quitting,
    /// The owner did not answer in time.
    Timeout,
    /// What the command needs is still loading, or could not be loaded.
    NotReady,
    NoActiveAccount,
    /// An account is being added, switched or removed.
    AccountChanging,
    /// A runtime is being started already.
    AlreadyStarting,
    /// Someone is filling in the New session form in the window.
    FormOpen,
    /// The active account has as many runtimes as this app starts per account.
    SessionLimitReached,
    /// No listed runtime has that name or endpoint.
    NoSuchRuntime,
    /// More than one listed runtime has that name or endpoint.
    AmbiguousRuntime,
    AlreadyStopping,
    /// This run of the app has no operation with that id (it never had, forgot it, or the id
    /// is from before the app restarted).
    UnknownOperation,
    OperationEnded,
    /// A stop cannot be cancelled.
    NotCancellable,
}

/// Where an operation that `start` or `stop` began stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub code: Code,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// One response line. `backend` names the owner that answered; only an unauthorized request
/// is told nothing, not even that.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<Flavor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
}

impl Response {
    pub fn success(backend: Flavor, data: serde_json::Value) -> Self {
        Response {
            ok: true,
            backend: Some(backend),
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(backend: Option<Flavor>, code: Code, message: Option<String>) -> Self {
        Response {
            ok: false,
            backend,
            data: None,
            error: Some(Failure { code, message }),
        }
    }

    pub fn code(&self) -> Option<Code> {
        self.error.as_ref().map(|failure| failure.code)
    }
}

/// The owner's secret. Never printed, logged or put on a command line.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Token(String);

/// Bytes of randomness in a token.
const TOKEN_BYTES: usize = 32;

impl Token {
    /// A fresh token from the OS's random source.
    pub fn generate() -> std::io::Result<Token> {
        let mut bytes = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        Ok(Token(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    /// Compares in time that does not depend on where the two differ, so a local process
    /// cannot find the token byte by byte from how fast the owner refuses.
    pub fn matches(&self, other: &str) -> bool {
        let (ours, theirs) = (self.0.as_bytes(), other.as_bytes());
        ours.len() == theirs.len()
            && ours
                .iter()
                .zip(theirs)
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }

    #[cfg(test)]
    pub(crate) fn from_text(text: &str) -> Token {
        Token(text.to_owned())
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(…)")
    }
}

/// What the owner writes for its clients: where it listens and the token.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub token: Token,
    pub backend: Flavor,
    pub pid: u32,
    pub version: String,
}

/// The folder that holds the state an owner owns: the real backend's data folder (core keeps
/// its files there whatever the config folder is), or the mock's config folder.
/// `config_override` is the value of `settings::CONFIG_DIR_ENV`.
pub fn state_dir(flavor: Flavor, config_override: Option<OsString>) -> Option<PathBuf> {
    match flavor {
        Flavor::Real => default_local_data_dir(),
        Flavor::Mock => crate::settings::config_dir(config_override),
    }
}

fn kind_name(flavor: Flavor) -> &'static str {
    match flavor {
        Flavor::Real => "real",
        Flavor::Mock => "mock",
    }
}

/// The file whose OS lock marks the owner. It is never deleted: on unix a lock on a deleted
/// file would not stop a process that creates the name again.
pub fn lock_file(dir: &Path, flavor: Flavor) -> PathBuf {
    dir.join(format!("control-{}.lock", kind_name(flavor)))
}

/// The owner's [`Endpoint`]. Apart from the lock, because Windows keeps other handles from
/// reading a locked file.
pub fn endpoint_file(dir: &Path, flavor: Flavor) -> PathBuf {
    dir.join(format!("control-{}.json", kind_name(flavor)))
}

#[cfg(test)]
mod tests {
    use colab_playground_core::{backend::BackendKind as Flavor, sessions::Accelerator};

    use super::{
        Code, Command, Request, Response, StartArgs, StopArgs, Token, endpoint_file, lock_file,
    };

    #[test]
    fn a_request_and_a_response_round_trip_as_one_line() {
        let request = Request {
            v: super::PROTOCOL_VERSION,
            token: Token::from_text("abc"),
            backend: Flavor::Mock,
            command: Command::Sessions,
        };
        let line = serde_json::to_string(&request).expect("serialises");
        assert!(!line.contains('\n'));
        assert_eq!(
            line, r#"{"v":1,"token":"abc","backend":"mock","command":"sessions"}"#,
            "a read request is what an owner of the first version reads"
        );
        assert_eq!(
            serde_json::from_str::<Request>(&line).expect("parses"),
            request
        );

        let start = Request {
            command: Command::Start(StartArgs {
                accelerator: Some(Accelerator::T4),
                name: None,
                yes: true,
            }),
            ..request
        };
        let line = serde_json::to_string(&start).expect("serialises");
        assert_eq!(
            line,
            r#"{"v":1,"token":"abc","backend":"mock","command":"start","args":{"accelerator":"T4","yes":true}}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(&line).expect("parses"),
            start
        );
        let preview: Command =
            serde_json::from_str(r#"{"command":"stop","args":{"runtime":"t4-91c2"}}"#)
                .expect("parses");
        assert_eq!(
            preview,
            Command::Stop(StopArgs {
                runtime: "t4-91c2".into(),
                account: None,
                yes: false,
            }),
            "without yes it is a preview"
        );
        for refused in [
            r#"{"command":"stop","args":{"runtime":"t4-91c2","force":true}}"#,
            r#"{"command":"stop"}"#,
            r#"{"command":"start","args":{"yes":"true"}}"#,
            r#"{"command":"status","args":{"account":"lin"}}"#,
        ] {
            assert!(
                serde_json::from_str::<Command>(refused).is_err(),
                "{refused} is refused, not read as something else"
            );
        }

        let response = Response::failure(Some(Flavor::Real), Code::ControlOff, None);
        let line = serde_json::to_string(&response).expect("serialises");
        assert_eq!(
            line,
            r#"{"ok":false,"backend":"real","error":{"code":"control_off"}}"#
        );
        assert_eq!(
            serde_json::from_str::<Response>(&line).expect("parses"),
            response
        );
    }

    #[test]
    fn tokens_are_fresh_hex_and_never_shown() {
        let one = Token::generate().expect("random bytes");
        let two = Token::generate().expect("random bytes");
        assert_ne!(one, two);
        assert_eq!(one.0.len(), super::TOKEN_BYTES * 2);
        assert!(one.0.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(format!("{one:?}"), "Token(…)");
        assert!(one.matches(&one.0.clone()));
        assert!(!one.matches(&two.0));
        assert!(!one.matches(""));
        assert!(!one.matches(&one.0[1..]));
    }

    #[test]
    fn each_backend_has_its_own_files() {
        let dir = std::path::Path::new("state");
        assert_ne!(lock_file(dir, Flavor::Mock), lock_file(dir, Flavor::Real));
        assert_ne!(
            endpoint_file(dir, Flavor::Mock),
            endpoint_file(dir, Flavor::Real)
        );
        assert_ne!(
            lock_file(dir, Flavor::Mock),
            endpoint_file(dir, Flavor::Mock)
        );
    }
}
