//! `colab-playground ctl`: asks the copy that owns the state and prints its answer. It never
//! builds a backend, starts a process or opens a window, so it cannot become an owner.

use std::{
    ffi::OsString,
    fmt, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    time::Duration,
};

use colab_playground_core::backend::BackendKind as Flavor;

use super::{
    Code, Command, Endpoint, PROTOCOL_VERSION, Request, Response, endpoint_file, kind_name,
    owner::ANSWER_TIMEOUT, state_dir,
};
use crate::backend::{BackendError, BackendKind, Build};

/// Windows retries a refused loopback connection for about two seconds before it reports the
/// refusal; a live owner's port completes the connection at once.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The owner answers within its own wait for the foreground; this leaves room for the trip.
const REPLY_TIMEOUT: Duration = Duration::from_secs(ANSWER_TIMEOUT.as_secs() + 5);
/// Far above any answer (a few hundred runtimes), so only a peer that is not an owner hits it.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// A checked `ctl` command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CtlArgs {
    pub command: Command,
    pub flavor: Flavor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtlError {
    UnknownArgument(String),
    MissingCommand,
    TwoCommands(&'static str, &'static str),
    /// Both `--mock` and `--real`, or a bad `COLAB_PLAYGROUND_BACKEND`.
    Backend(BackendError),
}

impl fmt::Display for CtlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CtlError::UnknownArgument(arg) => write!(f, "unknown argument {arg:?}"),
            CtlError::MissingCommand => write!(f, "name a command: {}", command_names(", ")),
            CtlError::TwoCommands(first, second) => {
                write!(f, "give one command, not both {first} and {second}")
            }
            CtlError::Backend(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CtlError {}

fn command_names(separator: &str) -> String {
    Command::READ.map(Command::name).join(separator)
}

/// How to call `ctl`, printed with any argument error and for `--help`.
pub fn usage() -> String {
    // A build without demo data has no copy on it to ask.
    let backends = if Build::CURRENT.mock {
        "[--real | --mock]"
    } else {
        "[--real]"
    };
    format!(
        "usage: colab-playground ctl <{}> {backends}\n  Prints the running Colab Playground's answer as one line of JSON. It must allow control\n  from the command line (Settings).",
        command_names("|")
    )
}

/// Checks every argument first: `Ok(None)` asks for the usage. The backend is chosen as the
/// app chooses it for `build` (flag, then `env`, then the build default), so a `ctl` of a debug
/// build without `--real` can never reach an owner on the real backend.
pub fn parse_args<S: AsRef<str>>(
    args: &[S],
    env: Option<&str>,
    build: Build,
) -> Result<Option<CtlArgs>, CtlError> {
    let mut command: Option<Command> = None;
    let mut help = false;
    for arg in args.iter().map(AsRef::as_ref) {
        match arg {
            "--mock" | "--real" => {}
            "--help" | "-h" => help = true,
            _ => {
                let Some(&named) = Command::READ.iter().find(|known| known.name() == arg) else {
                    return Err(CtlError::UnknownArgument(arg.to_owned()));
                };
                if let Some(first) = command {
                    return Err(CtlError::TwoCommands(first.name(), named.name()));
                }
                command = Some(named);
            }
        }
    }
    let kind = BackendKind::resolve(args, env, build).map_err(CtlError::Backend)?;
    if help {
        return Ok(None);
    }
    let command = command.ok_or(CtlError::MissingCommand)?;
    Ok(Some(CtlArgs {
        command,
        flavor: kind.flavor(),
    }))
}

/// What `ctl` prints, and its exit code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// One line of JSON, or the usage for `--help`.
    pub stdout: String,
    pub stderr: Option<String>,
    pub code: i32,
}

/// Exit code of a request the owner (or the lack of one) refused.
const FAILED: i32 = 1;
/// Exit code of a command line that was not checked out, as for the app itself.
const MISUSED: i32 = 2;

