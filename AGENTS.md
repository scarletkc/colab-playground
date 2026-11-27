# Agent notes for Colab Playground

What a coding agent must know before changing this repository. Rules have their home in
[`docs/`](docs): this file points there instead of repeating them, and the doc wins when the two
disagree. The few rules that exist only here say so.

## Never

- All [hard safety rules](docs/DESIGN.md#hard-safety-rules) apply to agents, including how to
  clean up after a real `colab` command.
- One of them is easy to break by accident: the app runs on the real backend only when you are
  asked to run it there, and a release build is on it unless `--mock` is given.
- Never push to `main`. Stated only here: never use the admin bypass of the `main` ruleset unless
  asked.
- Stated only here: no `Co-Authored-By` trailer that credits a coding agent (Claude, Devin and
  the like) in commits or pull requests.

## Build, run, check

- Debug builds run on the mock backend, which needs no WSL, CLI or Google account:
  [Build and run](docs/USAGE.md#build-and-run), [Demo data](docs/USAGE.md#demo-data).
- A check is made once. CI runs the whole suite on every pull request, so locally run only the
  checks of what you changed, and nothing waits for a person to check it by hand:
  [A check is made once](docs/TESTING.md#a-check-is-made-once).
- The tests of the real backend do not run on Windows:
  [Automated tests](docs/TESTING.md#automated-tests).
- A change to the interface is confirmed in screenshots, not by the headless UI tests:
  [Screenshots](docs/TESTING.md#screenshots).

## Rules every change follows

- [Product principle](docs/DESIGN.md#product-principle-the-tie-breaker): the tie-breaker when the
  docs are silent or two options compete.
- [Goals and non-goals](docs/DESIGN.md#goals-and-non-goals-v1) and
  [Invariants](docs/DESIGN.md#invariants): a change that breaks one is wrong even if every test
  passes. If a rule cannot be followed, stop and report.
- Part of these rules is enforced by the checks:
  [`crates/app/clippy.toml`](crates/app/clippy.toml) names what to use instead of each banned
  call, and [`crates/app/tests/lint.rs`](crates/app/tests/lint.rs) rejects unnamed numbers in UI
  code and a target OS named outside the modules that own it.
- [Docs and comments](docs/DEVELOPMENT.md#docs-and-comments): the code wins over a description, no
  doc describes how the code works, comments say why.

## Deciding on your own

Stated only here.

- A point nobody decided is yours to decide, not a reason to stop. Choose what a normal user
  expects and what is logically sound, as the
  [product principle](docs/DESIGN.md#product-principle-the-tie-breaker) says, or what comparable
  products from the major vendors do.
- A small leftover problem outside the change is fixed along with it. A larger one is reported
  and left alone.
- Report every such decision and fix when you finish: what you chose or fixed, and why.

## Landing a change

- Pull request against `main`, squash-merged; its title is a conventional commit and becomes the
  changelog line: [Landing a change](docs/DEVELOPMENT.md#landing-a-change). The types are the
  repository's own, not the usual set:
  [Writing commits for the changelog](docs/RELEASING.md#writing-commits-for-the-changelog).
- Review bot comments are leads, not verdicts, and only a problem that can really happen gets
  fixed: [Automated review comments](docs/DEVELOPMENT.md#automated-review-comments).

## Read before touching

- The core or the bundled `llama-cpp` plugin: [COLAB_NOTES.md](docs/COLAB_NOTES.md), how Colab and
  its CLI behave.
- The plugin system or a manifest: [PLUGINS.md](docs/PLUGINS.md), the contract with plugin authors.
- Workflows, the version or the changelog: [RELEASING.md](docs/RELEASING.md).
- A new feature: [ROADMAP.md](docs/ROADMAP.md), what is planned and why it is not built yet.
