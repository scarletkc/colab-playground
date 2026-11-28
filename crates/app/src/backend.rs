//! Which backend the UI talks to, and how it is built.
//!
//! `main` resolves the flags into a [`Launch`], and only once this copy owns the state into a
//! [`BackendSource`]; `AppState` turns that into an `Arc<dyn Backend>` once the settings
//! (defaults, WSL distro, `colab` path) are loaded. Tests
//! never come through here: the harness hands `AppState` a headless `MockBackend` directly.

#[cfg(feature = "mock")]
use std::time::Duration;
use std::{fmt, sync::Arc};

use colab_playground_core::{
    Defaults, Host, Toolchain,
    backend::{Backend, BackendKind as Flavor},
    real::{RealBackend, RealConfig},
};
#[cfg(feature = "mock")]
use colab_playground_core::{
    mock::{MockBackend, MockConfig, MockFailures, MockLoginFailure, MockScenario},
    sessions::Accelerator,
};

use crate::platform;

/// Environment variable that picks the backend when no flag is given: `mock` or `real`.
pub const BACKEND_ENV: &str = "COLAB_PLAYGROUND_BACKEND";

/// How long each simulated step of the interactive mock takes (pacing for `--mock` runs).
#[cfg(feature = "mock")]
pub const MOCK_STEP_DELAY: Duration = Duration::from_millis(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Deterministic simulation; never touches WSL, the `colab` CLI or a Google account.
    Mock,
    /// The real `colab` CLI through the core crate's `Host`.
    Real,
}

/// What an executable can run and starts on, as plain values so tests can try every kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Build {
    /// Demo data (the `mock` feature) is compiled in. Release downloads leave it out.
    pub mock: bool,
    pub debug: bool,
}

impl Build {
    /// This executable.
    pub const CURRENT: Build = Build {
        mock: cfg!(feature = "mock"),
        debug: cfg!(debug_assertions),
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    ConflictingFlags,
    InvalidEnv(String),
    #[cfg(feature = "mock")]
    UnknownScenario(String),
    #[cfg(feature = "mock")]
    UnknownFailure(String),
    UnknownArgument(String),
    MissingValue(&'static str),
    /// A simulation flag without the simulation (it would otherwise start the real backend).
    MockFlagWithoutMock(&'static str),
    /// Demo data asked for, by the named flag or variable, in a build without it.
    NoDemoData(String),
    /// A debug build without demo data, started without `--real`: it has nothing safe to start
    /// on, and never starts the real backend unasked.
    NoDefault,
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::ConflictingFlags => write!(f, "pass at most one of --mock and --real"),
            BackendError::InvalidEnv(value) if Build::CURRENT.mock => {
                write!(
                    f,
                    "{BACKEND_ENV}={value:?} is not one of \"mock\" or \"real\""
                )
            }
            BackendError::InvalidEnv(value) => write!(f, "{BACKEND_ENV}={value:?} is not \"real\""),
            #[cfg(feature = "mock")]
            BackendError::UnknownScenario(value) => write!(
                f,
                "--mock-scenario {value:?} is not one of {}",
                names(SCENARIOS, ", ")
            ),
            #[cfg(feature = "mock")]
            BackendError::UnknownFailure(value) => write!(
                f,
                "--mock-fail {value:?} is not one of {}",
                names(FAILURES, ", ")
            ),
            BackendError::UnknownArgument(arg) => write!(f, "unknown argument {arg:?}"),
            BackendError::MissingValue(flag) => write!(f, "{flag} needs a value"),
            BackendError::MockFlagWithoutMock(flag) => {
                write!(f, "{flag} only applies to the simulation; add --mock")
            }
            BackendError::NoDemoData(asked) => {
                write!(
                    f,
                    "{asked} needs demo data, which this build does not include"
                )
            }
            BackendError::NoDefault => write!(
                f,
                "this debug build has no demo data to start on: add --real, or build it with the mock feature"
            ),
        }
    }
}

impl std::error::Error for BackendError {}

impl BackendKind {
    /// Debug builds default to mock, release builds to real (a hard safety rule in
    /// `docs/DESIGN.md`).
    pub fn build_default(build: Build) -> Result<Self, BackendError> {
        match (build.debug, build.mock) {
            (false, _) => Ok(BackendKind::Real),
            (true, true) => Ok(BackendKind::Mock),
            (true, false) => Err(BackendError::NoDefault),
        }
    }

