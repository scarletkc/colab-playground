//! Control from the command line: `colab-playground ctl <command>` asks the copy of the app
//! that owns the state, so a script or an AI agent sees what the window shows.
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
//! line each way, one request per connection.

mod ctl;
mod owner;

use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

use colab_playground_core::{backend::BackendKind as Flavor, paths::default_local_data_dir};
use serde::{Deserialize, Serialize};

pub use ctl::{CtlArgs, CtlError, Outcome, parse_args, run as run_ctl, usage as ctl_usage};
pub use owner::{Claim, Owner, Service, answer, claim, claim_within, listen, mark_quitting, serve};

/// The first argument that makes the program a client instead of the app. The same program,
/// because Windows ships one exe that self-update replaces whole.
pub const CTL_COMMAND: &str = "ctl";

/// Bumped when a request or a response changes in a way an older copy would misread.
pub const PROTOCOL_VERSION: u32 = 1;

/// The longest request line the owner reads; a request is a few short fields.
pub const MAX_REQUEST_BYTES: u64 = 16 * 1024;

/// What a client asks. `Activate` is the second launch asking the owner to show its window;
/// `ctl` offers only the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Activate,
    Status,
    Accounts,
    Sessions,
    Usage,
}

impl Command {
    /// The commands `ctl` offers, in the order its usage lists them.
    pub const READ: [Command; 4] = [
        Command::Status,
        Command::Accounts,
        Command::Sessions,
        Command::Usage,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::Activate => "activate",
            Command::Status => "status",
            Command::Accounts => "accounts",
            Command::Sessions => "sessions",
            Command::Usage => "usage",
        }
    }
}

/// One request line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub token: Token,
    /// The backend the client looks for; an owner of the other kind refuses.
    pub backend: Flavor,
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
    /// The owner is on its way out (closing, or restarting into an update).
    Quitting,
    /// The owner did not answer in time.
    Timeout,
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
    use colab_playground_core::backend::BackendKind as Flavor;

    use super::{Code, Command, Request, Response, Token, endpoint_file, lock_file};

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
            line,
            r#"{"v":1,"token":"abc","backend":"mock","command":"sessions"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(&line).expect("parses"),
            request
        );

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
