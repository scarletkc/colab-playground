//! `colab-playground ctl`: asks the copy that owns the state and prints its answer. It never
//! builds a backend, starts a process or opens a window, so it cannot become an owner.

use std::{
    ffi::OsString,
    fmt, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use colab_playground_core::{
    backend::BackendKind as Flavor,
    sessions::{Accelerator, validate_session_name},
};

use super::{
    CancelArgs, Code, Command, Endpoint, OperationArgs, OperationState, PROTOCOL_VERSION, Request,
    Response, StartArgs, StopArgs, endpoint_file, kind_name, owner::ANSWER_TIMEOUT, state_dir,
};
use crate::backend::{BackendError, BackendKind, Build};

/// Windows retries a refused loopback connection for about two seconds before it reports the
/// refusal; a live owner's port completes the connection at once.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The owner answers within its own wait for the foreground; this leaves room for the trip.
const REPLY_TIMEOUT: Duration = Duration::from_secs(ANSWER_TIMEOUT.as_secs() + 5);
/// Far above any answer (a few hundred runtimes), so only a peer that is not an owner hits it.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
/// How often `operation --wait` asks again. Waiting is a series of short requests rather than
/// one held open: the owner answers one connection at a time, and a client killed while it
/// waits then leaves nothing behind.
const WAIT_POLL: Duration = Duration::from_secs(1);

/// The commands `ctl` offers, in the order its usage lists them.
const COMMANDS: [&str; 8] = [
    "status",
    "accounts",
    "sessions",
    "usage",
    "start",
    "stop",
    "operation",
    "cancel",
];
const RUNTIME: &str = "a runtime: its session name or endpoint, as `sessions` lists them";
const OPERATION_ID: &str = "the operation's id, as start or stop answered it";
const NAME_RULE: &str =
    "a session name: 1 to 63 letters, digits, '.', '_' or '-', starting with a letter or digit";

/// A checked `ctl` command line.
#[derive(Clone, Debug, PartialEq)]
pub struct CtlArgs {
    pub command: Command,
    pub flavor: Flavor,
    /// `operation --wait`: how long to wait for the operation to end.
    pub wait: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtlError {
    UnknownArgument(String),
    MissingCommand,
    TwoCommands(&'static str, &'static str),
    /// A flag without its value.
    MissingValue(&'static str),
    InvalidValue {
        flag: &'static str,
        value: String,
        expected: String,
    },
    /// A flag the command does not take.
    NotForCommand {
        flag: &'static str,
        command: &'static str,
    },
    /// `stop`, `operation` or `cancel` without what it acts on.
    MissingTarget {
        command: &'static str,
        what: &'static str,
    },
    /// Both `--mock` and `--real`, or a bad `COLAB_PLAYGROUND_BACKEND`.
    Backend(BackendError),
}

impl fmt::Display for CtlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CtlError::UnknownArgument(arg) => write!(f, "unknown argument {arg:?}"),
            CtlError::MissingCommand => write!(f, "name a command: {}", COMMANDS.join(", ")),
            CtlError::TwoCommands(first, second) => {
                write!(f, "give one command, not both {first} and {second}")
            }
            CtlError::MissingValue(flag) => write!(f, "{flag} needs a value"),
            CtlError::InvalidValue {
                flag,
                value,
                expected,
            } => write!(f, "{flag} {value:?} is not {expected}"),
            CtlError::NotForCommand { flag, command } => {
                write!(f, "{command} does not take {flag}")
            }
            CtlError::MissingTarget { command, what } => write!(f, "{command} needs {what}"),
            CtlError::Backend(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CtlError {}

/// The hardware `--hardware` takes, as the CLI names it (`CPU` for none).
fn hardware_choices(separator: &str) -> String {
    Accelerator::ALL
        .map(|accelerator| match accelerator {
            Accelerator::None => "CPU",
            other => other.as_str(),
        })
        .join(separator)
}

/// How to call `ctl`, printed with any argument error and for `--help`.
pub fn usage() -> String {
    // A build without demo data has no copy on it to ask.
    let backends = if Build::CURRENT.mock {
        "[--real | --mock]"
    } else {
        "[--real]"
    };
    let hardware = hardware_choices("|");
    format!(
        "usage: colab-playground ctl <command> {backends}
  status | accounts | sessions | usage
  start [--hardware <{hardware}>] [--name <name>] [--yes]
  stop <runtime> [--account <email>] [--yes]
  operation <id> [--wait <seconds>]
  cancel <id> [--yes]
Prints the running Colab Playground's answer as one line of JSON. It must allow control from
the command line (Settings); start, stop and cancel need it to allow starting and stopping
runtimes too. Without --yes those three change nothing and answer with what they would do.
<runtime> is a session name or endpoint as `sessions` lists them. start and stop answer with
an operation <id>; a wait that runs out leaves the operation running."
    )
}

/// The value of `flag`: after `=`, or the next argument unless that is a flag too.
fn value_of<'a>(
    flag: &'static str,
    inline: Option<&'a str>,
    rest: &mut impl Iterator<Item = &'a str>,
) -> Result<&'a str, CtlError> {
    match inline {
        Some(value) => Ok(value),
        None => rest
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or(CtlError::MissingValue(flag)),
    }
}

/// Checks every argument first: `Ok(None)` asks for the usage, which needs no backend, so
/// `--help` answers in every build. The backend is chosen as the app chooses it for `build`
/// (flag, then `env`, then the build default), from the arguments read as flags only (a value
/// such as `--name --real` is refused, never taken for the flag), so a `ctl` of a debug build
/// without `--real` can never reach an owner on the real backend.
pub fn parse_args<S: AsRef<str>>(
    args: &[S],
    env: Option<&str>,
    build: Build,
) -> Result<Option<CtlArgs>, CtlError> {
    let mut words: Vec<&str> = Vec::new();
    let mut backends: Vec<&str> = Vec::new();
    let mut given: Vec<&'static str> = Vec::new();
    let (mut help, mut yes) = (false, false);
    let (mut hardware, mut name, mut account, mut wait) = (None, None, None, None);
    let mut rest = args.iter().map(AsRef::as_ref);
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (arg, None),
        };
        match (flag, inline) {
            ("--mock", None) => backends.push("--mock"),
            ("--real", None) => backends.push("--real"),
            ("--help" | "-h", None) => help = true,
            ("--yes", None) => {
                yes = true;
                given.push("--yes");
            }
            ("--hardware", _) => {
                hardware = Some(value_of("--hardware", inline, &mut rest)?);
                given.push("--hardware");
            }
            ("--name", _) => {
                name = Some(value_of("--name", inline, &mut rest)?);
                given.push("--name");
            }
            ("--account", _) => {
                account = Some(value_of("--account", inline, &mut rest)?);
                given.push("--account");
            }
            ("--wait", _) => {
                wait = Some(value_of("--wait", inline, &mut rest)?);
                given.push("--wait");
            }
            _ if arg.starts_with('-') => return Err(CtlError::UnknownArgument(arg.to_owned())),
            _ => words.push(arg),
        }
    }
    let command = match words.first() {
        None => None,
        Some(word) => Some(
            *COMMANDS
                .iter()
                .find(|known| *known == word)
                .ok_or_else(|| CtlError::UnknownArgument((*word).to_owned()))?,
        ),
    };
    let (takes, target): (&[&str], Option<&'static str>) = match command {
        Some("start") => (&["--yes", "--hardware", "--name"], None),
        Some("stop") => (&["--yes", "--account"], Some(RUNTIME)),
        Some("operation") => (&["--wait"], Some(OPERATION_ID)),
        Some("cancel") => (&["--yes"], Some(OPERATION_ID)),
        _ => (&[], None),
    };
    if let Some(command) = command {
        if let Some(flag) = given.iter().find(|flag| !takes.contains(flag)) {
            return Err(CtlError::NotForCommand { flag, command });
        }
        let extra = words.get(if target.is_some() { 2 } else { 1 });
        if let Some(extra) = extra {
            return Err(match COMMANDS.iter().find(|known| *known == extra) {
                Some(second) => CtlError::TwoCommands(command, second),
                None => CtlError::UnknownArgument((*extra).to_owned()),
            });
        }
    }
    let accelerator = hardware
        .map(|value| {
            Accelerator::from_label(value).ok_or_else(|| CtlError::InvalidValue {
                flag: "--hardware",
                value: value.to_owned(),
                expected: format!("one of {}", hardware_choices(", ")),
            })
        })
        .transpose()?;
    if let Some(value) = name
        && validate_session_name(value).is_err()
    {
        return Err(CtlError::InvalidValue {
            flag: "--name",
            value: value.to_owned(),
            expected: NAME_RULE.to_owned(),
        });
    }
    let wait = wait
        .map(|value| {
            value
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
                .ok_or_else(|| CtlError::InvalidValue {
                    flag: "--wait",
                    value: value.to_owned(),
                    expected: "a number of seconds".to_owned(),
                })
        })
        .transpose()?;
    if help {
        return Ok(None);
    }
    let command = command.ok_or(CtlError::MissingCommand)?;
    let target = match target {
        Some(what) => Some(
            words
                .get(1)
                .map(|word| (*word).to_owned())
                .ok_or(CtlError::MissingTarget { command, what })?,
        ),
        None => None,
    };
    let command = match (command, target) {
        ("start", _) => Command::Start(StartArgs {
            accelerator,
            name: name.map(str::to_owned),
            yes,
        }),
        ("stop", Some(runtime)) => Command::Stop(StopArgs {
            runtime,
            account: account.map(str::to_owned),
            yes,
        }),
        ("operation", Some(id)) => Command::Operation(OperationArgs { id }),
        ("cancel", Some(id)) => Command::Cancel(CancelArgs { id, yes }),
        (read, _) => Command::READ
            .into_iter()
            .find(|known| known.name() == read)
            .expect("every other command reads and takes no arguments"),
    };
    let kind = BackendKind::resolve(&backends, env, build).map_err(CtlError::Backend)?;
    Ok(Some(CtlArgs {
        command,
        flavor: kind.flavor(),
        wait,
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
        Some(dir) => {
            let ask = || request(&dir, parsed.flavor, parsed.command.clone());
            match parsed.wait {
                Some(wait) => wait_for(ask, wait, WAIT_POLL, thread::sleep),
                None => ask(),
            }
        }
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

/// Asks with `ask` until the operation in the answer has ended, the answer is not one about a
/// running operation (a refusal, an unknown id), or `wait` has passed, sleeping `poll` between
/// asks. Returns the last answer: an operation still running when the wait runs out is answered
/// as it is now, which is not a failure, and nothing is cancelled.
fn wait_for(
    mut ask: impl FnMut() -> Result<serde_json::Value, Response>,
    wait: Duration,
    poll: Duration,
    mut sleep: impl FnMut(Duration),
) -> Result<serde_json::Value, Response> {
    let running = serde_json::to_value(OperationState::Running).ok();
    // A wait too long for the clock to reach has no end.
    let deadline = Instant::now().checked_add(wait);
    loop {
        let answer = ask()?;
        let left = deadline.map_or(Duration::MAX, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        if answer.pointer("/data/operation/state") != running.as_ref() || left.is_zero() {
            return Ok(answer);
        }
        sleep(poll.min(left));
    }
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
    use std::{
        cell::Cell,
        time::{Duration, Instant},
    };

    use colab_playground_core::{backend::BackendKind as Flavor, sessions::Accelerator};

    use super::{CtlArgs, CtlError, parse_args, request, run, wait_for};
    use crate::{
        backend::{BackendError, Build},
        control::{
            CancelArgs, Code, Command, OperationArgs, Response, StartArgs, StopArgs, listen,
        },
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
    /// A debug build without demo data: it starts only with `--real`.
    const DEBUG_WITHOUT_DEMO: Build = Build {
        mock: false,
        debug: true,
    };

    fn parsed(args: &[&str]) -> Result<Option<CtlArgs>, CtlError> {
        parse_args(args, None, DEBUG)
    }

    fn command(args: &[&str]) -> Command {
        parsed(args)
            .expect("a good command line")
            .expect("not --help")
            .command
    }

    #[test]
    fn every_argument_is_checked_before_anything_happens() {
        assert_eq!(
            parse_args(&["status", "--mock"], None, DEBUG),
            Ok(Some(CtlArgs {
                command: Command::Status,
                flavor: Flavor::Mock,
                wait: None,
            }))
        );
        assert_eq!(
            parse_args(&["--real", "sessions"], Some("mock"), DEBUG),
            Ok(Some(CtlArgs {
                command: Command::Sessions,
                flavor: Flavor::Real,
                wait: None,
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
    fn help_needs_no_backend_in_any_build() {
        for build in [DEBUG, DOWNLOAD, DEBUG_WITHOUT_DEMO] {
            assert_eq!(parse_args(&["--help"], None, build), Ok(None), "{build:?}");
            assert_eq!(
                parse_args(&["-h", "stop"], None, build),
                Ok(None),
                "{build:?}"
            );
        }
        assert_eq!(
            parse_args(&["status"], None, DEBUG_WITHOUT_DEMO),
            Err(CtlError::Backend(BackendError::NoDefault)),
            "a command still needs a backend it may ask"
        );
        assert_eq!(
            parse_args(&["--help", "--bogus"], None, DEBUG_WITHOUT_DEMO),
            Err(CtlError::UnknownArgument("--bogus".into())),
            "an unknown argument is still an error"
        );
    }

    #[test]
    fn the_changing_commands_take_their_own_arguments() {
        assert_eq!(
            command(&["start"]),
            Command::Start(StartArgs::default()),
            "a preview of the Defaults' hardware under a made-up name"
        );
        assert_eq!(
            command(&["start", "--hardware", "t4", "--name=train-1", "--yes"]),
            Command::Start(StartArgs {
                accelerator: Some(Accelerator::T4),
                name: Some("train-1".into()),
                yes: true,
            })
        );
        assert_eq!(
            command(&["start", "--hardware=cpu"]),
            Command::Start(StartArgs {
                accelerator: Some(Accelerator::None),
                ..StartArgs::default()
            })
        );
        assert_eq!(
            command(&[
                "stop",
                "m-mock-1f2e",
                "--account",
                "lin@example.com",
                "--yes"
            ]),
            Command::Stop(StopArgs {
                runtime: "m-mock-1f2e".into(),
                account: Some("lin@example.com".into()),
                yes: true,
            })
        );
        assert_eq!(
            command(&["cancel", "1a2b3c4d-1"]),
            Command::Cancel(CancelArgs {
                id: "1a2b3c4d-1".into(),
                yes: false,
            })
        );
        let waiting = parsed(&["operation", "1a2b3c4d-1", "--wait", "2.5"])
            .expect("a good command line")
            .expect("not --help");
        assert_eq!(
            waiting.command,
            Command::Operation(OperationArgs {
                id: "1a2b3c4d-1".into()
            })
        );
        assert_eq!(waiting.wait, Some(Duration::from_millis(2500)));

        let invalid = |flag: &'static str, value: &str| matches!(parsed(&["start", flag, value]), Err(CtlError::InvalidValue { flag: f, .. }) if f == flag);
        assert!(invalid("--hardware", "B200"));
        assert!(invalid("--name", "-dash-first"));
        assert!(invalid("--name", "has space"));
        assert!(matches!(
            parsed(&["operation", "x", "--wait", "-1"]),
            Err(CtlError::InvalidValue { flag: "--wait", .. })
        ));
        assert_eq!(
            parsed(&["start", "--name", "--real"]),
            Err(CtlError::MissingValue("--name")),
            "a flag is never taken for a value, so it cannot pick the backend either"
        );
        assert_eq!(
            parsed(&["start", "--name"]),
            Err(CtlError::MissingValue("--name"))
        );
        assert_eq!(
            parsed(&["status", "--yes"]),
            Err(CtlError::NotForCommand {
                flag: "--yes",
                command: "status"
            })
        );
        assert_eq!(
            parsed(&["operation", "x", "--yes"]),
            Err(CtlError::NotForCommand {
                flag: "--yes",
                command: "operation"
            }),
            "reading an operation changes nothing"
        );
        assert_eq!(
            parsed(&["start", "--wait", "5"]),
            Err(CtlError::NotForCommand {
                flag: "--wait",
                command: "start"
            })
        );
        assert!(matches!(
            parsed(&["stop", "--yes"]),
            Err(CtlError::MissingTarget {
                command: "stop",
                ..
            })
        ));
        assert_eq!(
            parsed(&["stop", "a100-7f3a", "t4-91c2"]),
            Err(CtlError::UnknownArgument("t4-91c2".into())),
            "one runtime at a time"
        );
        assert_eq!(
            parsed(&["--yes=1", "start"]),
            Err(CtlError::UnknownArgument("--yes=1".into()))
        );
    }

    /// Answers like the owner about an operation, in turn from `states`, the last one for good.
    fn owner_answer(states: &[&str], asked: &Cell<usize>) -> serde_json::Value {
        let state = states[asked.get().min(states.len() - 1)];
        asked.set(asked.get() + 1);
        serde_json::json!({
            "ok": true,
            "backend": "mock",
            "data": { "operation": { "id": "x-1", "state": state } }
        })
    }

    #[test]
    fn a_wait_ends_with_the_operation() {
        let asked = Cell::new(0);
        let slept = Cell::new(Duration::ZERO);
        let answer = wait_for(
            || Ok(owner_answer(&["running", "running", "succeeded"], &asked)),
            Duration::from_secs(60),
            Duration::from_millis(500),
            |by| slept.set(slept.get() + by),
        )
        .expect("answered");
        assert_eq!(answer["data"]["operation"]["state"], "succeeded");
        assert_eq!(asked.get(), 3);
        assert_eq!(slept.get(), Duration::from_secs(1));

        let asked = Cell::new(0);
        let endless = wait_for(
            || Ok(owner_answer(&["running", "cancelled"], &asked)),
            Duration::MAX,
            Duration::from_millis(500),
            |_| {},
        )
        .expect("answered");
        assert_eq!(
            endless["data"]["operation"]["state"], "cancelled",
            "a wait longer than the clock reaches waits for the end"
        );

        let refused = wait_for(
            || {
                Err(Response::failure(
                    Some(Flavor::Mock),
                    Code::UnknownOperation,
                    None,
                ))
            },
            Duration::from_secs(60),
            Duration::from_millis(500),
            |_| panic!("an unknown id is not waited for"),
        )
        .expect_err("refused");
        assert_eq!(refused.code(), Some(Code::UnknownOperation));
    }

    #[test]
    fn a_wait_that_runs_out_answers_with_the_operation_still_running() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = asked.clone();
        listen(dir.path(), Flavor::Mock, move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Response::success(
                Flavor::Mock,
                serde_json::json!({ "operation": { "id": "x-1", "state": "running" } }),
            )
        })
        .expect("listens");
        let wait = Duration::from_millis(300);
        let started = Instant::now();
        let ask = || {
            request(
                dir.path(),
                Flavor::Mock,
                Command::Operation(OperationArgs { id: "x-1".into() }),
            )
        };
        let answer = wait_for(ask, wait, Duration::from_millis(50), std::thread::sleep)
            .expect("an answer, not a failure");
        assert!(started.elapsed() >= wait, "{:?}", started.elapsed());
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["data"]["operation"]["state"], "running");
        assert!(
            asked.load(std::sync::atomic::Ordering::SeqCst) > 1,
            "asked again while it waited, one short request at a time"
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
