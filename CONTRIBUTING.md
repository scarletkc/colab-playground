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

Only fixes are accepted, each for a problem described in an issue.

- **No feature pull requests.** A pull request that adds a feature or changes how one works is
  closed without review. Suggest the feature in an issue instead, or build it as a plugin in a
  repository of your own ([PLUGINS.md](docs/PLUGINS.md)) and get it listed in
  [COMMUNITY_PLUGINS.md](docs/COMMUNITY_PLUGINS.md).
- **A fix starts from an issue.** Open one first, or pick an existing one, and link it in the pull
  request with `Fixes #<number>`. A pull request with no issue is closed. This covers corrections
  to the docs too.
- **The one exception:** a pull request that adds your plugin to the community list needs no
  issue; it follows [Getting a plugin listed](docs/COMMUNITY_PLUGINS.md#getting-a-plugin-listed).

A fix follows the rules every change here follows:

- the pull request's title is a conventional commit, because it becomes the changelog line, and
  uses this repository's own types
  ([Writing commits for the changelog](docs/RELEASING.md#writing-commits-for-the-changelog));
  how a pull request lands is in [Landing a change](docs/DEVELOPMENT.md#landing-a-change);
- the product principle, the scope, the hard safety rules and the invariants in
  [DESIGN.md](docs/DESIGN.md);
- [Docs and comments](docs/DEVELOPMENT.md#docs-and-comments);
- CI runs the whole test suite on the pull request, so locally run only the checks of what you
  changed ([A check is made once](docs/TESTING.md#a-check-is-made-once)).

If the fix changes the interface, attach screenshots of the running app to the pull request
([Screenshots](docs/TESTING.md#screenshots)). Nobody else runs your change on a real Colab
runtime: if it changes what happens there, say how you tried it.

A coding agent working in this repository reads [AGENTS.md](AGENTS.md).
