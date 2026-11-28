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

- **Any time:** Settings → About → "Give feedback" opens GitHub in your browser, where you choose
  to report a problem or suggest an idea. Either form has the app's version and operating system
  filled in.
- **When the app asks:** after a few runtimes or deployments, the app asks whether you enjoy it,
  with "Star on GitHub" (the project page) and "Give feedback". It asks only right after you stop
  a runtime while nothing else runs or starts, at most twice, and never again once you choose
  either link.
- **What is sent:** opening the issue forms passes the app's version and operating system to
  GitHub in the link. Nothing is posted until you submit the form, and the app itself sends nothing.
- **Sponsoring:** Settings → About → "Sponsor on GitHub" opens the author's GitHub Sponsors page
  in your browser. The app never asks for it.

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
  `update-unverified` (the download fails its check), `update-install-fails` (installing an
  update; add `update-available`) and `plugin-unloadable` (the plugins folder holds a plugin made
  for a newer version of the app).
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

`colab-playground ctl` lets a script or an AI agent read what the running app shows, and start,
stop and extend runtimes and forward their ports through it, so the window shows what they do as
it happens. It asks the running app and does nothing on its own: it never starts the app, signs in
or runs `colab`.

1. Turn it on in Settings → Command line → "Allow control from the command line". It is off until
   you do, and while it is off every command answers `control_off`. On its own it lets programs
   read, not change anything.
2. To let them change things too, also turn on "Also allow changes" below it: starting, stopping
   and extending runtimes, and opening and closing port forwards. A runtime started or extended
   this way uses compute units like any other. Until you do, every command that takes `--yes`
   answers `changes_off`; turning the first switch off turns this one off as well.
