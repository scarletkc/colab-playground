//! Owning a state: the lock, the loopback listener, and answering on the app's foreground.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use colab_playground_core::{backend::BackendKind as Flavor, paths::write_atomic};
use futures::{
    StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
};
use gpui::{App, Entity, Global};
use serde::Deserialize;

use super::{
    Code, Command, Endpoint, MAX_REQUEST_BYTES, PROTOCOL_VERSION, Response, Token, ctl,
    endpoint_file, lock_file,
};
use crate::state::AppState;

/// How long a second launch keeps trying: the owner may still be opening its window, or be
/// closing (an update starts the new version before the old one has quit).
const OWNER_WAIT: Duration = Duration::from_secs(10);
const CLAIM_RETRY: Duration = Duration::from_millis(150);
/// A client that has connected sends its line at once; one that does not is dropped, so it
/// cannot hold up the next.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the listener waits for the foreground, which answers from memory and makes no
/// backend call.
pub(super) const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// What a launch found.
#[derive(Debug)]
pub enum Claim {
    /// This process owns the state. Keep the [`Owner`] until the process ends.
    Owner(Owner),
    /// Another process owns it and was asked to show its window.
    Shown,
    /// Another process owns it but did not answer in time.
    Unresponsive,
    /// The lock could not be taken for another reason; the app runs without an owner's lock.
    Unavailable(io::Error),
}

/// The lock on a state, held until the process ends.
#[derive(Debug)]
pub struct Owner {
    /// Released by the OS when the process ends, however it ends.
    _lock: File,
    dir: PathBuf,
    flavor: Flavor,
    endpoint: Option<PathBuf>,
    quitting: Arc<AtomicBool>,
}

/// What the app needs to answer requests: they wait here until it serves them.
pub struct Service {
    jobs: UnboundedReceiver<Job>,
    quitting: Arc<AtomicBool>,
    endpoint: PathBuf,
}

struct Job {
    command: Command,
    reply: mpsc::Sender<Response>,
}

/// Set when the app is on its way out, so a second launch waits for the lock instead of
/// asking a window that is about to close to come forward.
struct Quitting(Arc<AtomicBool>);

impl Global for Quitting {}

/// Takes `dir`'s lock for `flavor`, or asks its owner to show its window.
pub fn claim(dir: &Path, flavor: Flavor) -> Claim {
    claim_within(dir, flavor, OWNER_WAIT)
}

/// [`claim`], trying for `wait` while another process holds the lock and does not answer.
pub fn claim_within(dir: &Path, flavor: Flavor, wait: Duration) -> Claim {
    if let Err(error) = fs::create_dir_all(dir) {
        return Claim::Unavailable(error);
    }
    let lock = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_file(dir, flavor))
    {
        Ok(file) => file,
        Err(error) => return Claim::Unavailable(error),
    };
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => {
                return Claim::Owner(Owner {
                    _lock: lock,
                    dir: dir.to_path_buf(),
                    flavor,
                    endpoint: None,
                    quitting: Arc::default(),
                });
            }
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => return Claim::Unavailable(error),
        }
        // Any answer but yes (starting, quitting, not listening yet) means: wait for the lock.
        if ctl::request(dir, flavor, Command::Activate)
            .is_ok_and(|answer| answer.get("ok") == Some(&serde_json::Value::Bool(true)))
        {
            return Claim::Shown;
        }
        if Instant::now() >= deadline {
            return Claim::Unresponsive;
        }
        thread::sleep(CLAIM_RETRY);
    }
}

impl Owner {
    /// Starts listening. Requests wait in the returned service until the app serves them.
    pub fn listen(&mut self) -> io::Result<Service> {
        let (jobs, inbox) = unbounded();
        let quitting = self.quitting.clone();
        let flavor = self.flavor;
        let endpoint = listen(&self.dir, flavor, move |command| {
            forward(&jobs, &quitting, flavor, command)
        })?;
        self.endpoint = Some(endpoint.clone());
        Ok(Service {
            jobs: inbox,
            quitting: self.quitting.clone(),
            endpoint,
        })
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(endpoint) = &self.endpoint {
            remove_endpoint(endpoint);
        }
    }
}

fn remove_endpoint(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => log::info!("control: removed {}", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("control: could not remove {}: {error}", path.display()),
    }
}

