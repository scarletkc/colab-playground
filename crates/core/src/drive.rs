//! Connecting the user's Google Drive to a runtime, for a plugin that asks for it
//! (`needs = ["drive"]`).
//!
//! `colab drivemount` runs under `assets/drive_mount.py`, which turns the CLI's terminal prompt
//! into lines read here and answered when the user says they allowed access. Only a look at the
//! runtime ([`check_snippet`]) says whether Drive is connected: the CLI exits 0 whether or not
//! the mount worked.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};

use crate::accounts::AccountId;
use crate::cli::{AUTH_PROMPT_MARKER, Cli, RESULT_MARKER, classify_failure};
use crate::clock::CancelToken;
use crate::error::{Error, ErrorKind, Result};
use crate::host::{HostChild, ScriptOutput, ScriptRequest, Stream, tail};
use crate::paths::HostPath;
use crate::sessions::validate_session_name;

/// Where Drive is mounted on the runtime. A constant, never a manifest's or an input's value:
/// the CLI writes it into the Python it runs.
pub const MOUNT_POINT: &str = "/content/drive";

/// The folder holding the user's files, under [`MOUNT_POINT`], in the order they are looked for.
const FOLDERS: [&str; 2] = ["MyDrive", "My Drive"];

/// Runs `colab drivemount` with the app answering its prompt.
const DRIVE_MOUNT_PY: &str = include_str!("../assets/drive_mount.py");

/// What [`DRIVE_MOUNT_PY`] prints for the app.
const CONSENT: &str = "[colab-playground] drive consent:";
const NOT_HANDED_OVER: &str = "[colab-playground] drive consent not handed over";
const CANCELLED: &str = "[colab-playground] drive mount cancelled";
const FAILED: &str = "[colab-playground] drive consent failed:";
const UNSUPPORTED: &str = "[colab-playground] unsupported colab CLI";

/// How long a run waits for the user to say they allowed access. Colab waits two minutes for
/// the consent and then mounts Drive without it, which a second try puts right; the CLI gives
/// up on the whole mount after ten minutes.
pub const CONSENT_WAIT: Duration = Duration::from_secs(9 * 60);

/// The longest one `colab drivemount` may take: the CLI's own ten minutes, and some to spare.
const ATTEMPT_LIMIT: Duration = Duration::from_secs(12 * 60);

/// How long the CLI gets to end once it was told to cancel: it waits for the kernel's answer.
const CANCEL_GRACE: Duration = Duration::from_secs(15);

/// Times Colab may miss the consent before connecting gives up.
const MAX_MISSES: u32 = 5;

const POLL: Duration = Duration::from_millis(100);

/// Where connecting Drive is, as a run reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum DriveStep {
    /// Asking Colab to connect it.
    Connecting,
    /// The user allows access on this Google page, then says so with
    /// [`crate::backend::Backend::confirm_drive_access`].
    Consent { url: String },
    /// Colab did not get the consent (not given yet, or handing it over failed); a new
    /// [`DriveStep::Consent`] follows.
    Missed,
    /// Connected: the user's files are in `folder` on the runtime.
    Connected { folder: String },
}

/// Connections waiting for the user to allow access, by account and session.
#[derive(Debug, Default)]
pub struct Consents(Mutex<HashMap<(AccountId, String), Sender<()>>>);

impl Consents {
    /// The user says they allowed access for the connection waiting on `session`.
    pub fn confirm(&self, account: &AccountId, session: &str) -> Result<()> {
        let waiting = self
            .lock()
            .get(&(account.clone(), session.to_owned()))
            .cloned();
        match waiting {
            Some(waiting) if waiting.send(()).is_ok() => Ok(()),
            _ => Err(
                Error::invalid("session", "no Google Drive connection waits on it")
                    .with("value", session),
            ),
        }
    }

    pub(crate) fn wait(&self, account: &AccountId, session: &str) -> Waiting<'_> {
        let (tx, rx) = mpsc::channel();
        let key = (account.clone(), session.to_owned());
        self.lock().insert(key.clone(), tx);
        Waiting {
            consents: self,
            key,
            rx,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<(AccountId, String), Sender<()>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A connection waiting for the user; it stops waiting when dropped.
pub(crate) struct Waiting<'a> {
    consents: &'a Consents,
    key: (AccountId, String),
    rx: Receiver<()>,
}

impl Waiting<'_> {
    /// The user said they allowed access.
    pub(crate) fn confirmed(&self) -> bool {
        self.rx.try_recv().is_ok()
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.consents.lock().remove(&self.key);
    }
}

/// Connecting failed. `detail` says why, for the details pane.
pub fn not_connected(detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::DriveNotConnected).detail(detail)
}

