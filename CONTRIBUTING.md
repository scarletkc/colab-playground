# Contributing to Colab Playground

This page is for people outside the project. Thank you for taking the time.

## Issues

Issues are welcome, for bugs and ideas alike.

- **A bug:** in the app, Settings → About → "Give feedback" opens a new issue with the app's
  version and operating system filled in ([Feedback](docs/USAGE.md#feedback)). Say what you did,
  what you expected and what happened instead.
- **An idea:** describe the problem it solves for you. [ROADMAP.md](docs/ROADMAP.md) lists what
  is already planned and why it is not built yet, and
  [DESIGN.md](docs/DESIGN.md#goals-and-non-goals-v1) what is out of scope.

## Pull requests

From outside the project, only fixes are accepted.

- **A fix starts from an issue.** Open one first, or pick an existing one, and link it in the pull
  request with `Fixes #<number>`. A pull request with no issue is closed. The fix may include a
  test for the problem it fixes.
- **Slowness is a problem to fix when a user notices it**, such as high CPU use while the app is
  idle. The issue says how you measured it, and the pull request shows the numbers before and
  after.
- **So is a mistake in the docs.** A typo or a broken link needs no issue. A wrong statement does,
  so that it can be checked first.
- **Everything else is closed without review:** features and changes to how one works,
  refactoring, speed-ups for a slowness nobody reported, and new or rewritten docs. Suggest a
  feature in an issue instead, or build it as a plugin in a repository of your own
  ([PLUGINS.md](docs/PLUGINS.md)) and get it listed in
  [COMMUNITY_PLUGINS.md](docs/COMMUNITY_PLUGINS.md).
- **A plugin listing needs no issue:** it follows
  [Getting a plugin listed](docs/COMMUNITY_PLUGINS.md#getting-a-plugin-listed).

A fix follows the same rules as every change here:

- **Title:** a conventional commit with this repository's own types, because it becomes the
  changelog line
  ([Writing commits for the changelog](docs/RELEASING.md#writing-commits-for-the-changelog)).
- **Design:** the product principle, the scope, the hard safety rules and the invariants in
  [DESIGN.md](docs/DESIGN.md).
- **Docs and comments:** written as
  [DEVELOPMENT.md](docs/DEVELOPMENT.md#docs-and-comments) says.
- **Checks:** CI runs the whole test suite on the pull request, so locally run only the checks of
  what you changed ([A check is made once](docs/TESTING.md#a-check-is-made-once)).
- **Screenshots:** if the fix changes the interface, attach screenshots of the running app to the
  pull request ([Screenshots](docs/TESTING.md#screenshots)).
- **Real Colab:** the maintainers do not run outside changes on a real Colab runtime, and no test
  does. If the fix changes what happens there, say in the pull request how you tried it.

The steps from branch to merge are in [Landing a change](docs/DEVELOPMENT.md#landing-a-change).
If you use a coding agent, it finds these rules in [AGENTS.md](AGENTS.md).
