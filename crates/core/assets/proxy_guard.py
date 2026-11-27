"""Runs `colab ssh --proxy-mode` for the app's tunnels so that it cannot create a runtime.

The tunnel's ProxyCommand starts this with the CLI's own interpreter:
`<python> -P -c <loader> <this file in base64> --config STATE ssh --proxy-mode -s NAME`.

`--proxy-mode -s NAME` creates a runtime when NAME is not in the state file. The tunnel
checks the record before every connect, but the CLI reads the file only once it has
started, so a record dropped in between would still start a billed runtime. Every
allocation in the CLI goes through `Client.assign` (google-colab-cli 0.8.0), so the CLI
runs here with that method replaced by a refusal, and a CLI without it does not run.
Stdout is the ssh byte stream: whatever the tunnel should read goes to stderr.
"""

import os
import sys

# The tunnel matches these (crates/core/src/tunnel.rs).
REFUSED = "[colab-playground] refused to create a runtime"
UNSUPPORTED = "[colab-playground] unsupported colab CLI"


def refuse(*_args, **_kwargs):
    sys.stderr.write(REFUSED + ": the session is not recorded, and a tunnel never creates one\n")
    sys.stderr.flush()
    # An exception could be caught on its way out of the CLI; this ends the process here.
    os._exit(3)


def main():
    try:
        from colab_cli.client import Client  # noqa: PLC0415
    except Exception as exc:  # noqa: BLE001
        sys.stderr.write("%s: colab_cli.client (%s)\n" % (UNSUPPORTED, type(exc).__name__))
        return 3
    if not callable(getattr(Client, "assign", None)):
        sys.stderr.write(UNSUPPORTED + ": no Client.assign\n")
        return 3
    # Replaced before the rest of the CLI is imported, so no part of it can hold on to the original.
    Client.assign = refuse
    try:
        from colab_cli.cli import main as colab  # noqa: PLC0415
    except Exception as exc:  # noqa: BLE001
        sys.stderr.write("%s: colab_cli.cli (%s)\n" % (UNSUPPORTED, type(exc).__name__))
        return 3
    # What the `colab` entry script runs, with the name it is run under.
    sys.argv[0] = "colab"
    return colab()


if __name__ == "__main__":
    sys.exit(main())