/// Python for `colab exec`: whether Drive is mounted and which folder holds the user's files.
pub fn check_snippet() -> String {
    let folders: Vec<String> = FOLDERS.iter().map(|f| format!("{f:?}")).collect();
    format!(
        r#"import json, os
_root = "{MOUNT_POINT}"
_mounted = False
try:
    with open("/proc/mounts") as _fh:
        for _line in _fh:
            _f = _line.split()
            if len(_f) > 2 and _f[1] == _root and _f[2] == "fuse.drive":
                _mounted = True
except OSError:
    pass
_folder = None
for _name in ({}, ):
    if _mounted and os.path.isdir(os.path.join(_root, _name)):
        _folder = os.path.join(_root, _name)
        break
print("{RESULT_MARKER}" + json.dumps({{"mounted": _mounted, "folder": _folder}}))
"#,
        folders.join(", ")
    )
}

/// The folder [`check_snippet`] found, or `None` when Drive is not mounted.
pub fn parse_check(v: &serde_json::Value) -> Result<Option<String>> {
    let mounted = v.get("mounted").and_then(serde_json::Value::as_bool);
    let folder = v.get("folder").and_then(serde_json::Value::as_str);
    match (mounted, folder) {
        (Some(true), Some(folder)) => Ok(Some(folder.to_owned())),
        (Some(true), None) => Err(not_connected(format!(
            "Drive is mounted at {MOUNT_POINT}, but none of {} is in it",
            FOLDERS.join(", ")
        ))),
        (Some(false), _) => Ok(None),
        (None, _) => {
            Err(Error::new(ErrorKind::Parse).detail(format!("unexpected Drive check: {v}")))
        }
    }
}

/// `colab --config <state> drivemount -s <session>` under [`DRIVE_MOUNT_PY`], run by `python`
/// (the CLI's interpreter), with stdin kept open for the answers.
pub fn mount_request(
    cli: &Cli,
    python: &str,
    state: &HostPath,
    session: &str,
) -> Result<ScriptRequest> {
    validate_session_name(session)?;
    let mut req = ScriptRequest::new(r#"exec "$CPG_PYTHON" -P -c "$CPG_LOADER" "$@""#)
        .args([
            B64.encode(DRIVE_MOUNT_PY),
            "--config".to_owned(),
            state.as_str().to_owned(),
            "drivemount".to_owned(),
            "-s".to_owned(),
            session.to_owned(),
        ])
        .stdin_pipe();
    req.env = cli.base_env.clone();
    req.env.push(("CPG_PYTHON".into(), python.to_owned()));
    req.env
        .push(("CPG_LOADER".into(), crate::tunnel::PYTHON_LOADER.to_owned()));
    Ok(req)
}

/// A line [`DRIVE_MOUNT_PY`] printed for the app.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mark {
    Consent(String),
    NotHandedOver,
    Cancelled,
    Failed(String),
    Unsupported(String),
}

fn mark(line: &str) -> Option<Mark> {
    let line = line.trim();
    if let Some(url) = line.strip_prefix(CONSENT) {
        let url = url.trim();
        return (!url.is_empty()).then(|| Mark::Consent(url.to_owned()));
    }
    if line.starts_with(NOT_HANDED_OVER) {
        return Some(Mark::NotHandedOver);
    }
    if line.starts_with(CANCELLED) {
        return Some(Mark::Cancelled);
    }
    if let Some(why) = line.strip_prefix(FAILED) {
        return Some(Mark::Failed(why.trim().to_owned()));
    }
    line.starts_with(UNSUPPORTED)
        .then(|| Mark::Unsupported(line.to_owned()))
}

/// How one `colab drivemount` ended.
#[derive(Debug, Default)]
struct Attempt {
    out: ScriptOutput,
    /// The user said they allowed access on the page it showed.
    consented: bool,
    /// Colab did not get the consent.
    missed: bool,
    failed: Option<String>,
    unsupported: Option<String>,
}

/// One session's Drive, connected through the CLI.
pub struct Connection<'a> {
    pub cli: &'a Cli,
    /// The CLI's interpreter.
    pub python: &'a str,
    /// The account's CLI state file.
    pub state: &'a HostPath,
    pub account: &'a AccountId,
    pub session: &'a str,
    pub consents: &'a Consents,
}

