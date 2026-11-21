use ely_gpui_component::primitives::IconName;

use crate::i18n::keys;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Route {
    /// First-run checklist and quick actions.
    #[default]
    Overview,
    /// Colab runtimes.
    Sessions,
    /// Installed plugins and their runs.
    Plugins,
    /// Deployments started before, to start again.
    Deployments,
    /// Google accounts.
    Accounts,
    Settings,
}

impl Route {
    /// Every route, in sidebar order.
    pub const ALL: [Route; 6] = [
        Route::Overview,
        Route::Sessions,
        Route::Plugins,
        Route::Deployments,
        Route::Accounts,
        Route::Settings,
    ];

    /// A stable lowercase name, used in element ids and test selectors (`nav-<slug>`).
    pub fn slug(self) -> &'static str {
        match self {
            Route::Overview => "overview",
            Route::Sessions => "sessions",
            Route::Plugins => "plugins",
            Route::Deployments => "deployments",
            Route::Accounts => "accounts",
            Route::Settings => "settings",
        }
    }

    /// The catalog key of its name (sidebar entry and page title).
    pub fn title_key(self) -> &'static str {
        match self {
            Route::Overview => keys::NAV_OVERVIEW,
            Route::Sessions => keys::NAV_SESSIONS,
            Route::Plugins => keys::NAV_PLUGINS,
            Route::Deployments => keys::NAV_DEPLOYMENTS,
            Route::Accounts => keys::NAV_ACCOUNTS,
            Route::Settings => keys::NAV_SETTINGS,
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Route::Overview => IconName::LayoutDashboard,
            Route::Sessions => IconName::Server,
            Route::Plugins => IconName::Puzzle,
            Route::Deployments => IconName::History,
            Route::Accounts => IconName::Users,
            Route::Settings => IconName::Settings,
        }
    }

    /// The selector the sidebar entry carries in test builds.
    pub fn nav_selector(self) -> &'static str {
        match self {
            Route::Overview => "nav-overview",
            Route::Sessions => "nav-sessions",
            Route::Plugins => "nav-plugins",
            Route::Deployments => "nav-deployments",
            Route::Accounts => "nav-accounts",
            Route::Settings => "nav-settings",
        }
    }
}
