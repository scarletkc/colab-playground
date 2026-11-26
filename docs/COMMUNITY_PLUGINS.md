# Community plugins

Plugins written by other people, each in a repository of its own. To install one, paste its
repository URL into **Install a plugin** on the app's Plugins page; the app shows what the plugin
asks for and what it receives before anything is installed.

**Being listed is not a guarantee.** When a plugin is added, its author states that it meets the
rules below, and nobody else runs it. The app installs whatever the repository holds on the day you
install it, so a plugin can change after it was listed. Its script runs on your paid runtime and
receives what you type into its form, plus your saved Hugging Face token if it asks for it
([Security model](PLUGINS.md#security-model)). Install only plugins you trust.

| Plugin | What it does | Tested on | Receives |
| --- | --- | --- | --- |

## Getting a plugin listed

Open a pull request titled `docs: list <plugin name>` that adds one row to the table, in
alphabetical order of the name:

- **Plugin:** the English name from `[name]` in its manifest, linked to the repository's `https://`
  URL, which is what users paste to install it.
- **What it does:** one sentence, in the user's words.
- **Tested on:** the accelerators you ran it on, its default `accelerator` first.
- **Receives:** `Its form`, or `Its form and your Hugging Face token` for `needs = ["hf_token"]`.

A plugin is listed when:

- **It installs from that URL:** `plugin.toml` and its script are at the repository's root
  ([Layout](PLUGINS.md#layout)). Its `id` is not taken by a listed or a bundled plugin, because
  two plugins with one `id` cannot be installed side by side.
- **You ran it on a real Colab runtime** with the latest release, on every accelerator in its row,
  and it reached `ready`.
- **Its default `accelerator` is the cheapest one that runs its default settings**, since that is
  what users pay for unless they change it.
- **It installs a fixed release of the software it runs**, not the tip of a branch, so it keeps
  working the way it did when you ran it.
- **It follows [PLUGINS.md](PLUGINS.md).** In particular, it touches the heartbeat only while
  there is work, so an idle stop still fires, and a token reaches only the service it is for.
- **Its README says what it does on the runtime:** what it downloads and installs, the ports it
  serves and what it does with a token.
- **It has a licence.**

## Removal

A plugin leaves the list when it no longer installs or works with the latest release, when its
repository is gone or archived, or when it breaks a rule above. Report one in an issue. Its author
can have it removed at any time.
