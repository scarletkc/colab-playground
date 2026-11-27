use anyhow::Context as _;
use ely_gpui_component::Assets;
use gpui::{
    App, AppContext, Bounds, Entity, Global, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions,
};

use crate::{
    i18n::{keys, tr},
    platform,
    state::{AppState, StateOptions},
    ui::{
        metrics::{WINDOW_MIN_SIZE, WINDOW_SIZE},
        root::RootView,
    },
};

struct OpeningMainWindow;

impl Global for OpeningMainWindow {}

/// Brings the window forward, or opens it again after the traffic lights closed it.
pub(crate) fn show_or_open_main_window(cx: &mut App) {
    if let Some(handle) = cx.windows().first().copied() {
        let _ = cx.update_window(handle, |_, window, _| window.activate_window());
        cx.activate(true);
        return;
    }
    if cx.has_global::<OpeningMainWindow>() {
        return;
    }
    cx.set_global(OpeningMainWindow);
    let opened = open_main_window(AppState::global(cx), cx);
    cx.remove_global::<OpeningMainWindow>();
    if let Err(error) = opened {
        log::error!("app: {error:#}");
        return;
    }
    cx.activate(true);
}

/// Runs the app until it quits.
pub fn run(options: StateOptions) {
    let app = gpui_platform::application().with_assets(Assets);
    app.on_reopen(|cx| show_or_open_main_window(cx));
    app.run(move |cx: &mut App| {
        if let Err(error) = ely_gpui_component::init(cx) {
            log::error!("app: Ely failed to start: {error:#}");
            cx.quit();
            return;
        }
        let state = AppState::init(options, cx);
        platform::install_status_extra(cx, |cx| show_or_open_main_window(cx));
        cx.observe(&state, |state, cx| {
            let lines = state.read(cx).status_extra(cx);
            platform::refresh_status_extra(&lines, cx);
        })
        .detach();
        let lines = state.read(cx).status_extra(cx);
        platform::refresh_status_extra(&lines, cx);
        if let Err(error) = open_main_window(state, cx) {
            log::error!("app: {error:#}");
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() && !platform::keeps_running_without_windows(cx) {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

/// Options for the main window: centred, with a minimum size, and no OS title bar (the title
/// is still set for the taskbar and window switcher; the app draws its own strip, see
/// `ui::titlebar`). Decorations, background and traffic lights per OS come from `platform`.
pub fn window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, WINDOW_SIZE, cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(WINDOW_MIN_SIZE),
        titlebar: Some(TitlebarOptions {
            title: Some(tr(cx, keys::APP_NAME)),
            appears_transparent: true,
            traffic_light_position: platform::traffic_light_position(cx),
        }),
        window_decorations: platform::window_decorations(),
        window_background: platform::window_background(),
        app_id: Some("colab-playground".into()),
        ..Default::default()
    }
}

pub fn open_main_window(
    state: Entity<AppState>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<RootView>> {
    let options = window_options(cx);
    cx.open_window(options, |window, cx| {
        window.on_next_frame(|_, _| log::info!("app: first frame drawn"));
        cx.new(|cx| RootView::new(state, window, cx))
    })
    .context("the main window failed to open")
}
