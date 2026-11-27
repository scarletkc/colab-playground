//! What the macOS menu bar extra would show, driven from the same state as the window.

use colab_playground::format;
use colab_playground_core::mock::MockScenario;

use super::support::{Options, TestApp};

#[gpui::test]
fn four_runtimes_share_one_line_with_the_burn_rate(cx: &mut gpui::TestAppContext) {
    let mut app = TestApp::new(cx);
    let lines = app.read(|state, cx| state.status_extra(cx));
    assert!(
        lines.text.starts_with("4 ·"),
        "count then rate, got {}",
        lines.text
    );
}

#[gpui::test]
fn two_runtimes_name_both_gpus_then_the_total_rate(cx: &mut gpui::TestAppContext) {
    let mut scenario = MockScenario::demo();
    scenario.runtimes.truncate(2);
    scenario.accounts.truncate(1);
    let mut app = TestApp::with(cx, Options::scenario(scenario));
    let lines = app.read(|state, cx| state.status_extra(cx));
    let rate = app.read(|state, cx| format::cu_rate(cx, state.usage_summary().rate_cu_per_hour));
    assert_eq!(lines.text, format!("A100 T4 · {rate}"));
}

#[gpui::test]
fn no_runtime_still_shows_the_count_and_rate(cx: &mut gpui::TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    let lines = app.read(|state, cx| state.status_extra(cx));
    assert_eq!(lines.text, "0 · 0.00 CU/h");
}
