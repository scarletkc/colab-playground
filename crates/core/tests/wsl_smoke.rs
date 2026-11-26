//! Smoke test of the Windows → WSL script bridge.
//!
//! Ignored by default: it spawns `wsl.exe` and must only be run on purpose
//! (`cargo test -p colab-playground-core --test wsl_smoke -- --ignored`). It
//! only performs `printf`/`cat` round trips through the default distribution.
#![cfg(windows)]

use std::time::{Duration, Instant};

use colab_playground_core::host::{Host, HostKeepalive, LineAction, ScriptRequest};
use colab_playground_core::{CancelToken, ErrorKind};

#[test]
#[ignore = "spawns wsl.exe; run explicitly"]
fn wsl_keepalive_holds_until_stdin_closes() {
    let host = Host::Wsl { distro: None };
    let keepalive = HostKeepalive::new(host.clone());
    keepalive.ensure();
    // `ensure` returns once the session bus is up: the socket must exist by now.
    let bus = host
        .run_quiet(
            ScriptRequest::new(
                r#"case ${DBUS_SESSION_BUS_ADDRESS-} in
unix:path=*) s=${DBUS_SESSION_BUS_ADDRESS#unix:path=}; s=${s%%[,;]*}; [ -S "$s" ] && echo bus=up || echo bus=missing ;;
*) echo bus=none ;;
esac"#,
            ),
            &CancelToken::new(),
        )
        .expect("wsl.exe run");
    assert!(!bus.stdout.contains("bus=missing"), "{}", bus.stdout);
    std::thread::sleep(Duration::from_secs(3));
    assert!(keepalive.is_running(), "the keepalive ended while held");
    drop(keepalive);

    // A process like it ends once its stdin closes, which is what happens when the app exits.
    let mut child = host
        .spawn(ScriptRequest::new("cat >/dev/null").stdin_pipe())
        .expect("wsl.exe spawn");
    std::thread::sleep(Duration::from_secs(3));
    assert!(child.try_wait().is_none(), "ended before stdin closed");
    child.close_stdin();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(child.try_wait(), Some(Some(0)));
}

#[test]
#[ignore = "spawns wsl.exe; run explicitly"]
fn wsl_bridge_round_trips_argv_env_stdin_exit_code_and_utf8() {
    let host = Host::Wsl { distro: None };
    let script = r#"printf 'a1=[%s]\n' "$1"
printf 'a2=[%s]\n' "$2"
printf 'argc=%s\n' "$#"
printf 'env=[%s]\n' "$CPG_SMOKE"
printf 'stdin=['
cat
printf ']\n'
printf 'to-stderr\n' >&2
exit 7"#;
    let tricky = "héllo 'single' \"double\" $(not run) `nor this` ; | & * 中文 ✓";
    let req = ScriptRequest::new(script)
        .arg(tricky)
        .arg("line1\nline2")
        .env("CPG_SMOKE", "value with spaces=and equals ñ")
        .stdin_bytes("piped ✓ 输入".as_bytes().to_vec())
        .timeout(Duration::from_secs(120));
    let mut lines = Vec::new();
    let out = host
        .run(req, &CancelToken::new(), &mut |l| {
            lines.push(l.clone());
            LineAction::Continue
        })
        .expect("wsl.exe run");
    assert_eq!(
        out.code,
        Some(7),
        "stdout: {}\nstderr: {}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains(&format!("a1=[{tricky}]")),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("a2=[line1\nline2]"), "{}", out.stdout);
    assert!(out.stdout.contains("argc=2"), "{}", out.stdout);
    assert!(
        out.stdout.contains("env=[value with spaces=and equals ñ]"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("stdin=[piped ✓ 输入]"),
        "{}",
        out.stdout
    );
    assert!(out.stderr.contains("to-stderr"));
    assert!(
        !out.stderr.contains("CPG-PGID"),
        "the job marker must be stripped"
    );
    assert!(
        !out.stderr.contains("__cpg"),
        "bash's report of the finished job must not reach stderr: {}",
        out.stderr
    );

    // A long-running job is killed as a process group inside WSL.
    let cancel = CancelToken::new();
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        c2.cancel();
    });
    let err = host
        .run_quiet(ScriptRequest::new("sleep 60 & sleep 60; wait"), &cancel)
        .unwrap_err();
    assert!(err.is(ErrorKind::Cancelled));
}
