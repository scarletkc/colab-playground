//! Colab Playground's desktop shell: GPUI + Ely GPUI Components over `colab-playground-core`.
//!
//! # How the app is put together (read this before adding a feature)
//!
//! One [`state::AppState`] entity owns everything the UI shows: the current [`route::Route`],
//! the persisted [`settings::Settings`] (including core's `Defaults`), the backend
//! (`Arc<dyn Backend>`, built from a [`backend::BackendSource`]), what it last reported
//! (probe, accounts, usage), the open account dialog and the toast host. `AppState::init`
//! creates it and publishes it as a global ([`state::AppState::global`]); views read it in
//! `render` and change it only through its methods, which call `cx.notify()` so every observing
//! view re-renders.
//! Backend calls block, so once the window is up the UI thread makes none (the one exception is
//! startup, where `backend::build` constructs the backend and pushes the saved `Defaults`;
//! a rebuild after a host change runs in the background): `AppState::run_backend` runs the call on the
//! background executor with a `CancelToken` and applies the result on the foreground; at most
//! one call of each `state::Op` runs at a time, cancelling flips the token (a dropped `Task`
//! does not stop a blocking call), and streamed events arrive through
//! `run_backend_streaming`. Errors are words only in [`errors`], which maps core's typed errors
//! and next steps to catalog messages and buttons. Every user-visible string is a key in
//! [`i18n::keys`] with a message in all three catalogs; colours, radii and sizes come from
//! `cx.theme()` (sizes it has no token for, from [`ui::metrics`]); platform-specific code lives
//! only in [`platform`]. Tests drive the real views headlessly through `tests/ui/support`,
//! always over a headless `MockBackend`.

pub mod app;
pub mod backend;
pub mod control;
pub mod errors;
pub mod format;
pub mod i18n;
pub mod platform;
pub mod route;
pub mod settings;
pub mod state;
pub mod ui;
