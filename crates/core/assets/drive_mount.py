"""Runs `colab drivemount` for the app, which shows Google's consent page and answers for the user.

The app starts this with the CLI's own interpreter:
`<python> -P -c <loader> <this file in base64> --config STATE drivemount -s NAME`.

`colab drivemount` (google-colab-cli 0.8.0) prints the URL of the consent page, tries to open a
browser, and waits for Enter on /dev/tty, else on stdin, before it hands the consent to Colab.
Here the URL comes out as one line for the app and no browser opens, and Enter is read from stdin
alone: the app writes `continue` there once the user says access is allowed, or `cancel`.

The CLI answers the kernel's request for the consent only when handing it over worked. Left
unanswered, the kernel waits two minutes and then mounts Drive without it, and every `colab exec`
on the runtime waits behind it. Here the kernel gets an error whenever the CLI leaves it
unanswered or the user cancels, so `drive.mount` ends at once.

Every line meant for the app goes to stdout and starts with `[colab-playground]`.
"""

import builtins
import errno
import sys

# The app matches these (crates/core/src/drive.rs).
CONSENT = "[colab-playground] drive consent:"
NOT_HANDED_OVER = "[colab-playground] drive consent not handed over"
CANCELLED = "[colab-playground] drive mount cancelled"
FAILED = "[colab-playground] drive consent failed:"
UNSUPPORTED = "[colab-playground] unsupported colab CLI"

# What the kernel gets instead of the consent: `drive.mount` raises it as a MessageError.
REFUSAL = "Google Drive access was not given"


def say(line):
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


class Cancelled(BaseException):
    """Raised through the CLI's hook, whose `except Exception` clauses would swallow anything less."""


class Answers:
    """Stdin as the CLI's wait for Enter reads it: `continue` is Enter, anything else cancels."""

    def readline(self, *_args):
        line = sys.__stdin__.readline()
        # Ends the CLI's prompt line, as Enter in a terminal does.
        say("")
        if line.strip() == "continue":
            return "\n"
        # `cancel`, or end of input: the app is gone.
        raise Cancelled()

    def __getattr__(self, name):
        return getattr(sys.__stdin__, name)


def show_consent(uri):
    say("%s %s" % (CONSENT, uri))
    # The CLI's own version returns whether it opened a browser.
    return False


def no_terminal(file, *args, **kwargs):
    if file == "/dev/tty":
        raise OSError(errno.ENXIO, "the app answers on stdin")
    return builtins.open(file, *args, **kwargs)


class _Channel:
    """The kernel's stdin channel, noting whether the CLI answered the request."""

    def __init__(self, channel):
        self._channel = channel
        self.answered = False

    def send(self, message):
        self.answered = True
        return self._channel.send(message)

    def __getattr__(self, name):
        return getattr(self._channel, name)


class _Client:
    """The kernel client as the CLI's hook sees it, with the stdin channel watched."""

    def __init__(self, client):
        self._client = client
        self.stdin_channel = _Channel(client.stdin_channel)

    def __getattr__(self, name):
        return getattr(self._client, name)


def refuse(message, client):
    """Answers the kernel's request with an error, which ends `drive.mount`."""
    value = {
        "type": "colab_reply",
        "colab_msg_id": message.get("metadata", {}).get("colab_msg_id"),
        "error": REFUSAL,
    }
    reply = client.session.msg("input_reply", {"value": value})
    if "header" in message:
        reply["parent_header"] = message["header"]
    client.stdin_channel.send(reply)


def answering(hook):
    """`hook`, made to leave no request of the kernel unanswered."""

    def run(message, client):
        watched = _Client(client)
        try:
            handled = hook(message, watched)
        except Cancelled:
            refuse(message, client)
            say(CANCELLED)
            return True
        except Exception as exc:  # noqa: BLE001
            refuse(message, client)
            say("%s %s: %s" % (FAILED, type(exc).__name__, exc))
            return True
        if handled and not watched.stdin_channel.answered:
            refuse(message, client)
            say(NOT_HANDED_OVER)
        return handled

    return run


def guarded(runtime_class):
    """The CLI's runtime class, with every hook it is given made `answering`."""

    class Runtime(runtime_class):
        @property
        def colab_request_hook(self):
            return self.__dict__.get("_cpg_hook")

        @colab_request_hook.setter
        def colab_request_hook(self, hook):
            self.__dict__["_cpg_hook"] = answering(hook) if callable(hook) else hook

    return Runtime


def main():
    try:
        from colab_cli.commands import automation  # noqa: PLC0415
    except Exception as exc:  # noqa: BLE001
        say("%s: colab_cli.commands.automation (%s)" % (UNSUPPORTED, type(exc).__name__))
        return 3
    # What this file replaces, all read by the CLI's hook from the module when it runs.
    missing = [
        name
        for name in ("show_drive_auth_url", "ColabRuntime", "run_automation")
        if not callable(getattr(automation, name, None))
    ]
    if missing:
        say("%s: no automation.%s" % (UNSUPPORTED, ", automation.".join(missing)))
        return 3
    automation.show_drive_auth_url = show_consent
    automation.open = no_terminal
    automation.ColabRuntime = guarded(automation.ColabRuntime)
    sys.stdin = Answers()
    try:
        from colab_cli.cli import main as colab  # noqa: PLC0415
    except Exception as exc:  # noqa: BLE001
        say("%s: colab_cli.cli (%s)" % (UNSUPPORTED, type(exc).__name__))
        return 3
    # What the `colab` entry script runs, with the name it is run under.
    sys.argv[0] = "colab"
    return colab()


if __name__ == "__main__":
    sys.exit(main())
