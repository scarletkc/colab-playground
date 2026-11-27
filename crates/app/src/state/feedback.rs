//! Asking for a star or feedback, once the app has proved useful.
//!
//! The ask waits until [`READY_BEFORE_ASKING`] runtimes or deployments have come up, then comes
//! at a pause: just after the user stopped a runtime, with nothing else billing or starting and
//! no other dialog open. It comes at most [`MOST_ASKS`] times, each no sooner than
//! [`ASK_AGAIN_AFTER`] after the one before, and never again once the user opened either link
//! from it. Both links only open the browser; the app sends nothing.

use std::time::Duration;

use gpui::Context;

use super::{AppState, Op, Overlay, updates::secs};
use crate::{platform, settings::Settings};

/// Runtimes and deployments that must have come up before the app asks.
pub const READY_BEFORE_ASKING: u32 = 3;
pub const MOST_ASKS: u32 = 2;
/// How long "Not now" puts the next ask off.
pub const ASK_AGAIN_AFTER: Duration = Duration::from_secs(30 * 24 * 3600);

/// The project's page on GitHub (the package repository), where it can be starred.
pub const PROJECT_PAGE: &str = env!("CARGO_PKG_REPOSITORY");

/// Ids of text fields that every issue form in `.github/ISSUE_TEMPLATE/` has. Released apps keep
/// linking them, so renaming one leaves it empty for every user of those versions.
const VERSION_FIELD: &str = "version";
const SYSTEM_FIELD: &str = "os";

/// GitHub's choice of issue forms, with this build's version and system filled in: GitHub passes
/// the fields given to the chooser on to the form the user picks.
pub fn feedback_page() -> String {
    let fields = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(VERSION_FIELD, env!("CARGO_PKG_VERSION"))
        .append_pair(SYSTEM_FIELD, &platform::system())
        .finish();
    format!("{PROJECT_PAGE}/issues/new/choose?{fields}")
}

/// Whether the app may ask at `now` (seconds since the Unix epoch).
fn due(settings: &Settings, now: u64) -> bool {
    !settings.feedback_answered
        && settings.ready_count >= READY_BEFORE_ASKING
        && settings.feedback_asked < MOST_ASKS
        && settings
            .feedback_asked_at
            .is_none_or(|at| now >= at.saturating_add(ASK_AGAIN_AFTER.as_secs()))
}

impl AppState {
    /// A runtime or deployment came up.
    pub(crate) fn count_ready(&mut self, cx: &mut Context<Self>) {
        self.settings.ready_count = self.settings.ready_count.saturating_add(1);
        self.persist_settings(cx);
    }

    /// The user's stop ended a runtime and a listing was asked for. The list shown can still
    /// lack a runtime that came up meanwhile (a start that finished during the stop, one opened
    /// in the browser), and that one bills, so an ask that is due waits for that listing.
    pub(crate) fn ask_for_feedback_after_stop(&mut self, cx: &mut Context<Self>) {
        if !self
            .backend_now()
            .map(secs)
            .is_some_and(|now| due(&self.settings, now))
        {
            return;
        }
        self.feedback_waits_for_listing = self.ops.is_running(Op::Runtimes);
        if !self.feedback_waits_for_listing {
            self.ask_for_feedback_if_due(cx);
        }
    }

    /// A runtime listing is back: a stop waiting for it asks now, if it may. A listing that
    /// another follows began before the stop ended, so the stop waits for that one.
    pub(crate) fn listed_for_feedback(&mut self, cx: &mut Context<Self>) {
        if self.ops.is_running(Op::Runtimes) {
            return;
        }
        if std::mem::take(&mut self.feedback_waits_for_listing) {
            self.ask_for_feedback_if_due(cx);
        }
    }