3. Call the program you start the app with, followed by `ctl`: "Program to run", below the two
   switches, shows its path and copies it. The app does not add it to `PATH`, so in these
   examples `colab-playground` stands for that path (from source, use
   `cargo run -p colab-playground -- ctl status`):

   ```sh
   colab-playground ctl status      # what the app shows at a glance, and what it is busy with
   colab-playground ctl accounts    # the saved accounts and the active one
   colab-playground ctl sessions    # the runtimes on every saved account
   colab-playground ctl usage       # compute units per account and in total
   colab-playground ctl start --hardware T4         # what a start would do; --yes starts it
   colab-playground ctl stop a100-7f3a              # what a stop would do; --yes stops it
   colab-playground ctl extend a100-7f3a            # what an extension would do; --yes extends
   colab-playground ctl forward a100-7f3a --port 8000   # what a forward would do; --yes opens it
   colab-playground ctl close-forward 8000 --yes    # close the forward on localhost:8000
   colab-playground ctl operation <id> --wait 600   # follow a change until it ends
   colab-playground ctl cancel <id> --yes           # cancel a start and give back what it got
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
  (`control_off`'s and `changes_off`'s are in the app's language, the others are in English).
- **Exit code:** 0 when the app answered (also when the operation it reports failed, or a wait ran
  out), 1 when it refused or nothing answered, 2 for a wrong command line (the usage goes to
  standard error). `ctl --help` prints the usage.
- **On Windows**, the downloaded program is a windowed one, so a shell you type into does not wait
  for it: the prompt comes back before the answer is printed, and the exit code is lost. A script
  or an agent that reads the output through a pipe or a file is not affected; at the prompt, add
  `| Out-String` in PowerShell to wait for the answer.

| Code | Meaning |
| --- | --- |
| `invalid_arguments` | The command line was wrong. |
| `not_running` | No copy of the app runs on that backend (for demo data: with that config folder). |
| `control_off` | Control from the command line is turned off; the message says where to turn it on. |
| `changes_off` | Changes from the command line are turned off; the message says where to turn them on. |
| `quitting` | The app is closing, or restarting into an update (also while it asks whether to quit, or downloads an update). Ask again in a moment. |
| `timeout` | The app did not answer in time. It leaves undone a request nobody waits for any more, so a change asked this way almost never still happens; after a `start --yes` that ended like this, look at `sessions` before asking again. |
| `not_ready` | What the command needs is still loading (the accounts, the runtime list), or could not be loaded; the message says which. |
| `no_active_account` | No account is active: none is signed in, the CLI is not set up, or it is signed in to an account the app has not saved. |
| `account_changing` | An account is being added, switched or removed in the window; for `start`, `extend` and `forward`, also while the switch or sign-in dialog is open there. |
| `already_starting` | A runtime is being started already, from the window or the command line. |
| `form_open` | The New session dialog is open in the window with what was entered there (the form, or a start that failed or was cancelled); it is left alone until it is started or closed there. |
| `session_limit_reached` | The active account has as many runtimes as this app starts per account (Settings → Defaults); the message names them. |
| `no_such_runtime`, `ambiguous_runtime` | `stop`, `extend` or `forward` named no listed runtime, or more than one. |
| `already_stopping` | That runtime is being stopped already. |
| `account_not_active` | `extend` and `forward` act only on runtimes of the active account, as the window does; accounts are switched in the window. |
| `no_auto_stop` | `extend` named a runtime without an auto-stop: one opened in the browser or another tool, or one whose auto-stop is off because it was started without one or cancelled (the window can set one). |
| `auto_stop_changing` | That runtime's auto-stop is being extended, set or cancelled already. |
| `extension_not_due` | `extend` named a runtime with more time left before its auto-stop than one extension adds; ask again once that much or less is left. Right after an extension it can come as a failed operation instead. |
| `unnamed_runtime` | `forward` named a runtime opened in the browser, which has no session name to reach it by. |
| `port_in_use` | Another forward of the app listens on that port of this computer. |
| `already_forwarding` | That port of that runtime is being forwarded already. |
| `no_such_forward`, `ambiguous_forward` | `close-forward` named a port that no forward listens on, or one that more than one forward is listed on (close those in the window). |
| `already_closing` | That forward is being closed already. |
| `unknown_operation` | This run of the app has no operation with that id. |
| `operation_ended`, `not_cancellable` | `cancel` was asked for an operation that has ended, or for one that is not a start. |
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
  watched, newest first, with `reason` `user` (stopped in the window), `command_line` (by
  `ctl stop`), `guard` with its `concern`, or `vanished`). Each
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

Changing things:

- **A preview first.** The commands that take `--yes` change nothing without it: they answer
  `{"preview": {…}}` with what they would do, from what the window's own confirmation shows, or
  with the code the change would be refused with. With `--yes` they go through the window's own
  steps: the window shows the progress and a notice that the command line asked, closing the window
  counts the runtime like any other, and the same limits apply.
- **Refused, not queued.** A change that cannot happen now (another start under way, an account
  being switched, the limit reached, the app closing, that runtime's auto-stop or forward being
  changed already, an extension that is not due yet) is refused with its code, and nothing waits
  to happen later.
- **`start`** starts a runtime on the active account. `--hardware` is `CPU`, `T4`, `L4`, `G4`,
  `A100`, `H100`, `V5E1` or `V6E1` (any case), `--name` a session name; everything else comes from
  Settings → Defaults, as the New session dialog fills it in. No option changes a limit (the
  runtime limit, spending cap, balance floor, idle stop or session limit): those stay with you in
  the window. Without `--name` a name is made up, and a start after a preview uses the name it
  showed. The preview holds `account` (`id`, `email`), `name`, `accelerator`, `high_mem`,
  `min_vram_mib`, `attempts`, `estimated_rate_cu_per_hour` (an estimate, `≈` in the window) with
  `rate_source` (`measured`, by the app on this hardware at `rate_measured_at`, or `published` by
  other users), `policy` (when it stops on its own:
  `max_runtime`, `max_spend_cu`, `min_balance_cu`, `idle_timeout`) and `session_limit` (`limit`;
  `counted`, the runtimes that count; `allows`; both `null` while the runtimes are not listed, and
  the start is checked again as it begins).
- **`stop <runtime>`** stops a runtime on any saved account and ends its billing. `<runtime>` is a
  session name or an endpoint (`id.name` or `id.endpoint` in `sessions`); a runtime opened in the
  browser has no name, so its endpoint names it. `--account` with an email or account id keeps to
  one account, for a name that two accounts have. The preview holds `runtime` (as in `sessions`)
  and `ends`: `billing` and `files` (everything on the runtime is wiped), and `port_forwards` when
  it has any.
- **`extend <runtime>`** moves the auto-stop of a runtime on the active account later by what the
  window's "Extend by …" adds: the runtime limit in Settings → Defaults, or two hours without one.
  No option chooses another amount, and the other limits (spending cap, balance floor, idle stop)
  still apply. Unlike Extend in the window, it is refused with `extension_not_due` while more time
  is left before the auto-stop than one extension adds: a program that keeps asking keeps the
  runtime running, but never puts its auto-stop more than two extensions away. Right after an
  extension, until `sessions` shows its new `auto_stop_at`, the refusal can come as an operation
  that ends `failed` with the error `extension_not_due` instead of at once.
  `<runtime>` and `--account` name it as for `stop`, and only a runtime with an auto-stop can be
  extended. The preview holds `runtime` (as in `sessions`) and `extend_by`, not the new time: the
  app reads the time left from the runtime as it extends. Once the operation succeeded, its
  `outcome` holds `auto_stop_at`, the new auto-stop, and `auto_stop_on_runtime_at`, when the
  runtime stops itself also with the app closed (`null`: the stop holds only while the app runs).
- **`forward <runtime>`** forwards a port of a runtime on the active account to this computer, as
  the window's "Forward a port" does. `--port` is the port on the runtime (8000 when left out, as
  in the dialog; never 8080, which Colab keeps), `--local-port` the port on this computer (the
  same number when left out), and `--health` a path the app checks through the forward. A runtime
  opened in the browser has no session name and cannot be forwarded. The preview holds `runtime`
  and `forward` (`session`, `remote_port`, `local_port`, `health_path`, `url`). The operation ends
  once the forward is up (`succeeded`), when it fails first (`failed`, with the forward's error,
  such as `ssh_busy` or `port_in_use`), or when it is closed or its runtime stops before that
  (`cancelled`). After that, `sessions` shows how the forward is doing.
- **`close-forward <local-port>`** closes the forward that listens on that port of this computer,
  or clears one there that failed, as its close button in the window does. Its runtime keeps running and
  billing: the preview holds `forward` (as in `sessions`, with its `url`) and `stops_billing`,
  which is always `false`.
- **Operations.** Every change answers `--yes` at once with `{"operation": {…}}`, as
  `ctl operation <id>` shows it: each waits on Colab or on a forward's connection, which takes
  seconds or longer, and the app answers one request at a time. An operation has `id`, `kind`
  (`start`, `stop`, `extend`, `forward` or `close_forward`), `state` (`running`, `succeeded`,
  `failed` with an `error`, or `cancelled`), `cancelling`, `started_at`, `ended_at` and `account`
  (`id`, `email`). By kind, it has `runtime` (for a start `name`, `accelerator`, `high_mem`; for a
  stop or an extension `id`, `label`) or `forward` (as in the preview of `forward`); an extension
  also has `extend_by` and `outcome`. A start also has `progress`, its steps as the window lists
  them (each with an `event` such as `allocating`, `probed`, `releasing`, `retrying` or `ready`),
  and once it succeeded `outcome` (`name`, `hardware`, `attempts`, `below_spec`, `watchdog`,
  `started_at`). `ok` only says that the app answered; how the operation went is its `state`.
- **Waiting:** `operation <id> --wait <seconds>` asks again until the operation ends or the time
  is up, then answers as `operation <id>` does. A wait that runs out answers with the operation
  still `running`: that is not a failure, and the operation goes on.
- **Cancelling:** `cancel <id> --yes` asks a start to stop and to give back anything it got; it
  answers at once, with the operation still `running` and `cancelling`. The operation ends as
  `cancelled` once nothing it got is left billing. If a runtime could not be given back, it ends
  as `failed` with the error `orphan_suspected` instead, and that runtime may still bill:
  `sessions` lists it, unless its account is among `unlisted_accounts`, and `stop` ends it. Cancel
  in the window ends the operation the same way.
  Only a start can be cancelled (`not_cancellable` for the others: a forward that is still coming
  up ends when `close-forward` closes it), and not once it has ended (`operation_ended`). Nothing
  else cancels: a `ctl` that is stopped or killed while it waits leaves the operation running.
- **How long an id lasts:** until the app quits, and the app keeps the 100 operations that ended
  last. For an id from before that, from before a restart, or one it never gave, `operation`
  answers `unknown_operation`, which says nothing about how the operation went: `sessions` shows
  what runs.
