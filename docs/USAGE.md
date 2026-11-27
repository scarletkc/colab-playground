# Building and running Colab Playground

How to build the app, choose its backend, try it with demo data, and find what it stores.
What the app does is in the [README](../README.md); the principles and rules behind it are in
[DESIGN.md](DESIGN.md).

## Install a release

Download the file for your computer from the
[latest release](https://github.com/scarletkc/colab-playground/releases/latest); there is no
installer. Later versions install from inside the app (see [Updates](#updates)); settings and data
stay where they are (see [Where things are stored](#where-things-are-stored)).

- **Windows x64:** `colab-playground-x86_64-pc-windows-msvc.exe` is the whole app; run it. It is
  not code-signed, so SmartScreen may say "Windows protected your PC": click **More info**, then
  **Run anyway**.
- **Linux x64:** unpack `colab-playground-x86_64-unknown-linux-gnu.tar.gz` (`tar -xzf <file>`) and
  run `colab-playground` in the folder it creates. It needs glibc 2.35 or newer (for example
  Ubuntu 22.04 or Debian 12 and later) and libraries a desktop installation already has: libxcb,
  libxkbcommon and libxkbcommon-x11.
- **macOS 11 or later on Apple silicon:** unzip `colab-playground-aarch64-apple-darwin.zip` and
  move `Colab Playground.app` to Applications (opened where it was unzipped, it cannot update
  itself). It is not notarised by Apple, so macOS refuses the first launch: try to open it once,
  then open System Settings → Privacy & Security and, under Security, click **Open Anyway**
  (offered for about an hour after the attempt) and enter your login password. After that it opens
  normally.
- `SHA256SUMS` in the same release lists the SHA-256 of each file; compare it with the output of
  `sha256sum <file>` (Linux), `shasum -a 256 <file>` (macOS) or `Get-FileHash <file>` (PowerShell).
  `SHA256SUMS.sig` is the project's signature of that list, which the app checks before it
  installs an update ([what it covers](RELEASING.md#the-update-signature)).

A download is a release build without demo data: it always uses the real Colab CLI (see
[Build and run](#build-and-run)), so the real-mode requirements below apply.

## Updates

The app checks GitHub for a newer release, and installs it when you ask.

- **Automatic check:** shortly after each start, then once a day while the app runs. A check that
  fails is tried again every hour until one gets an answer. Turn it off in Settings → About →
  "Check for updates automatically".
- **By hand:** Settings → About → "Check for updates".
- **When a newer version exists:** a notice appears once for that version, and the Settings entry
  stays marked until you have updated. "Download and restart", on the notice and in Settings →
  About, first says what a restart affects: open port forwards close, runtimes keep running and
  billing, and an auto-stop set on a runtime keeps working. It then downloads the version, checks
  it, puts it in place of the running one and restarts; the steps are shown as they happen, and
  Cancel works until the new version is being put in place. It waits while a runtime is being
  started, a deployment runs, an account is being added or the CLI is being installed. "What's
  new" opens the release page.
- **The check before installing:** the download must match `SHA256SUMS` in the release, and
  `SHA256SUMS` must carry the project's signature for that release (see
  [The update signature](RELEASING.md#the-update-signature)). If anything fails, this version stays
  as it is and keeps working (in the rare case that it cannot be put back where it was, the app
  says where it is now). A download that fails the check is not installed, and the app does not
  offer that version's download page either.
- **When the app cannot update itself** it says why and offers "Open download page" instead, where
  you download the new version as in [Install a release](#install-a-release) and replace the old
  file or app. That happens when the app cannot write to its folder (for example
  `C:\Program Files`), when macOS runs it from where it was unzipped (move it to Applications), for
  a build that is not a release download (such as `cargo run`), and when its files are not laid out
  as the download leaves them.
- **What is sent:** the check is one request for this repository's latest release. Installing
  downloads `SHA256SUMS`, `SHA256SUMS.sig` and the file for your computer from that release. Each
  request carries the app's version and nothing about you or your computer.
- On Windows the replaced program is deleted the next time the app starts.
- A check that fails is reported in Settings → About (and in a message when you started it by hand).
- Demo data never contacts GitHub: it simulates the download, the check and the installation, and
  replaces nothing.

## Feedback

- **Any time:** Settings → About → "Give feedback" opens a new issue on GitHub in your browser,
  with the app's version and operating system filled in.
- **When the app asks:** after a few runtimes or deployments, the app asks whether you enjoy it,
  with "Star on GitHub" (the project page) and "Give feedback". It asks only right after you stop
  a runtime while nothing else runs or starts, at most twice, and never again once you choose
  either link.
- **What is sent:** opening the issue form passes the app's version and operating system to GitHub
  in its link. Nothing is posted until you submit the form, and the app itself sends nothing.

## Requirements

- Rust: the version in `rust-version` under `[workspace.package]` in
  [`Cargo.toml`](../Cargo.toml), or newer.
- Linux build: `libxkbcommon-x11-dev` and `libfontconfig-dev` (Ubuntu package names). Windows and
  macOS need nothing beyond the Rust toolchain.
- Real mode only: on Windows, WSL with a Linux distribution; inside it (or locally on macOS and
  Linux) `bash`, `ssh` for port forwarding, a keyring the CLI can use, and either the Colab CLI
  (`colab`, package `google-colab-cli`) or `uv` / `python3` to install it. Installing plugins from
  a Git URL uses the `git` of the computer running the app (Git for Windows on Windows).

## Build and run

```sh
cargo run -p colab-playground              # debug build: demo data (mock backend)
cargo run -p colab-playground -- --real    # debug build against the real Colab CLI
cargo run -p colab-playground --release    # release build: real Colab CLI by default
```

- Debug builds default to the **mock** backend, release builds to the **real** one.
- `--mock` or `--real` overrides the default; without a flag, `COLAB_PLAYGROUND_BACKEND=mock` or
  `=real` does. A flag wins over the variable. Both flags at once, or any other value of the
  variable, is an error (exit code 2).
- An unknown argument, a flag without its value, or a demo flag without `--mock` is an error too
  (exit code 2, with the usage line on standard error), so a mistyped flag never starts the other
  backend. Mock mode shows a **Demo data** badge, and Settings → About → Backend names the backend
  in use.
- The mock backend is the `mock` cargo feature, on by default. Release downloads are built without
  it (`--no-default-features`), so they run only the real backend: there `--mock`, the demo flags
  and `COLAB_PLAYGROUND_BACKEND=mock` are errors (exit code 2), and Settings → About has no
  Backend row. A debug build without it starts only with `--real`.
- One copy runs per backend: starting the app again while it runs brings its window to the front
  instead. A demo copy and a real one can run side by side, and so can demo copies with different
  `COLAB_PLAYGROUND_CONFIG_DIR` folders.

## Demo data

The mock backend is a deterministic simulation: it never runs WSL or the CLI, never touches a
Google account, and bills nothing. Its sign-in opens a placeholder Google URL; any code is accepted.

```sh
cargo run -p colab-playground -- --mock-scenario signed-out --mock-fail cli-missing
```

- `--mock-scenario demo` (default): two accounts; on the active one an A100 session, a T4 session
  and a runtime "opened in the browser"; one runtime on the second account.
  `single`: one account, nothing running. `signed-out`: first run, no account.
- `--mock-fail a,b,…` injects failures: `cli-missing`, `install-fails`, `install-off-path` (the
  install succeeds but `colab` is still not found, as when it lands in a folder outside `PATH`;
  add `cli-missing`), `keyring`, `usage-fails`,
  `login-invalid-code`, `login-unverified`, `not-entitled` (A100 and H100), `no-answer` (Colab
  never answers a request for an A100: after a minute the progress says how long it has waited,
  after about ten minutes it fails, and cancelling it shows the cleanup of the runtime the request
  got anyway), `ssh-busy` (the first
  forward gets HTTP 429), `plugin-fails` (at the download stage), `watchdog-restart-fails` (setting
  an auto-stop again after cancelling it cannot put the stop back on the runtime), `runtimes-fail`
  (runtimes cannot be listed), `update-available` and `update-check-fails` (the update check),
  `update-unsupported` (this copy cannot update itself), `update-download-fails`,
  `update-unverified` (the download fails its check) and `update-install-fails` (installing an
  update; add `update-available`).
- Both accept `--flag value` or `--flag=value` and apply only when the mock is selected (add
  `--mock` to a release build of your own). A simulated A100 has 40 GB first and 80 GB on the next
  attempt: set a minimum GPU memory of 70 GB to watch the retry. Git plugin installs are not
  simulated.

Demo mode still reads and writes the normal settings file; set `COLAB_PLAYGROUND_CONFIG_DIR` to
another folder to keep experiments apart. A demo copy and a real one running side by side on that
file do not undo each other's changes, but each shows a change made in the other only from its
next start.

## Where things are stored

- **Settings:** `settings.json` in `<OS config dir>/colab-playground`: `%APPDATA%\colab-playground`
  on Windows, `~/Library/Application Support/colab-playground` on macOS,
  `${XDG_CONFIG_HOME:-~/.config}/colab-playground` on Linux. `COLAB_PLAYGROUND_CONFIG_DIR` replaces
  that folder (empty means unset). An unreadable file is moved aside as `settings.json.corrupt`.
  Settings → About shows the file and opens its folder.
- **Real-mode data** (not moved by that variable): `accounts.json` (no secrets), `ledger.json`
  (sessions the app started) and user plugins in `<OS data dir>/colab-playground` (the same folder
  as the settings on Windows and macOS, `${XDG_DATA_HOME:-~/.local/share}/colab-playground` on
  Linux). Per-account CLI state files are under `~/.local/share/colab-playground/accounts/` where
  the CLI runs (inside WSL on Windows; `$XDG_DATA_HOME` is honoured).
- **Sign-ins and the Hugging Face token:** the keyring where the CLI runs (inside WSL on Windows).
  On Windows the app keeps the WSL distribution running while it is open, so a password-protected
  keyring asks to be unlocked once after WSL starts rather than at every background refresh.
- **CLI location:** Settings → "Colab CLI location" picks the WSL distribution and, if detection
  fails, the path of `colab` (its Python interpreter is optional and otherwise found from it).
- **Logs** go to standard error; `RUST_LOG` replaces the default filter
  `warn,colab_playground=info`. Release builds on Windows open no console window.
- **The running copy:** `control-real.lock` and `control-mock.lock` mark it, and
  `control-real.json` or `control-mock.json`, which only you can read, tells
  [`ctl`](#control-from-the-command-line) how to reach it; the app removes that file when it
  closes. They are in the real-mode data folder for the real backend and in the settings folder
  for demo data.

- **SSH key:** port forwarding leaves `~/.ssh/config` alone and creates `~/.ssh/id_ed25519` only
  when there is neither an ed25519 nor an ECDSA key.

## Working with several accounts

Extending, forwarding and deploying need the runtime's account to be active; stopping works on
any saved account.

## Control from the command line

`colab-playground ctl` lets a script or an AI agent read what the running app shows. It asks the
running app and does nothing on its own: it never starts the app, signs in or runs `colab`.

1. Turn it on in Settings → Command line → "Allow control from the command line". It is off until
   you do, and while it is off every command answers `control_off`.
2. Call the program you start the app with (the downloaded `.exe` on Windows;
   `cargo run -p colab-playground -- ctl status` from source):

   ```sh
   colab-playground ctl status      # what the app shows at a glance, and what it is busy with
   colab-playground ctl accounts    # the saved accounts and the active one
   colab-playground ctl sessions    # the runtimes on every saved account
   colab-playground ctl usage       # compute units per account and in total
   ```

- **Which app it asks:** the one on the backend the app itself would pick: `--mock` or `--real`,
  else `COLAB_PLAYGROUND_BACKEND`, else the build's default (real for a release build, demo data
  for a debug build). For demo data it asks the copy that uses the same
  `COLAB_PLAYGROUND_CONFIG_DIR`. A command never reaches a copy on the other backend.
- **Output:** one line of JSON on standard output, also when it is piped or written to a file:
  `{"ok": true, "backend": "mock", "data": {…}}`, or
  `{"ok": false, "backend": "mock", "error": {"code": "…", "message": "…"}}`. `backend` names the
  copy that answered, or the one `ctl` looked for when none did; only a wrong command line has
  none. The field names and codes stay as they are; a `message` is for people and may change
  (`control_off`'s is in the app's language, the others are in English).
- **Exit code:** 0 when the app answered, 1 when it refused or nothing answered, 2 for a wrong
  command line (the usage goes to standard error). `ctl --help` prints the usage.
- **On Windows**, the downloaded program is a windowed one, so a shell you type into does not wait
  for it: the prompt comes back before the answer is printed, and the exit code is lost. A script
  or an agent that reads the output through a pipe or a file is not affected; at the prompt, add
  `| Out-String` in PowerShell to wait for the answer.

| Code | Meaning |
| --- | --- |
| `invalid_arguments` | The command line was wrong. |
| `not_running` | No copy of the app runs on that backend (for demo data: with that config folder). |
| `control_off` | Control from the command line is turned off; the message says where to turn it on. |
| `quitting` | The app is closing, or restarting into an update. Ask again in a moment. |
| `timeout` | The app did not answer in time. |
| `connection_failed`, `bad_response` | Something other than the app answered, or the connection broke. |
| `unauthorized`, `invalid_request`, `unsupported_version`, `wrong_backend` | The request did not come from this version of `ctl`, for example after an update replaced the program while an older copy still runs. Restart the app. |

What the answers hold:

- **Every `data`:** `now`, the app's clock when it answered. Times are seconds since the Unix
  epoch on that clock, durations are seconds, and compute units are numbers (`cu`).
- **Unknown is never empty or zero.** `accounts`, `sessions` and `usage`, and the parts of
  `status`, carry a `state`: `not_loaded` (not asked for yet, as before the CLI is set up and an
  account saved), `loading`, `ready`, or `failed` with an `error`. Lists and totals are there only when `ready`.
  A total over accounts whose usage is not known says how many it leaves out, and is `null`
  while it knows none. `refreshing` is true while a newer reading is on its way.
- **Errors** inside an answer are `{"kind", "params", "detail", "next_step"}`; `kind` is a stable
  code such as `http`, `auth_required` or `keyring_unavailable`.
- **`status`:** `version`; `backend_error` (the backend could not start); `environment`
  (`cli_installed`, `cli_version`); `accounts` (`saved`, `active` with `id` and `email`,
  `unadopted`: the CLI is signed in to an account the app has not saved); `runtimes` (`running`,
  `unlisted_accounts`); `usage` (as `total` in `usage`); `activity`, a list of
  `installing_cli`, `adding_account`, `switching_account`, `starting_runtime`, `deploying`,
  `stopping_runtimes` and `updating`.
- **`accounts`:** `accounts` (each with `id`, `email`, `created_at`, `updated_at` and `usage`, the
  last reading saved with it), `active` (an `id`), `active_email`, `has_token` (the CLI is signed
  in), `unadopted`.
- **`sessions`:** `listed_at`; `runtimes`; `unlisted_accounts` (accounts whose runtimes could not
  be listed, with `account`, `email` and `error`: their runtimes are unknown, not absent);
  `guard_error` (the last cost-guard round failed); `ended` (runtimes that ended while the app
  watched, newest first, with `reason` `user`, `guard` with its `concern`, or `vanished`). Each
  runtime has `id` (`account`, `endpoint`, `name`: `null` for one opened in the browser), `label`,
  `account_email`, `on_active_account`, `managed` (started by this app), `accelerator`,
  `hardware`, `gpu`, `vram_mib`, `high_mem`, `region`, `started_at`,
  `estimated_rate_cu_per_hour`, `estimated_spend_cu` (estimates, `≈` in the window),
  `auto_stop_at` (the earliest stop the app or the runtime will make),
  `auto_stop_on_runtime_at` (when the runtime stops itself, also with the app closed; `null`
  when nothing on the runtime does, so the stop holds only while the app runs; it can be a
  little later than `auto_stop_at`), `policy` (`max_runtime`, `max_spend_cu`,
  `min_balance_cu`, `idle_timeout`), `stopping`, `warnings` (limits close to being reached),
  `stop_failure` (the cost guard could not stop it: it is still billing) and `tunnels` (each with
  its `spec`, `state` and `healthy`).
- **`usage`:** `error` (the last refresh failed); `total` (`balance_cu`, `rate_cu_per_hour`,
  `runtimes`, `known_accounts`, `unknown_accounts`, `stale`); `accounts`, each with `account`,
  `email`, `active`, `snapshot` (`balance_cu`, `rate_cu_per_hour`, `assignments`, `fetched_at`;
  `null` while not known), `stale` (the window marks it stale: a refresh failed or it is
  old), `eta_secs` (time left at the current rate), `unmanaged_runtimes` (`null`
  when not known), `error` and `history` (recent balances).