/// Runs `ctl` with `args` (after `ctl`). `backend_env` and `config_override` are the values of
/// `COLAB_PLAYGROUND_BACKEND` and `COLAB_PLAYGROUND_CONFIG_DIR`.
pub fn run<S: AsRef<str>>(
    args: &[S],
    backend_env: Option<&str>,
    config_override: Option<OsString>,
) -> Outcome {
    let parsed = match parse_args(args, backend_env, Build::CURRENT) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            return Outcome {
                stdout: usage(),
                stderr: None,
                code: 0,
            };
        }
        Err(error) => {
            let response = Response::failure(None, Code::InvalidArguments, Some(error.to_string()));
            return Outcome {
                stdout: line(&serde_json::to_value(response).unwrap_or_default()),
                stderr: Some(format!("colab-playground ctl: {error}\n{}", usage())),
                code: MISUSED,
            };
        }
    };
    let answer = match state_dir(parsed.flavor, config_override) {
        Some(dir) => request(&dir, parsed.flavor, parsed.command),
        None => Err(Response::failure(
            Some(parsed.flavor),
            Code::NotRunning,
            Some("no folder for this backend's state was found".to_owned()),
        )),
    };
    let answer = answer.unwrap_or_else(|failure| serde_json::to_value(failure).unwrap_or_default());
    let ok = answer.get("ok") == Some(&serde_json::Value::Bool(true));
    Outcome {
        stdout: line(&answer),
        stderr: None,
        code: if ok { 0 } else { FAILED },
    }
}

fn line(value: &serde_json::Value) -> String {
    value.to_string()
}