    pub fn flavor(self) -> Flavor {
        match self {
            BackendKind::Mock => Flavor::Mock,
            BackendKind::Real => Flavor::Real,
        }
    }

    /// A `--mock`/`--real` flag wins over `env` (the value of [`BACKEND_ENV`]), which wins over
    /// [`Self::build_default`]. Other arguments are ignored. Pure, so tests never touch the
    /// process environment.
    pub fn resolve<S: AsRef<str>>(
        args: &[S],
        env: Option<&str>,
        build: Build,
    ) -> Result<Self, BackendError> {
        let mock = args.iter().any(|arg| arg.as_ref() == "--mock");
        let real = args.iter().any(|arg| arg.as_ref() == "--real");
        let asked = match (mock, real) {
            (true, true) if build.mock => return Err(BackendError::ConflictingFlags),
            (true, _) => "--mock".to_owned(),
            (false, true) => return Ok(BackendKind::Real),
            (false, false) => match env.map(str::trim) {
                None | Some("") => return Self::build_default(build),
                Some(value) if value.eq_ignore_ascii_case("real") => return Ok(BackendKind::Real),
                Some(value) if value.eq_ignore_ascii_case("mock") => {
                    format!("{BACKEND_ENV}={value}")
                }
                Some(value) => return Err(BackendError::InvalidEnv(value.to_owned())),
            },
        };
        if build.mock {
            Ok(BackendKind::Mock)
        } else {
            Err(BackendError::NoDemoData(asked))
        }
    }
}

/// The mock's starting world and injected failures, from `--mock-scenario` and `--mock-fail`.
#[cfg(feature = "mock")]
#[derive(Clone, Debug, PartialEq)]
pub struct MockFlags {
    pub scenario: MockScenario,
    pub failures: MockFailures,
}

#[cfg(feature = "mock")]
type StartWorld = fn() -> MockScenario;
#[cfg(feature = "mock")]
type Inject = fn(&mut MockFailures);

#[cfg(feature = "mock")]
const SCENARIOS: &[(&str, StartWorld)] = &[
    ("demo", MockScenario::demo),
    ("single", MockScenario::single),
    ("signed-out", MockScenario::signed_out),
];

#[cfg(feature = "mock")]
const FAILURES: &[(&str, Inject)] = &[
    ("cli-missing", |f| f.cli_missing = true),
    ("install-fails", |f| f.install_fails = true),
    ("install-off-path", |f| f.install_off_path = true),
    ("keyring", |f| f.keyring_unavailable = true),
    ("usage-fails", |f| f.usage_fetch_fails = true),
    ("login-invalid-code", |f| {
        f.login = Some(MockLoginFailure::InvalidCode)
    }),
    ("login-unverified", |f| {
        f.login = Some(MockLoginFailure::VerificationFailed)
    }),
    ("not-entitled", |f| {
        f.not_entitled = vec![Accelerator::A100, Accelerator::H100]
    }),
    ("no-answer", |f| {
        f.allocation_hangs = vec![Accelerator::A100]
    }),
    ("ssh-busy", |f| f.ssh_busy_attempts = 1),
    ("plugin-fails", |f| {
        f.plugin_fail_stage = Some("download".into())
    }),
    ("watchdog-restart-fails", |f| {
        f.watchdog_restart_fails = true
    }),
    ("update-available", |f| f.update_available = true),
    ("update-check-fails", |f| f.update_check_fails = true),
    ("update-unsupported", |f| f.update_unsupported = true),
    ("update-download-fails", |f| f.update_download_fails = true),
    ("update-unverified", |f| f.update_unverified = true),
    ("update-install-fails", |f| f.update_install_fails = true),
    ("runtimes-fail", |f| f.runtimes_listing_fails = true),
    ("plugin-unloadable", |f| f.plugin_unloadable = true),
];

#[cfg(feature = "mock")]
fn names<T>(table: &[(&str, T)], separator: &str) -> String {
    let names: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    names.join(separator)
}

#[cfg(feature = "mock")]
impl MockFlags {
    /// Reads `--mock-scenario NAME` (default `demo`) and `--mock-fail a,b`, as `--flag value`
    /// or `--flag=value`.
    pub fn parse<S: AsRef<str>>(args: &[S]) -> Result<Self, BackendError> {
        let mut flags = MockFlags {
            scenario: MockScenario::demo(),
            failures: MockFailures::default(),
        };
        if let Some(name) = flag_value(args, "--mock-scenario") {
            let Some((_, scenario)) = SCENARIOS.iter().find(|(known, _)| *known == name) else {
                return Err(BackendError::UnknownScenario(name));
            };
            flags.scenario = scenario();
        }
        if let Some(list) = flag_value(args, "--mock-fail") {
            for name in list
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                let Some((_, inject)) = FAILURES.iter().find(|(known, _)| *known == name) else {
                    return Err(BackendError::UnknownFailure(name.to_owned()));
                };
                inject(&mut flags.failures);
            }
        }
        Ok(flags)
    }
}