/// Hands one request to the foreground and waits for its answer.
fn forward(
    jobs: &UnboundedSender<Job>,
    quitting: &AtomicBool,
    flavor: Flavor,
    command: Command,
) -> Response {
    let gone = || Response::failure(Some(flavor), Code::Quitting, None);
    if quitting.load(Ordering::SeqCst) {
        return gone();
    }
    let (reply, answer) = mpsc::channel();
    if jobs.unbounded_send(Job { command, reply }).is_err() {
        return gone();
    }
    match answer.recv_timeout(ANSWER_TIMEOUT) {
        Ok(response) => response,
        Err(RecvTimeoutError::Timeout) => Response::failure(Some(flavor), Code::Timeout, None),
        Err(RecvTimeoutError::Disconnected) => gone(),
    }
}

/// Binds a loopback port the OS picks, writes `dir`'s endpoint file for `flavor` with a fresh
/// token, and answers every authorised request with `handler` on a thread of its own. Returns
/// the endpoint file.
pub fn listen<H>(dir: &Path, flavor: Flavor, mut handler: H) -> io::Result<PathBuf>
where
    H: FnMut(Command) -> Response + Send + 'static,
{
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let token = Token::generate()?;
    let endpoint = Endpoint {
        port,
        token: token.clone(),
        backend: flavor,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let path = endpoint_file(dir, flavor);
    write_atomic(&path, serde_json::to_string(&endpoint)?.as_bytes())?;
    thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if let Err(error) = answer_connection(&stream, &token, flavor, &mut handler)
                        {
                            log::debug!("control: a connection ended early: {error}");
                        }
                    }
                    Err(error) => log::warn!("control: accepting a connection failed: {error}"),
                }
            }
        })?;
    log::info!("control: listening on 127.0.0.1:{port}");
    Ok(path)
}

fn answer_connection<H>(
    stream: &TcpStream,
    token: &Token,
    flavor: Flavor,
    handler: &mut H,
) -> io::Result<()>
where
    H: FnMut(Command) -> Response,
{
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST_BYTES)).read_line(&mut line)?;
    let response = respond(&line, token, flavor, handler);
    let mut text = serde_json::to_string(&response)?;
    text.push('\n');
    let mut writer = stream;
    writer.write_all(text.as_bytes())?;
    writer.flush()
}

/// Every field optional, so the token is checked before anything else about the request.
#[derive(Deserialize)]
struct RawRequest {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    v: Option<u32>,
    #[serde(default)]
    backend: Option<serde_json::Value>,
    #[serde(default)]
    command: Option<serde_json::Value>,
}

/// The response to one request line. A request without the token learns nothing, not even
/// which backend answers.
pub(super) fn respond<H>(line: &str, token: &Token, flavor: Flavor, handler: &mut H) -> Response
where
    H: FnMut(Command) -> Response + ?Sized,
{
    let raw = serde_json::from_str::<RawRequest>(line.trim())
        .ok()
        .filter(|raw| raw.token.as_deref().is_some_and(|sent| token.matches(sent)));
    let Some(raw) = raw else {
        log::warn!("control: refused a request without the token");
        return Response::failure(None, Code::Unauthorized, None);
    };
    let refuse = |code, message: String| Response::failure(Some(flavor), code, Some(message));
    if raw.v != Some(PROTOCOL_VERSION) {
        return refuse(
            Code::UnsupportedVersion,
            format!("this copy speaks protocol version {PROTOCOL_VERSION}"),
        );
    }
    let backend = raw
        .backend
        .and_then(|value| serde_json::from_value::<Flavor>(value).ok());
    if backend != Some(flavor) {
        return refuse(
            Code::WrongBackend,
            format!("this copy runs on the {} backend", super::kind_name(flavor)),
        );
    }
    let Some(command) = raw
        .command
        .and_then(|value| serde_json::from_value::<Command>(value).ok())
    else {
        return refuse(Code::InvalidRequest, "unknown command".to_owned());
    };
    log::info!("control: asked {}", command.name());
    handler(command)
}

/// Answers the requests waiting in `service` from `state` on the foreground, for as long as
/// the app runs, and removes the endpoint file when the app quits.
pub fn serve(service: Service, state: Entity<AppState>, cx: &mut App) {
    let Service {
        mut jobs,
        quitting,
        endpoint,
    } = service;
    cx.set_global(Quitting(quitting.clone()));
    cx.on_app_quit(move |_| {
        quitting.store(true, Ordering::SeqCst);
        remove_endpoint(&endpoint);
        async {}
    })
    .detach();
    // Weak: the task lives as long as the app, and a strong handle would outlive the entities
    // gpui releases when it quits.
    let state = state.downgrade();
    cx.spawn(async move |cx| {
        while let Some(job) = jobs.next().await {
            let response = cx.update(|cx| match state.upgrade() {
                Some(state) => answer(job.command, &state, cx),
                None => Response::failure(None, Code::Quitting, None),
            });
            // The listener gave up waiting: nobody is left to tell.
            let _ = job.reply.send(response);
        }
    })
    .detach();
}