/// Sends `command` to the owner of `dir`'s state on `flavor` and returns its answer, a JSON
/// object with `ok` and `backend` (added when the owner left it out). The error is the failure
/// to report instead.
pub(crate) fn request(
    dir: &Path,
    flavor: Flavor,
    command: Command,
) -> Result<serde_json::Value, Response> {
    let fail = |code, message: String| Response::failure(Some(flavor), code, Some(message));
    let file = endpoint_file(dir, flavor);
    let not_running = || {
        fail(
            Code::NotRunning,
            format!(
                "no Colab Playground on the {} backend owns {}",
                kind_name(flavor),
                dir.display()
            ),
        )
    };
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(not_running()),
        Err(error) => {
            return Err(fail(
                Code::ConnectionFailed,
                format!("{}: {error}", file.display()),
            ));
        }
    };
    let endpoint: Endpoint = serde_json::from_str(&text).map_err(|error| {
        fail(
            Code::ConnectionFailed,
            format!("{}: {error}", file.display()),
        )
    })?;
    if endpoint.backend != flavor {
        return Err(not_running());
    }
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|error| {
        // Nobody listens on the port: the owner that wrote the file ended without removing it.
        // (The kernel completes a loopback connection to a listener before it is accepted, so
        // a live owner does not time out here.)
        match error.kind() {
            io::ErrorKind::ConnectionRefused | io::ErrorKind::TimedOut => not_running(),
            _ => fail(Code::ConnectionFailed, error.to_string()),
        }
    })?;
    let exchange = || -> io::Result<String> {
        stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
        let request = Request {
            v: PROTOCOL_VERSION,
            token: endpoint.token.clone(),
            backend: flavor,
            command,
        };
        let mut text = serde_json::to_string(&request)?;
        text.push('\n');
        let mut writer = &stream;
        writer.write_all(text.as_bytes())?;
        writer.flush()?;
        let mut answer = String::new();
        BufReader::new((&stream).take(MAX_RESPONSE_BYTES)).read_line(&mut answer)?;
        Ok(answer)
    };
    let answer = exchange().map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock || error.kind() == io::ErrorKind::TimedOut {
            fail(Code::Timeout, error.to_string())
        } else {
            fail(Code::ConnectionFailed, error.to_string())
        }
    })?;
    let bad = || {
        fail(
            Code::BadResponse,
            format!("port {} did not answer as Colab Playground", endpoint.port),
        )
    };
    let mut answer: serde_json::Value = serde_json::from_str(answer.trim()).map_err(|_| bad())?;
    let object = answer.as_object_mut().ok_or_else(bad)?;
    if !object.get("ok").is_some_and(serde_json::Value::is_boolean) {
        return Err(bad());
    }
    object
        .entry("backend")
        .or_insert_with(|| serde_json::json!(flavor));
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use colab_playground_core::backend::BackendKind as Flavor;

    use super::{CtlArgs, CtlError, parse_args, run};
    use crate::{
        backend::{BackendError, Build},
        control::Command,
    };

    const DEBUG: Build = Build {
        mock: true,
        debug: true,
    };
    /// A release download: no demo data.
    const DOWNLOAD: Build = Build {
        mock: false,
        debug: false,
    };

    #[test]
    fn every_argument_is_checked_before_anything_happens() {
        assert_eq!(
            parse_args(&["status", "--mock"], None, DEBUG),
            Ok(Some(CtlArgs {
                command: Command::Status,
                flavor: Flavor::Mock,
            }))
        );
        assert_eq!(
            parse_args(&["--real", "sessions"], Some("mock"), DEBUG),
            Ok(Some(CtlArgs {
                command: Command::Sessions,
                flavor: Flavor::Real,
            }))
        );
        assert_eq!(
            parse_args(&["usage"], Some("real"), DEBUG).map(|args| args.map(|a| a.flavor)),
            Ok(Some(Flavor::Real))
        );
        assert_eq!(
            parse_args(&["accounts"], None, DEBUG).map(|args| args.map(|a| a.flavor)),
            Ok(Some(Flavor::Mock)),
            "without a flag or the variable, the build's own default"
        );
        assert_eq!(
            parse_args(&["accounts"], None, DOWNLOAD).map(|args| args.map(|a| a.flavor)),
            Ok(Some(Flavor::Real))
        );
        assert_eq!(
            parse_args(&["status", "--mock"], None, DOWNLOAD),
            Err(CtlError::Backend(BackendError::NoDemoData("--mock".into()))),
            "a build without demo data has no copy on it to ask"
        );
        assert_eq!(
            parse_args(&["status", "--mok"], None, DEBUG),
            Err(CtlError::UnknownArgument("--mok".into())),
            "a typo of --mock is not ignored"
        );
        assert_eq!(
            parse_args(&["activate"], None, DEBUG),
            Err(CtlError::UnknownArgument("activate".into())),
            "bringing the window forward is the second launch's, not ctl's"
        );
        assert_eq!(
            parse_args(
                &["--mock", "--mock-scenario", "single", "status"],
                None,
                DEBUG
            ),
            Err(CtlError::UnknownArgument("--mock-scenario".into())),
            "ctl starts no simulation"
        );
        assert_eq!(
            parse_args(&["status", "usage"], None, DEBUG),
            Err(CtlError::TwoCommands("status", "usage"))
        );
        assert_eq!(
            parse_args(&["--mock"], None, DEBUG),
            Err(CtlError::MissingCommand)
        );
        assert_eq!(
            parse_args(&["status", "--mock", "--real"], None, DEBUG),
            Err(CtlError::Backend(BackendError::ConflictingFlags))
        );
        assert_eq!(
            parse_args(&["status"], Some("fake"), DEBUG),
            Err(CtlError::Backend(BackendError::InvalidEnv("fake".into())))
        );
        assert_eq!(parse_args(&["--help"], None, DEBUG), Ok(None));
        assert_eq!(
            parse_args(&["--help", "--bogus"], None, DEBUG),
            Err(CtlError::UnknownArgument("--bogus".into()))
        );
    }

    #[test]
    fn a_bad_command_line_exits_2_with_the_usage() {
        let outcome = run(&["stauts", "--mock"], None, None);
        assert_eq!(outcome.code, 2);
        let stderr = outcome.stderr.expect("the usage");
        assert!(stderr.contains("unknown argument \"stauts\""), "{stderr}");
        assert!(stderr.contains("usage: colab-playground ctl"), "{stderr}");
        let json: serde_json::Value = serde_json::from_str(&outcome.stdout).expect("JSON");
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "invalid_arguments");

        let help = run(&["--help"], None, None);
        assert_eq!(help.code, 0);
        assert!(help.stdout.starts_with("usage: colab-playground ctl"));
    }

    // On demo data, which only a build with it can ask for: never the real backend's folder.
    #[cfg(feature = "mock")]
    #[test]
    fn with_no_owner_ctl_says_so_and_fails() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let outcome = run(
            &["status", "--mock"],
            None,
            Some(dir.path().as_os_str().to_owned()),
        );
        assert_eq!(outcome.code, 1);
        assert_eq!(outcome.stderr, None);
        let json: serde_json::Value = serde_json::from_str(&outcome.stdout).expect("JSON");
        assert_eq!(json["ok"], false);
        assert_eq!(json["backend"], "mock");
        assert_eq!(json["error"]["code"], "not_running");
        assert!(
            !dir.path().join("control-mock.lock").exists(),
            "ctl takes no lock"
        );
    }
}