    /// Asks for a star or feedback when it is due and nothing else needs the user.
    fn ask_for_feedback_if_due(&mut self, cx: &mut Context<Self>) {
        let Some(now) = self.backend_now().map(secs) else {
            return;
        };
        let busy = self.overlay.is_some() || self.dialog.is_some() || self.billing().any();
        if busy || !due(&self.settings, now) {
            return;
        }
        self.settings.feedback_asked += 1;
        self.settings.feedback_asked_at = Some(now);
        log::info!(
            "feedback: asking for a star or feedback ({} of {MOST_ASKS})",
            self.settings.feedback_asked
        );
        self.persist_settings(cx);
        self.overlay = Some(Overlay::Feedback);
        cx.notify();
    }

    /// Whether the app will not ask again after this time, so dismissing can say so.
    pub fn feedback_asked_for_the_last_time(&self) -> bool {
        self.settings.feedback_asked >= MOST_ASKS
    }

    /// Opens the project page from the prompt: GitHub has no link that stars a repository, so
    /// the user stars it there.
    pub fn star_from_prompt(&mut self, cx: &mut Context<Self>) {
        self.answer_prompt(PROJECT_PAGE, cx);
    }

    pub fn give_feedback_from_prompt(&mut self, cx: &mut Context<Self>) {
        self.answer_prompt(&feedback_page(), cx);
    }

    fn answer_prompt(&mut self, url: &str, cx: &mut Context<Self>) {
        cx.open_url(url);
        self.settings.feedback_answered = true;
        self.persist_settings(cx);
        self.close_overlay(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        ASK_AGAIN_AFTER, MOST_ASKS, PROJECT_PAGE, READY_BEFORE_ASKING, SYSTEM_FIELD, VERSION_FIELD,
        due, feedback_page,
    };
    use crate::settings::Settings;

    fn used(ready_count: u32) -> Settings {
        Settings {
            ready_count,
            ..Settings::default()
        }
    }

    #[test]
    fn it_waits_until_a_few_runtimes_came_up() {
        assert!(!due(&used(READY_BEFORE_ASKING - 1), 0));
        assert!(due(&used(READY_BEFORE_ASKING), 0));
    }

    #[test]
    fn not_now_puts_the_next_ask_off_and_the_asks_end() {
        let asked = 1_767_225_600;
        let later = ASK_AGAIN_AFTER.as_secs();
        let mut settings = Settings {
            feedback_asked: 1,
            feedback_asked_at: Some(asked),
            ..used(READY_BEFORE_ASKING)
        };
        assert!(!due(&settings, asked + later - 1));
        assert!(due(&settings, asked + later));
        settings.feedback_asked = MOST_ASKS;
        assert!(!due(&settings, asked + later * 10));
    }

    #[test]
    fn an_answer_ends_the_asks() {
        let settings = Settings {
            feedback_answered: true,
            ..used(READY_BEFORE_ASKING)
        };
        assert!(!due(&settings, 0));
    }

    #[test]
    fn the_issue_forms_get_the_version_and_the_system() {
        let page = feedback_page();
        let query = page
            .strip_prefix(&format!("{PROJECT_PAGE}/issues/new/choose?"))
            .expect("the project's choice of issue forms");
        let fields: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(
            fields,
            [
                (
                    VERSION_FIELD.to_owned(),
                    env!("CARGO_PKG_VERSION").to_owned()
                ),
                (SYSTEM_FIELD.to_owned(), crate::platform::system()),
            ]
        );
    }

    #[test]
    fn every_issue_form_has_the_fields_the_link_fills_in() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/ISSUE_TEMPLATE");
        let mut forms = 0;
        for entry in std::fs::read_dir(&dir).expect("the issue forms") {
            let path = entry.expect("an issue form").path();
            if path.file_name() == Some("config.yml".as_ref()) {
                continue;
            }
            let form = std::fs::read_to_string(&path).expect("a readable issue form");
            for id in [VERSION_FIELD, SYSTEM_FIELD] {
                let field = format!("id: {id}");
                assert!(
                    form.lines().any(|line| line.trim() == field),
                    "{} has no `{id}` field",
                    path.display()
                );
            }
            forms += 1;
        }
        assert!(forms > 0, "no issue forms in {}", dir.display());
    }
}