/// How to call the program, printed with any argument error.
#[cfg(feature = "mock")]
pub fn usage() -> String {
    format!(
        "usage: colab-playground [--real | --mock [--mock-scenario {}] [--mock-fail NAME[,NAME...]]]\n  --mock-fail names: {}\n{}",
        names(SCENARIOS, "|"),
        names(FAILURES, ", "),
        crate::control::ctl_usage()
    )
}

/// How to call the program, printed with any argument error.
#[cfg(not(feature = "mock"))]
pub fn usage() -> String {
    format!(
        "usage: colab-playground [--real]\n{}",
        crate::control::ctl_usage()
    )
}

const VALUE_FLAGS: [&str; 2] = ["--mock-scenario", "--mock-fail"];

/// Rejects anything but the known flags, and a value flag without a value. Runs before any
/// backend is chosen, so a mistyped `--mock` can never start the real backend.
pub fn check_args<S: AsRef<str>>(args: &[S]) -> Result<(), BackendError> {
    let mut iter = args.iter().map(AsRef::as_ref);
    while let Some(arg) = iter.next() {
        if arg == "--mock" || arg == "--real" {
            continue;
        }
        if platform::is_launcher_argument(arg) {
            continue;
        }
        if let Some(&flag) = VALUE_FLAGS.iter().find(|flag| **flag == arg) {
            match iter.next() {
                Some(value) if !value.trim().is_empty() && !value.starts_with("--") => continue,
                _ => return Err(BackendError::MissingValue(flag)),
            }
        }
        if let Some(&flag) = VALUE_FLAGS
            .iter()
            .find(|flag| arg.starts_with(&format!("{flag}=")))
        {
            if arg[flag.len() + 1..].trim().is_empty() {
                return Err(BackendError::MissingValue(flag));
            }
            continue;
        }
        return Err(BackendError::UnknownArgument(arg.to_owned()));
    }
    Ok(())
}

/// The backend the command line (and `env`, the value of [`BACKEND_ENV`]) asks `build` for,
/// after checking every argument. Simulation flags without the simulation are an error, and so
/// is any way of asking for demo data in a build without it.
pub fn kind_from_args<S: AsRef<str>>(
    args: &[S],
    env: Option<&str>,
    build: Build,
) -> Result<BackendKind, BackendError> {
    check_args(args)?;
    let demo_flag = VALUE_FLAGS
        .into_iter()
        .find(|flag| flag_value(args, flag).is_some());
    if let (Some(flag), false) = (demo_flag, build.mock) {
        return Err(BackendError::NoDemoData(flag.to_owned()));
    }
    match (BackendKind::resolve(args, env, build)?, demo_flag) {
        (BackendKind::Real, Some(flag)) => Err(BackendError::MockFlagWithoutMock(flag)),
        (kind, _) => Ok(kind),
    }
}

/// What the command line asks for, with every argument checked and nothing built yet: the
/// app first finds out whether another copy already owns this state, and only the owner
/// builds a backend.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    #[cfg(feature = "mock")]
    Mock(MockFlags),
    Real,
}