/// Marks the app as on its way out (or back, when restarting into an update failed).
pub fn mark_quitting(quitting: bool, cx: &App) {
    if let Some(Quitting(flag)) = cx.try_global::<Quitting>() {
        flag.store(quitting, Ordering::SeqCst);
    }
}

/// The answer to `command` from what the app shows now.
pub fn answer(command: Command, state: &Entity<AppState>, cx: &mut App) -> Response {
    let flavor = state.read(cx).flavor();
    let quitting = cx
        .try_global::<Quitting>()
        .is_some_and(|Quitting(flag)| flag.load(Ordering::SeqCst));
    if quitting {
        return Response::failure(Some(flavor), Code::Quitting, None);
    }
    if command == Command::Activate {
        for window in cx.windows() {
            if let Err(error) = window.update(cx, |_, window, _| window.activate_window()) {
                log::warn!("control: could not bring a window forward: {error}");
            }
        }
        cx.activate(true);
        return Response::success(flavor, serde_json::json!({}));
    }
    state.read(cx).control_answer(command, cx)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::{Ipv4Addr, TcpStream},
        sync::mpsc,
        thread,
        time::Duration,
    };

    use colab_playground_core::backend::BackendKind as Flavor;
    use futures::StreamExt;

    use super::{Claim, Owner, claim_within, listen, respond};
    use crate::control::{
        Code, Command, Endpoint, PROTOCOL_VERSION, Response, Token, ctl, endpoint_file,
    };

    /// A quick claim, for a lock nobody answers for.
    const SHORT: Duration = Duration::from_millis(300);

    fn owner(dir: &std::path::Path, flavor: Flavor) -> Owner {
        match claim_within(dir, flavor, SHORT) {
            Claim::Owner(owner) => owner,
            other => panic!("expected to own {flavor:?}, got {other:?}"),
        }
    }

    fn data(response: &Response) -> &serde_json::Value {
        response.data.as_ref().expect("an answer with data")
    }

    /// Answers every request with the command's name, the way the app's foreground would.
    fn echo(command: Command) -> Response {
        Response::success(
            Flavor::Mock,
            serde_json::json!({ "command": command.name() }),
        )
    }

    fn send_line(dir: &std::path::Path, flavor: Flavor, line: &str) -> String {
        let text = std::fs::read_to_string(endpoint_file(dir, flavor)).expect("an endpoint");
        let endpoint: Endpoint = serde_json::from_str(&text).expect("an endpoint");
        let mut stream =
            TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).expect("connects");
        stream.write_all(line.as_bytes()).expect("sent");
        let mut answer = String::new();
        BufReader::new(stream)
            .read_line(&mut answer)
            .expect("answered");
        answer
    }

    #[test]
    fn a_request_round_trips_through_the_listener() {
        let dir = tempfile::tempdir().expect("a temp dir");
        listen(dir.path(), Flavor::Mock, echo).expect("listens");
        let answer = ctl::request(dir.path(), Flavor::Mock, Command::Sessions).expect("answered");
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["backend"], "mock");
        assert_eq!(answer["data"]["command"], "sessions");
    }

    #[test]
    fn a_wrong_or_missing_token_gets_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (asked, handled) = mpsc::channel();
        listen(dir.path(), Flavor::Mock, move |command| {
            asked.send(command).expect("the test listens");
            echo(command)
        })
        .expect("listens");
        let unauthorized = r#"{"ok":false,"error":{"code":"unauthorized"}}"#;
        for line in [
            r#"{"v":1,"backend":"mock","command":"status"}"#,
            r#"{"v":1,"token":"0000","backend":"mock","command":"status"}"#,
            r#"{"v":1,"token":7,"backend":"mock","command":"status"}"#,
            "GET / HTTP/1.1",
            "",
        ] {
            let answer = send_line(dir.path(), Flavor::Mock, &format!("{line}\n"));
            assert_eq!(answer.trim(), unauthorized, "for {line:?}");
        }
        assert!(
            handled.try_recv().is_err(),
            "nothing reached the app without the token"
        );
    }

    #[test]
    fn the_token_is_checked_before_the_rest_of_the_request() {
        let token = Token::from_text("secret");
        let mut handler = |command| echo(command);
        let answer = |line: &str, handler: &mut dyn FnMut(Command) -> Response| {
            respond(line, &token, Flavor::Mock, handler)
        };
        assert_eq!(
            answer(r#"{"v":99,"command":"bogus"}"#, &mut handler).code(),
            Some(Code::Unauthorized)
        );
        let wrong_version = answer(
            r#"{"v":99,"token":"secret","backend":"mock","command":"status"}"#,
            &mut handler,
        );
        assert_eq!(wrong_version.code(), Some(Code::UnsupportedVersion));
        assert_eq!(wrong_version.backend, Some(Flavor::Mock));
        let other_kind = answer(
            &format!(
                r#"{{"v":{PROTOCOL_VERSION},"token":"secret","backend":"real","command":"status"}}"#
            ),
            &mut handler,
        );
        assert_eq!(other_kind.code(), Some(Code::WrongBackend));
        let unknown = answer(
            &format!(
                r#"{{"v":{PROTOCOL_VERSION},"token":"secret","backend":"mock","command":"stop"}}"#
            ),
            &mut handler,
        );
        assert_eq!(unknown.code(), Some(Code::InvalidRequest));
        let fine = answer(
            &format!(
                r#"{{"v":{PROTOCOL_VERSION},"token":"secret","backend":"mock","command":"usage"}}"#
            ),
            &mut handler,
        );
        assert_eq!(data(&fine)["command"], "usage");
    }

    #[test]
    fn a_client_never_reaches_an_owner_of_the_other_backend() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut mock = owner(dir.path(), Flavor::Mock);
        let _service = mock.listen().expect("listens");
        let answer = ctl::request(dir.path(), Flavor::Real, Command::Status)
            .expect_err("no real owner runs");
        assert_eq!(answer.code(), Some(Code::NotRunning));
        assert_eq!(answer.backend, Some(Flavor::Real));
    }

    #[test]
    fn with_no_owner_the_client_says_so() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let answer =
            ctl::request(dir.path(), Flavor::Mock, Command::Status).expect_err("nothing runs");
        assert_eq!(answer.code(), Some(Code::NotRunning));

        // An owner that crashed leaves its endpoint file behind; its port no longer answers.
        let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|listener| listener.local_addr())
            .expect("a port")
            .port();
        let stale = Endpoint {
            port: closed,
            token: Token::from_text("old"),
            backend: Flavor::Mock,
            pid: 1,
            version: "0.0.0".into(),
        };
        std::fs::write(
            endpoint_file(dir.path(), Flavor::Mock),
            serde_json::to_string(&stale).expect("serialises"),
        )
        .expect("left behind");
        let answer =
            ctl::request(dir.path(), Flavor::Mock, Command::Status).expect_err("nothing runs");
        assert_eq!(answer.code(), Some(Code::NotRunning), "{answer:?}");
    }

    #[test]
    fn a_second_launch_finds_the_owner_and_asks_it_forward() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut first = owner(dir.path(), Flavor::Mock);
        let mut service = first.listen().expect("listens");
        let (asked, activated) = mpsc::channel();
        thread::spawn(move || {
            while let Some(job) = futures::executor::block_on(service.jobs.next()) {
                asked.send(job.command).expect("the test listens");
                let _ = job
                    .reply
                    .send(Response::success(Flavor::Mock, serde_json::json!({})));
            }
        });
        assert!(matches!(
            claim_within(dir.path(), Flavor::Mock, SHORT),
            Claim::Shown
        ));
        assert_eq!(activated.recv().expect("asked"), Command::Activate);

        // A real owner of the same folder is a different state.
        assert!(matches!(
            claim_within(dir.path(), Flavor::Real, SHORT),
            Claim::Owner(_)
        ));
    }

    #[test]
    fn a_second_launch_waits_for_an_owner_that_is_quitting() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut first = owner(dir.path(), Flavor::Mock);
        let service = first.listen().expect("listens");
        service
            .quitting
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let releases = thread::spawn(move || {
            thread::sleep(SHORT);
            drop(service);
            drop(first);
        });
        let second = claim_within(dir.path(), Flavor::Mock, Duration::from_secs(5));
        releases.join().expect("released");
        assert!(matches!(second, Claim::Owner(_)), "{second:?}");
    }

    #[test]
    fn an_owner_that_never_answers_is_reported() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let _first = owner(dir.path(), Flavor::Mock);
        assert!(matches!(
            claim_within(dir.path(), Flavor::Mock, SHORT),
            Claim::Unresponsive
        ));
    }

    #[test]
    fn a_clean_exit_removes_the_endpoint_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut first = owner(dir.path(), Flavor::Mock);
        let _service = first.listen().expect("listens");
        assert!(endpoint_file(dir.path(), Flavor::Mock).exists());
        drop(first);
        assert!(!endpoint_file(dir.path(), Flavor::Mock).exists());
    }

    #[cfg(unix)]
    #[test]
    fn only_the_user_can_read_the_endpoint_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = listen(dir.path(), Flavor::Mock, echo).expect("listens");
        let mode = std::fs::metadata(path)
            .expect("written")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
