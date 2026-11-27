#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{ffi::OsString, io::Write as _};

use colab_playground::{
    app,
    backend::{BACKEND_ENV, launch_from_args, usage},
    control::{self, CTL_COMMAND, Claim, Owner},
    platform,
    settings::{CONFIG_DIR_ENV, SettingsStore, config_dir},
    state::StateOptions,
};
use colab_playground_core::backend::BackendKind as Flavor;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,colab_playground=info"),
    )
    .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let backend_env = std::env::var(BACKEND_ENV).ok();
    let config_override = std::env::var_os(CONFIG_DIR_ENV);
    if args.first().map(String::as_str) == Some(CTL_COMMAND) {
        platform::attach_console();
        let outcome = control::run_ctl(&args[1..], backend_env.as_deref(), config_override);
        // A reader that closed the pipe early has what it wanted.
        let _ = writeln!(std::io::stdout().lock(), "{}", outcome.stdout);
        if let Some(text) = outcome.stderr {
            let _ = writeln!(std::io::stderr().lock(), "{text}");
        }
        std::process::exit(outcome.code);
    }
    // Every argument is checked before any backend exists: a typo never starts the real one.
    let launch = match launch_from_args(&args, backend_env.as_deref()) {
        Ok(launch) => launch,
        Err(error) => {
            log::error!("{error}");
            eprintln!("colab-playground: {error}\n{}", usage());
            std::process::exit(2);
        }
    };
    let mut owner = own(launch.flavor(), config_override.clone());
    let service = owner.as_mut().and_then(|owner| {
        owner
            .listen()
            .inspect_err(|error| log::error!("control: cannot listen: {error}"))
            .ok()
    });
    let store = SettingsStore::new(config_dir(config_override));
    app::run(
        StateOptions {
            source: launch.into_source(),
            store,
            os_locale: platform::os_locale(),
        },
        service,
    );
    // Held until here (on macOS the process ends inside `run`, and the OS lets go of it).
    drop(owner);
}

/// Takes this backend's state, or hands over to the copy that owns it and ends this process.
fn own(flavor: Flavor, config_override: Option<OsString>) -> Option<Owner> {
    let Some(dir) = control::state_dir(flavor, config_override) else {
        log::warn!("control: no folder for this backend's state; a second copy is not detected");
        return None;
    };
    match control::claim(&dir, flavor) {
        Claim::Owner(owner) => Some(owner),
        Claim::Shown => {
            log::info!(
                "app: another copy owns {}; it was brought forward",
                dir.display()
            );
            std::process::exit(0);
        }
        Claim::Unresponsive => {
            log::error!(
                "app: another copy owns {} but does not answer",
                dir.display()
            );
            eprintln!(
                "colab-playground: another copy owns {} but does not answer",
                dir.display()
            );
            std::process::exit(1);
        }
        Claim::Unavailable(error) => {
            log::error!(
                "control: cannot lock {}: {error}; running without the owner's lock",
                dir.display()
            );
            None
        }
    }
}
