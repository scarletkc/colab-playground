//! Owning a state: the lock, the loopback listener, and answering on the app's foreground.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
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

use super::{
    Code, Command, Endpoint, MAX_REQUEST_BYTES, PROTOCOL_VERSION, Response, Token, ctl,
    endpoint_file, lock_file,
};
use crate::state::AppState;

/// How long a second launch keeps trying: the owner may still be opening its window, or be
/// closing (an update starts the new version before the old one has quit).
const OWNER_WAIT: Duration = Duration::from_secs(10);
const CLAIM_RETRY: Duration = Duration::from_millis(150);
/// A client that has connected sends its line at once; one whose whole line has not arrived by
/// then is dropped, so it cannot hold up the next.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes asked of the connection at a time; a request fits in one.
const READ_CHUNK: usize = 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the listener waits for the foreground, which answers from memory and waits on no
/// backend call: a command that starts one answers once it is under way.
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
    claim: Arc<AtomicU8>,
    /// The connection the answer goes to, which tells whether its client still waits.
    client: Option<Arc<TcpStream>>,
}

/// Where a job stands between the listener, which waits only so long, and the foreground. It
/// leaves `PENDING` once, to one side: a request the listener answered `timeout` is never done
/// afterwards, when it could start a billed runtime nobody was told about.
const PENDING: u8 = 0;
const TAKEN: u8 = 1;
const ABANDONED: u8 = 2;