impl Launch {
    pub fn flavor(&self) -> Flavor {
        match self {
            #[cfg(feature = "mock")]
            Launch::Mock(_) => Flavor::Mock,
            Launch::Real => Flavor::Real,
        }
    }

    pub fn into_source(self) -> BackendSource {
        match self {
            #[cfg(feature = "mock")]
            Launch::Mock(flags) => BackendSource::interactive_mock(flags),
            Launch::Real => BackendSource::Real,
        }
    }
}

/// [`kind_from_args`] for this executable, with the simulation's flags read when it is chosen.
pub fn launch_from_args<S: AsRef<str>>(
    args: &[S],
    env: Option<&str>,
) -> Result<Launch, BackendError> {
    match kind_from_args(args, env, Build::CURRENT)? {
        BackendKind::Real => Ok(Launch::Real),
        #[cfg(feature = "mock")]
        BackendKind::Mock => Ok(Launch::Mock(MockFlags::parse(args)?)),
        #[cfg(not(feature = "mock"))]
        BackendKind::Mock => unreachable!("kind_from_args refuses demo data in a build without it"),
    }
}

fn flag_value<S: AsRef<str>>(args: &[S], flag: &str) -> Option<String> {
    let prefix = format!("{flag}=");
    let mut iter = args.iter().map(AsRef::as_ref);
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().map(str::to_owned);
        }
        if let Some(value) = arg.strip_prefix(&prefix) {
            return Some(value.to_owned());
        }
    }
    None
}

#[derive(Clone)]
pub enum BackendSource {
    /// Build a `RealBackend` from the settings (rebuilt when the WSL distro or `colab` path
    /// changes).
    Real,
    /// A ready simulation (the interactive one for `--mock`, a headless one in tests).
    Mock(Arc<dyn Backend>),
}

impl BackendSource {
    /// The interactive simulation for `--mock`: system clock, human-paced steps.
    #[cfg(feature = "mock")]
    pub fn interactive_mock(flags: MockFlags) -> Self {
        let mut config = MockConfig::interactive(MOCK_STEP_DELAY);
        config.scenario = flags.scenario;
        config.failures = flags.failures;
        BackendSource::Mock(Arc::new(MockBackend::new(config)))
    }

    pub fn flavor(&self) -> Flavor {
        match self {
            BackendSource::Real => Flavor::Real,
            BackendSource::Mock(_) => Flavor::Mock,
        }
    }
}

/// Where the real backend finds the CLI (from Settings).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostSetup {
    /// WSL distribution (Windows); `None` = the default one.
    pub wsl_distro: Option<String>,
    /// `colab` and its Python interpreter; `None` = found automatically.
    pub toolchain: Option<Toolchain>,
}

/// Builds the backend for `source`. A mock is returned as it is, with `defaults` pushed in.
pub fn build(
    source: &BackendSource,
    host: &HostSetup,
    defaults: &Defaults,
) -> colab_playground_core::Result<Arc<dyn Backend>> {
    match source {
        BackendSource::Mock(mock) => {
            mock.set_defaults(defaults.clone());
            Ok(mock.clone())
        }
        BackendSource::Real => {
            let mut config = RealConfig::for_app(defaults.clone())?;
            if let Host::Wsl { distro } = &mut config.host {
                distro.clone_from(&host.wsl_distro);
            }
            config.toolchain.clone_from(&host.toolchain);
            Ok(Arc::new(RealBackend::new(config)?))
        }
    }
}

/// Whether the shell shows the "Demo data" badge (the product principle in `docs/DESIGN.md`).
pub fn shows_demo_badge(flavor: Flavor) -> bool {
    flavor == Flavor::Mock
}

#[cfg(test)]
mod tests {
    use colab_playground_core::backend::BackendKind as Flavor;

    use super::{BackendError, BackendKind, Build, check_args, kind_from_args, shows_demo_badge};

    const DEBUG: Build = Build {
        mock: true,
        debug: true,
    };
    const RELEASE: Build = Build {
        mock: true,
        debug: false,
    };
    const DOWNLOAD: Build = Build {
        mock: false,
        debug: false,
    };
    const DEBUG_WITHOUT_MOCK: Build = Build {
        mock: false,
        debug: true,
    };

