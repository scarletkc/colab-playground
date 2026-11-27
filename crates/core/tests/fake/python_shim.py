#!/usr/bin/env python3
"""Fake "CLI interpreter" for the account helper and the tunnel's proxy guard in integration tests.

Invoked exactly like the real one (``<python> -c <loader> <op> [args...]`` for the
helper, ``<python> -P -c <loader> <guard> [args...]`` in a ProxyCommand), it installs
fake ``keyring`` / ``google-auth`` / ``requests`` / ``colab_cli`` modules and then
runs the loader, which executes the *real* ``account_helper.py`` read from stdin or
the real ``proxy_guard.py`` passed in base64. Never resolves the system's packages
(see fake_common).
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fake_common  # noqa: E402

fake_common.install_fake_modules()

args = sys.argv[1:]
# A ProxyCommand passes `-P` first; the shim's module path does not depend on it.
if args[:1] == ["-P"]:
    args = args[1:]
if len(args) < 2 or args[0] != "-c":
    sys.stderr.write("fake python: only `[-P] -c CODE` is supported\n")
    sys.exit(2)

code = args[1]
sys.argv = ["-c"] + args[2:]
exec(compile(code, "<string>", "exec"), {"__name__": "__main__"})