impl Job {
    /// Takes the job for the foreground to do. False when nobody waits for its answer any more:
    /// the listener answered `timeout`, or the client hung up (it waits only so long itself,
    /// from before its request was read, and it may have been stopped).
    fn take(&self) -> bool {
        if self.client.as_deref().is_some_and(hung_up) {
            return false;
        }
        self.claim
            .compare_exchange(PENDING, TAKEN, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

/// Whether the other end closed the connection. A client keeps it open until it has its answer
/// and sends nothing after its request, so a read that would end at once means it left.
fn hung_up(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let gone = match stream.peek(&mut [0u8; 1]) {
        Ok(read) => read == 0,
        Err(error) => error.kind() != io::ErrorKind::WouldBlock,
    };
    // The listener writes the answer to this connection, and waits for the write.
    if let Err(error) = stream.set_nonblocking(false) {
        log::warn!("control: could not set a connection back to blocking: {error}");
    }
    gone
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
        let endpoint = listen_with(
            &self.dir,
            flavor,
            move |command, client: &Arc<TcpStream>| {
                forward(
                    &jobs,
                    &quitting,
                    flavor,
                    command,
                    Some(client.clone()),
                    ANSWER_TIMEOUT,
                )
            },
        )?;
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

/// Hands one request to the foreground and waits up to `wait` for it to begin on it. Answers
/// `timeout` only for a request the foreground then never does.
fn forward(
    jobs: &UnboundedSender<Job>,
    quitting: &AtomicBool,
    flavor: Flavor,
    command: Command,
    client: Option<Arc<TcpStream>>,
    wait: Duration,
) -> Response {
    if quitting.load(Ordering::SeqCst) {
        return Response::failure(Some(flavor), Code::Quitting, None);
    }
    let (reply, answer) = mpsc::channel();
    let claim = Arc::new(AtomicU8::new(PENDING));
    let job = Job {
        command,
        reply,
        claim: claim.clone(),
        client,
    };
    if jobs.unbounded_send(job).is_err() {
        return Response::failure(Some(flavor), Code::Quitting, None);
    }
    await_answer(&answer, &claim, flavor, wait)
}

/// Waits up to `wait` for the foreground to begin on a job, and then for its answer.
fn await_answer(
    answer: &mpsc::Receiver<Response>,
    claim: &AtomicU8,
    flavor: Flavor,
    wait: Duration,
) -> Response {
    // The job was dropped undone: the app is on its way out, or the client hung up.
    let gone = || Response::failure(Some(flavor), Code::Quitting, None);
    match answer.recv_timeout(wait) {
        Ok(response) => response,
        Err(RecvTimeoutError::Timeout) => {
            let abandoned = claim
                .compare_exchange(PENDING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
            if abandoned {
                return Response::failure(Some(flavor), Code::Timeout, None);
            }
            // The foreground has begun on it: its answer says what was done.
            answer.recv().unwrap_or_else(|_| gone())
        }
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
    listen_with(dir, flavor, move |command, _: &Arc<TcpStream>| {
        handler(command)
    })
}

/// [`listen`], with a handler that also gets the connection its answer goes to.
fn listen_with<H>(dir: &Path, flavor: Flavor, mut handler: H) -> io::Result<PathBuf>
where
    H: FnMut(Command, &Arc<TcpStream>) -> Response + Send + 'static,
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
                        let stream = Arc::new(stream);
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
    stream: &Arc<TcpStream>,
    token: &Token,
    flavor: Flavor,
    handler: &mut H,
) -> io::Result<()>
where
    H: FnMut(Command, &Arc<TcpStream>) -> Response,
{
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let line = read_request(stream, Instant::now() + READ_TIMEOUT)?;
    let response = respond(&line, token, flavor, &mut |command| {
        handler(command, stream)
    });
    let mut text = serde_json::to_string(&response)?;
    text.push('\n');
    let mut writer: &TcpStream = stream;
    writer.write_all(text.as_bytes())?;
    writer.flush()
}

/// Reads the request line, giving up at `deadline` however the bytes arrive. A read timeout
/// alone bounds each read, not the line: connections are answered one at a time, so a client
/// that sent a byte now and then, and never the newline, would keep every other one waiting.
fn read_request(stream: &TcpStream, deadline: Instant) -> io::Result<String> {
    let mut line = Vec::new();
    let mut chunk = [0u8; READ_CHUNK];
    let mut reader = stream;
    while (line.len() as u64) < MAX_REQUEST_BYTES {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        stream.set_read_timeout(Some(left))?;
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        if let Some(end) = chunk[..read].iter().position(|byte| *byte == b'\n') {
            line.extend_from_slice(&chunk[..end]);
            break;
        }
        line.extend_from_slice(&chunk[..read]);
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// The fields of a request; any other is refused rather than ignored, so a newer client's
/// request is never read as a different one.
const REQUEST_FIELDS: [&str; 5] = ["v", "token", "backend", "command", "args"];

/// The response to one request line. A request without the token learns nothing, not even
/// which backend answers: the line is read as a bare object so the token is checked before
/// anything else about it.
pub(super) fn respond<H>(line: &str, token: &Token, flavor: Flavor, handler: &mut H) -> Response
where
    H: FnMut(Command) -> Response + ?Sized,
{
    let raw = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(line.trim())
        .ok()
        .filter(|raw| {
            raw.get("token")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|sent| token.matches(sent))
        });
    let Some(mut raw) = raw else {
        log::warn!("control: refused a request without the token");
        return Response::failure(None, Code::Unauthorized, None);
    };
    let refuse = |code, message: String| Response::failure(Some(flavor), code, Some(message));
    if raw.get("v").and_then(serde_json::Value::as_u64) != Some(u64::from(PROTOCOL_VERSION)) {
        return refuse(
            Code::UnsupportedVersion,
            format!("this copy speaks protocol version {PROTOCOL_VERSION}"),
        );
    }
    let backend = raw
        .get("backend")
        .and_then(|value| serde_json::from_value::<Flavor>(value.clone()).ok());
    if backend != Some(flavor) {
        return refuse(
            Code::WrongBackend,
            format!("this copy runs on the {} backend", super::kind_name(flavor)),
        );
    }
    if let Some(field) = raw
        .keys()
        .find(|key| !REQUEST_FIELDS.contains(&key.as_str()))
    {
        return refuse(
            Code::InvalidRequest,
            format!("this copy does not know the field {field:?}"),
        );
    }
    let mut asked = serde_json::Map::new();
    for field in ["command", "args"] {
        if let Some(value) = raw.remove(field).filter(|value| !value.is_null()) {
            asked.insert(field.to_owned(), value);
        }
    }
    let command = match serde_json::from_value::<Command>(asked.into()) {
        Ok(command) => command,
        Err(error) => {
            return refuse(
                Code::InvalidRequest,
                format!("this copy does not understand the request: {error}"),
            );
        }
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
            if !job.take() {
                log::warn!(
                    "control: left {} undone; nobody waits for its answer any more",
                    job.command.name()
                );
                continue;
            }
            let response = cx.update(|cx| match state.upgrade() {
                Some(state) => answer(job.command, &state, cx),
                None => Response::failure(None, Code::Quitting, None),
            });
            // The listener is gone only when the app is on its way out.
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

/// The answer to `command` from what the app shows now, after doing what it asks.
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
    state.update(cx, |state, cx| state.control_answer(command, cx))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, BufRead, BufReader, Write},
        net::{Ipv4Addr, TcpListener, TcpStream},
        sync::{Arc, atomic::AtomicU8, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use colab_playground_core::backend::BackendKind as Flavor;
    use futures::StreamExt;

    use super::{
        Claim, Job, Owner, PENDING, await_answer, claim_within, forward, hung_up, listen,
        read_request, respond,
    };
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
            asked.send(command.clone()).expect("the test listens");
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
    fn a_request_that_trickles_in_is_dropped_at_the_deadline() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a port");
        let port = listener.local_addr().expect("an address").port();
        // Every byte arrives well within the deadline; the newline never does.
        let trickle = thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            while stream.write_all(b"x").is_ok() {
                thread::sleep(SHORT / 20);
            }
        });
        let (stream, _) = listener.accept().expect("a client");
        let started = Instant::now();
        let error = read_request(&stream, started + SHORT).expect_err("no line arrived");
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ),
            "{error}"
        );
        assert!(started.elapsed() < SHORT * 10, "{:?}", started.elapsed());
        drop(stream);
        trickle.join().expect("the client saw the connection close");
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
        let asked = |rest: &str| {
            format!(r#"{{"v":{PROTOCOL_VERSION},"token":"secret","backend":"mock",{rest}}}"#)
        };
        for refused in [
            r#""command":"bogus""#,
            r#""command":"stop""#,
            r#""command":"usage","args":{"account":"lin"}"#,
            r#""command":"stop","args":{"runtime":"t4-91c2","force":true}"#,
            r#""command":"stop","args":{"runtime":"t4-91c2"},"yes":true"#,
            r#""args":{"runtime":"t4-91c2"}"#,
        ] {
            assert_eq!(
                answer(&asked(refused), &mut handler).code(),
                Some(Code::InvalidRequest),
                "{refused} is refused, not read as another request"
            );
        }
        let fine = answer(&asked(r#""command":"usage""#), &mut handler);
        assert_eq!(data(&fine)["command"], "usage");
        let stop = answer(
            &asked(r#""command":"stop","args":{"runtime":"t4-91c2"}"#),
            &mut handler,
        );
        assert_eq!(data(&stop)["command"], "stop");
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
                assert!(job.take());
                asked.send(job.command.clone()).expect("the test listens");
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
    fn a_request_answered_as_timed_out_is_never_done_afterwards() {
        let flavor = Flavor::Mock;
        let quitting = std::sync::atomic::AtomicBool::new(false);
        let start = || Command::Start(crate::control::StartArgs::default());
        let wait = Duration::from_millis(20);

        // The foreground is busy for longer than the listener waits.
        let (jobs, mut inbox) = futures::channel::mpsc::unbounded::<Job>();
        let answer = forward(&jobs, &quitting, flavor, start(), None, wait);
        assert_eq!(answer.code(), Some(Code::Timeout));
        let late = futures::executor::block_on(inbox.next()).expect("still queued");
        assert!(!late.take(), "the foreground leaves it undone");

        // The foreground began on it before the wait ran out: its answer is the one given.
        let (reply, answer) = mpsc::channel();
        let job = Job {
            command: start(),
            reply,
            claim: Arc::new(AtomicU8::new(PENDING)),
            client: None,
        };
        assert!(job.take());
        let claim = job.claim.clone();
        let foreground = thread::spawn(move || {
            thread::sleep(SHORT);
            let done = Response::success(flavor, serde_json::json!({ "operation": "x-1" }));
            job.reply.send(done).expect("the listener still waits");
        });
        let answer = await_answer(&answer, &claim, flavor, wait);
        foreground.join().expect("the foreground ran");
        assert!(answer.ok, "{answer:?}");
    }

    #[test]
    fn a_request_whose_client_hung_up_is_left_undone() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("binds");
        let client = TcpStream::connect(listener.local_addr().expect("an address")).expect("up");
        let (server, _) = listener.accept().expect("accepts");
        let server = Arc::new(server);
        let job = || Job {
            command: Command::Start(crate::control::StartArgs::default()),
            reply: mpsc::channel().0,
            claim: Arc::new(AtomicU8::new(PENDING)),
            client: Some(server.clone()),
        };
        assert!(job().take(), "its client still waits");
        // Looking did not change how the answer is written and read.
        (&*server).write_all(b"answer\n").expect("written");
        let mut answer = String::new();
        BufReader::new(&client)
            .read_line(&mut answer)
            .expect("read");
        assert_eq!(answer, "answer\n");

        drop(client);
        // The close reaches this end a moment later.
        let deadline = Instant::now() + SHORT;
        while !hung_up(&server) {
            assert!(Instant::now() < deadline, "the close never arrived");
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!job().take(), "nobody waits for the answer");
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