    #[test]
    fn flags_win_over_env_and_env_over_the_build_default() {
        let none: [&str; 0] = [];
        assert_eq!(
            BackendKind::resolve(&none, None, DEBUG),
            Ok(BackendKind::Mock)
        );
        assert_eq!(
            BackendKind::resolve(&none, Some(""), RELEASE),
            Ok(BackendKind::Real)
        );
        assert_eq!(
            BackendKind::resolve(&none, Some("real"), DEBUG),
            Ok(BackendKind::Real)
        );
        assert_eq!(
            BackendKind::resolve(&none, Some(" MOCK "), RELEASE),
            Ok(BackendKind::Mock)
        );
        assert_eq!(
            BackendKind::resolve(&["--mock"], Some("real"), RELEASE),
            Ok(BackendKind::Mock)
        );
        assert_eq!(
            BackendKind::resolve(&["--real"], Some("mock"), DEBUG),
            Ok(BackendKind::Real)
        );
        assert_eq!(
            BackendKind::resolve(&["--mock-scenario", "single", "--mock"], None, DEBUG),
            Ok(BackendKind::Mock)
        );
    }

    #[test]
    fn debug_builds_default_to_mock_and_release_builds_to_real() {
        assert_eq!(BackendKind::build_default(DEBUG), Ok(BackendKind::Mock));
        assert_eq!(BackendKind::build_default(RELEASE), Ok(BackendKind::Real));
        assert_eq!(BackendKind::build_default(DOWNLOAD), Ok(BackendKind::Real));
        assert_eq!(
            BackendKind::build_default(DEBUG_WITHOUT_MOCK),
            Err(BackendError::NoDefault),
            "without demo data a debug build has no safe default"
        );
    }

    #[test]
    fn a_build_without_demo_data_refuses_it_instead_of_starting_the_real_backend() {
        let none: [&str; 0] = [];
        assert_eq!(kind_from_args(&none, None, DOWNLOAD), Ok(BackendKind::Real));
        assert_eq!(
            kind_from_args(&["--mock"], None, DOWNLOAD),
            Err(BackendError::NoDemoData("--mock".into()))
        );
        assert_eq!(
            kind_from_args(&["--mock", "--real"], None, DOWNLOAD),
            Err(BackendError::NoDemoData("--mock".into())),
            "not a conflict with a flag the build does not have"
        );
        assert_eq!(
            kind_from_args(&none, Some("mock"), DOWNLOAD),
            Err(BackendError::NoDemoData(
                "COLAB_PLAYGROUND_BACKEND=mock".into()
            ))
        );
        assert_eq!(
            kind_from_args(&["--real", "--mock-scenario", "single"], None, DOWNLOAD),
            Err(BackendError::NoDemoData("--mock-scenario".into())),
            "not the advice to add --mock"
        );
        assert_eq!(
            kind_from_args(&["--mock-fail=keyring"], None, DEBUG_WITHOUT_MOCK),
            Err(BackendError::NoDemoData("--mock-fail".into()))
        );
        assert_eq!(
            kind_from_args(&none, None, DEBUG_WITHOUT_MOCK),
            Err(BackendError::NoDefault)
        );
        assert_eq!(
            kind_from_args(&none, Some("real"), DEBUG_WITHOUT_MOCK),
            Ok(BackendKind::Real)
        );
        assert_eq!(
            kind_from_args(&["--mok"], None, DOWNLOAD),
            Err(BackendError::UnknownArgument("--mok".into()))
        );
    }