impl Connection<'_> {
    /// Connects Drive, asking the user through `step` when Colab needs their consent, and
    /// returns the folder holding their files. A runtime that has it connected already
    /// is not asked again.
    pub fn connect(&self, step: &dyn Fn(DriveStep), cancel: &CancelToken) -> Result<String> {
        step(DriveStep::Connecting);
        if let Some(folder) = self.check(cancel)? {
            step(DriveStep::Connected {
                folder: folder.clone(),
            });
            return Ok(folder);
        }
        let mut misses = 0;
        let mut tried_again = false;
        loop {
            let attempt = self.attempt(step, cancel)?;
            if let Some(what) = attempt.unsupported {
                return Err(Error::new(ErrorKind::UnsupportedCliVersion).detail(what));
            }
            if attempt.missed {
                misses += 1;
                if misses >= MAX_MISSES {
                    return Err(not_connected(format!(
                        "Colab did not get the consent {misses} times"
                    )));
                }
                step(DriveStep::Missed);
                step(DriveStep::Connecting);
                continue;
            }
            if let Some(failure) = attempt.failed {
                return Err(not_connected(failure));
            }
            if !attempt.out.success() {
                return Err(classify_failure(&attempt.out));
            }
            if let Some(folder) = self.check(cancel)? {
                step(DriveStep::Connected {
                    folder: folder.clone(),
                });
                return Ok(folder);
            }
            // Colab stops waiting for the consent after two minutes and mounts without it. Given
            // late, the consent still counts, and the next try needs no page.
            if attempt.consented && !tried_again {
                tried_again = true;
                step(DriveStep::Connecting);
                continue;
            }
            return Err(not_connected(tail(&attempt.out.stderr, 800)));
        }
    }

    fn check(&self, cancel: &CancelToken) -> Result<Option<String>> {
        let v = self
            .cli
            .exec_json(self.state, self.session, &[], &check_snippet(), 60, cancel)?;
        parse_check(&v)
    }

    fn attempt(&self, step: &dyn Fn(DriveStep), cancel: &CancelToken) -> Result<Attempt> {
        let req = mount_request(self.cli, self.python, self.state, self.session)?;
        let mut child = self.cli.host.spawn(req)?;
        let started = Instant::now();
        let mut attempt = Attempt::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut waiting: Option<(Waiting<'_>, Instant)> = None;
        // Told to stop while it waited for the user: when, and what the attempt ends with.
        let mut stopping: Option<(Instant, Error)> = None;
        loop {
            if stopping.is_none() {
                let stop = if cancel.is_cancelled() {
                    Some(Error::cancelled())
                } else if waiting
                    .as_ref()
                    .is_some_and(|(_, since)| since.elapsed() > CONSENT_WAIT)
                {
                    Some(not_connected(format!(
                        "nobody said access was allowed within {} minutes",
                        CONSENT_WAIT.as_secs() / 60
                    )))
                } else {
                    None
                };
                if let Some(error) = stop {
                    // Waiting for the user, the CLI tells the kernel no and ends; otherwise it
                    // hands over or mounts, which nothing here can take back.
                    if waiting.take().is_none() || child.write_stdin(b"cancel\n").is_err() {
                        child.kill();
                        return Err(error);
                    }
                    stopping = Some((Instant::now(), error));
                }
            }
            if let Some((at, _)) = &stopping
                && at.elapsed() > CANCEL_GRACE
            {
                child.kill();
                break;
            }
            if waiting.as_ref().is_some_and(|(w, _)| w.confirmed()) {
                waiting = None;
                if let Err(e) = child.write_stdin(b"continue\n") {
                    child.kill();
                    return Err(e);
                }
                attempt.consented = true;
                step(DriveStep::Connecting);
            }
            match child.recv_line(POLL) {
                Ok(line) if line.stream == Stream::Stderr => {
                    let prompt = line.text.contains(AUTH_PROMPT_MARKER);
                    err.push(line.text);
                    if prompt {
                        return Err(auth_failure(&mut child, out, err));
                    }
                }
                Ok(line) => {
                    match mark(&line.text) {
                        Some(Mark::Consent(url)) if stopping.is_none() => {
                            waiting = Some((
                                self.consents.wait(self.account, self.session),
                                Instant::now(),
                            ));
                            step(DriveStep::Consent { url });
                        }
                        Some(Mark::NotHandedOver) => attempt.missed = true,
                        Some(Mark::Failed(why)) => attempt.failed = Some(why),
                        Some(Mark::Unsupported(what)) => attempt.unsupported = Some(what),
                        _ => {}
                    }
                    out.push(line.text);
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if started.elapsed() > ATTEMPT_LIMIT {
                child.kill();
                return Err(
                    Error::new(ErrorKind::Timeout).detail("`colab drivemount` did not end in time")
                );
            }
        }
        if let Some((_, error)) = stopping {
            return Err(error);
        }
        attempt.out = ScriptOutput {
            code: child.wait(),
            stdout: out.join("\n"),
            stderr: err.join("\n"),
            killed: false,
        };
        Ok(attempt)
    }
}

/// The CLI has no usable sign-in and asks for one, which only the app's own sign-in gives.
fn auth_failure(child: &mut HostChild, out: Vec<String>, err: Vec<String>) -> Error {
    child.kill();
    let e = classify_failure(&ScriptOutput {
        code: None,
        stdout: out.join("\n"),
        stderr: err.join("\n"),
        killed: true,
    });
    if e.kind == ErrorKind::KeyringUnavailable {
        e
    } else {
        e.with_kind(ErrorKind::AuthRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrapper_prints_what_this_reads() {
        for line in [CONSENT, NOT_HANDED_OVER, CANCELLED, FAILED, UNSUPPORTED] {
            assert!(DRIVE_MOUNT_PY.contains(&format!("\"{line}\"")), "{line}");
        }
    }

    #[test]
    fn marks_are_read_from_the_wrappers_lines() {
        assert_eq!(
            mark("[colab-playground] drive consent: https://accounts.google.com/x?y=1"),
            Some(Mark::Consent("https://accounts.google.com/x?y=1".into()))
        );
        assert_eq!(mark("[colab-playground] drive consent:"), None);
        assert_eq!(mark(NOT_HANDED_OVER), Some(Mark::NotHandedOver));
        assert_eq!(mark(CANCELLED), Some(Mark::Cancelled));
        assert_eq!(
            mark("[colab-playground] drive consent failed: KeyError: 'token'"),
            Some(Mark::Failed("KeyError: 'token'".into()))
        );
        assert!(matches!(
            mark("[colab-playground] unsupported colab CLI: no automation.ColabRuntime"),
            Some(Mark::Unsupported(_))
        ));
        assert_eq!(
            mark("[colab] Mounting Google Drive to '/content/drive' on s..."),
            None
        );
        assert_eq!(mark("Mounted at /content/drive"), None);
    }

    #[test]
    fn the_request_runs_the_wrapper_on_the_cli_with_its_state_file() {
        let cli = Cli::new(
            crate::Host::Local,
            "/x/colab",
            vec![("HOME".into(), "/h".into())],
        );
        let state = HostPath::new("/d/accounts/a1/sessions.json");
        let req = mount_request(&cli, "/x/python", &state, "llama-cpp-1a2b").unwrap();
        assert_eq!(B64.decode(&req.args[0]).unwrap(), DRIVE_MOUNT_PY.as_bytes());
        assert_eq!(
            &req.args[1..],
            [
                "--config",
                "/d/accounts/a1/sessions.json",
                "drivemount",
                "-s",
                "llama-cpp-1a2b"
            ]
        );
        assert!(req.env.contains(&("HOME".into(), "/h".into())));
        assert!(req.env.contains(&("CPG_PYTHON".into(), "/x/python".into())));
        assert_eq!(req.stdin, crate::host::StdinMode::Pipe);
        assert!(mount_request(&cli, "/x/python", &state, "-s x").is_err());
    }

    #[test]
    fn the_check_names_the_folder_only_when_drive_is_mounted() {
        let snippet = check_snippet();
        assert!(snippet.contains(r#"_root = "/content/drive""#), "{snippet}");
        assert!(
            snippet.contains(r#"("MyDrive", "My Drive", )"#),
            "{snippet}"
        );
        let v = |j: &str| serde_json::from_str::<serde_json::Value>(j).unwrap();
        assert_eq!(
            parse_check(&v(
                r#"{"mounted": true, "folder": "/content/drive/MyDrive"}"#
            ))
            .unwrap(),
            Some("/content/drive/MyDrive".into())
        );
        assert_eq!(
            parse_check(&v(r#"{"mounted": false, "folder": null}"#)).unwrap(),
            None
        );
        assert!(
            parse_check(&v(r#"{"mounted": true, "folder": null}"#))
                .unwrap_err()
                .is(ErrorKind::DriveNotConnected)
        );
        assert!(parse_check(&v("{}")).unwrap_err().is(ErrorKind::Parse));
    }

    #[test]
    fn a_confirm_reaches_only_a_connection_that_waits() {
        let consents = Consents::default();
        let account = AccountId::new("a1").unwrap();
        assert!(consents.confirm(&account, "s1").is_err(), "nothing waits");
        let waiting = consents.wait(&account, "s1");
        assert!(!waiting.confirmed());
        assert!(
            consents
                .confirm(&AccountId::new("a2").unwrap(), "s1")
                .is_err()
        );
        consents.confirm(&account, "s1").unwrap();
        assert!(waiting.confirmed());
        drop(waiting);
        assert!(
            consents.confirm(&account, "s1").is_err(),
            "it stopped waiting"
        );
    }
}