    #[test]
    fn unknown_arguments_and_missing_values_are_errors() {
        assert_eq!(
            check_args(&["--mok"]),
            Err(BackendError::UnknownArgument("--mok".into())),
            "a typo of --mock is not ignored"
        );
        assert_eq!(
            check_args(&["--mock", "stray"]),
            Err(BackendError::UnknownArgument("stray".into()))
        );
        assert_eq!(
            check_args(&["--mock", "--mock-scenario"]),
            Err(BackendError::MissingValue("--mock-scenario"))
        );
        assert_eq!(
            check_args(&["--mock-scenario", "--mock"]),
            Err(BackendError::MissingValue("--mock-scenario")),
            "a flag is not a value"
        );
        assert_eq!(
            check_args(&["--mock", "--mock-fail="]),
            Err(BackendError::MissingValue("--mock-fail"))
        );
        assert!(
            check_args(&["--mock", "--mock-scenario", "single", "--mock-fail=keyring"]).is_ok()
        );
        assert!(check_args::<&str>(&[]).is_ok());
        assert!(check_args(&["--real"]).is_ok());

        // Simulation flags without the simulation would start the real backend.
        assert_eq!(
            kind_from_args(&["--mock-scenario", "single"], Some("real"), DEBUG),
            Err(BackendError::MockFlagWithoutMock("--mock-scenario"))
        );
        assert_eq!(
            kind_from_args(&["--mock-scenario", "single"], None, RELEASE),
            Err(BackendError::MockFlagWithoutMock("--mock-scenario"))
        );
        assert_eq!(
            kind_from_args(&["--mock-scenario", "single"], Some("mock"), RELEASE),
            Ok(BackendKind::Mock)
        );
        assert_eq!(
            kind_from_args(&["--real"], None, DEBUG),
            Ok(BackendKind::Real)
        );
    }

    #[test]
    fn conflicting_flags_and_unknown_env_values_fail() {
        assert_eq!(
            BackendKind::resolve(&["--mock", "--real"], None, DEBUG),
            Err(BackendError::ConflictingFlags)
        );
        assert_eq!(
            BackendKind::resolve(&[] as &[&str], Some("fake"), DOWNLOAD),
            Err(BackendError::InvalidEnv("fake".into()))
        );
    }

    #[cfg(feature = "mock")]
    #[test]
    fn mock_scenarios_and_failures_parse() {
        use colab_playground_core::mock::MockScenario;

        use super::{Launch, MockFlags, launch_from_args, usage};

        let none: [&str; 0] = [];
        assert_eq!(
            MockFlags::parse(&none).map(|flags| flags.scenario),
            Ok(MockScenario::demo())
        );
        let flags = MockFlags::parse(&[
            "--mock",
            "--mock-scenario=signed-out",
            "--mock-fail",
            "cli-missing, keyring",
        ])
        .expect("valid flags");
        assert_eq!(flags.scenario, MockScenario::signed_out());
        assert!(flags.failures.cli_missing && flags.failures.keyring_unavailable);
        assert!(!flags.failures.install_fails);
        assert_eq!(
            MockFlags::parse(&["--mock-scenario", "busy"]).map(|flags| flags.scenario),
            Err(BackendError::UnknownScenario("busy".into()))
        );
        assert_eq!(
            MockFlags::parse(&["--mock-fail=nope"]).map(|flags| flags.scenario),
            Err(BackendError::UnknownFailure("nope".into()))
        );
        let flags = MockFlags::parse(&["--mock", "--mock-fail", "watchdog-restart-fails"]).unwrap();
        assert!(flags.failures.watchdog_restart_fails);
        let flags =
            MockFlags::parse(&["--mock", "--mock-fail=update-available,update-check-fails"])
                .unwrap();
        assert!(flags.failures.update_available && flags.failures.update_check_fails);
        let flags = MockFlags::parse(&[
            "--mock",
            "--mock-fail=update-unsupported,update-download-fails,update-unverified,update-install-fails",
        ])
        .unwrap();
        let f = &flags.failures;
        assert!(
            f.update_unsupported
                && f.update_download_fails
                && f.update_unverified
                && f.update_install_fails
        );
        assert!(usage().contains("update-available"));
        assert!(usage().contains("watchdog-restart-fails"));

        assert!(matches!(
            launch_from_args(&["--mock", "--mock-scenario", "nope"], None),
            Err(BackendError::UnknownScenario(_))
        ));
        assert!(matches!(
            launch_from_args(&["--mock-scenario", "single"], Some("mock")),
            Ok(Launch::Mock(_))
        ));
        assert!(matches!(
            launch_from_args(&["--real"], None),
            Ok(Launch::Real)
        ));
    }

    #[test]
    fn only_the_simulation_carries_the_demo_badge() {
        assert!(shows_demo_badge(Flavor::Mock));
        assert!(!shows_demo_badge(Flavor::Real));
    }
}
