"""Stand-in for google-colab-cli 0.8.0's `colab_cli.commands.automation`, for the drive wrapper.

Only `drivemount` is here, built like the real one: its hook reads `show_drive_auth_url`, `open`
and `sys.stdin` from this module when it runs, which is what the wrapper replaces. A simulated
kernel stands in for `drive.mount`: it asks for the consent and mounts once it gets it, or ends
with the error it was answered. Whether the user allowed access is the server knob
`drive_consent`; what the kernel was answered is kept with the runtime in the fake state.
"""

import sys

import fake_common as fc

CONSENT_URL = "https://accounts.google.com/o/oauth2/auth?client_id=fake-drive&scope=drive"

# The request the kernel sends, and the answer it waits for.
REQUEST_HEADER = {"msg_id": "request-1", "msg_type": "colab_request"}
REQUEST_ID = 7


def show_drive_auth_url(uri):
    sys.stdout.write("\n[colab] REQUIRED: Google Drive Authorization needed.\nPlease visit:\n\n%s\n\n" % uri)
    sys.stdout.flush()
    return False


class _Session:
    def msg(self, msg_type, content):
        return {"header": {"msg_type": msg_type}, "content": content}


class _StdinChannel:
    def __init__(self):
        self.sent = []

    def send(self, message):
        self.sent.append(message)


class _KernelClient:
    def __init__(self):
        self.session = _Session()
        self.stdin_channel = _StdinChannel()


class ColabRuntime:
    """The runtime's kernel; `drive` is the runtime's record of its Drive in the fake state."""

    def __init__(self, url, token, session_name=None, history=None, drive=None):
        self.session_name = session_name
        self.drive = drive
        self.colab_request_hook = None

    def execute_code(self, code, allow_stdin=False, timeout=None):
        """`drive.mount`, which asks for the consent even when Drive is mounted already."""
        client = _KernelClient()
        request = {"msg_type": "colab_request", "header": REQUEST_HEADER,
                   "metadata": {"colab_msg_id": REQUEST_ID},
                   "content": {"request": {"authType": "dfs_ephemeral"}}}
        if self.colab_request_hook:
            try:
                self.colab_request_hook(request, client)
            except Exception:  # noqa: BLE001
                # The real client logs a failing hook and goes on.
                pass
        answers = [
            m["content"]["value"]
            for m in client.stdin_channel.sent
            if m.get("parent_header") == REQUEST_HEADER
            and m["content"]["value"].get("colab_msg_id") == REQUEST_ID
        ]
        answer = answers[0] if answers else None
        self.drive.setdefault("answers", []).append(
            None if answer is None else ("error" if "error" in answer else "ok"))
        if answer is not None and "error" in answer:
            return [{"output_type": "error", "ename": "MessageError", "evalue": answer["error"],
                     "traceback": ["MessageError: %s" % answer["error"]]}]
        if self.drive.get("mounted"):
            return [{"text": "Drive already mounted at /content/drive; to attempt to forcibly remount, "
                             "call drive.mount(\"/content/drive\", force_remount=True).\n"}]
        if answer is None:
            # Unanswered, the real kernel waits two minutes and mounts without the consent.
            return [{"output_type": "error", "ename": "ValueError", "evalue": "mount failed",
                     "traceback": ["ValueError: mount failed"]}]
        self.drive["mounted"] = True
        return [{"text": "Mounted at /content/drive\n"}]


def run_automation(name, op, code, allow_stdin=False, path=None, timeout=None, drive=None):
    runtime = ColabRuntime("https://proxy.invalid", "proxy-token", session_name=name, drive=drive)

    def drivefs_hook(deserialize_msg, wsclient):
        content = deserialize_msg.get("content", {})
        if content.get("request", {}).get("authType") != "dfs_ephemeral":
            return False
        msg_id = deserialize_msg.get("metadata", {}).get("colab_msg_id")
        if not fc.knob("drive_consent"):
            show_drive_auth_url(CONSENT_URL)
            sys.stdout.write("Press Enter after you have granted access... ")
            sys.stdout.flush()
            try:
                with open("/dev/tty") as tty:
                    tty.readline()
            except (FileNotFoundError, OSError, IOError):
                try:
                    sys.stdin.readline()
                except Exception:  # noqa: BLE001
                    try:
                        input()
                    except Exception:  # noqa: BLE001
                        pass
        sys.stdout.write("[colab] Authorizing VM...\n")
        if fc.knob("drive_consent"):
            sys.stdout.write("[colab] Credentials propagated. Resuming mount...\n")
            reply = wsclient.session.msg(
                "input_reply", {"value": {"type": "colab_reply", "colab_msg_id": msg_id}})
            if "header" in deserialize_msg:
                reply["parent_header"] = deserialize_msg["header"]
            wsclient.stdin_channel.send(reply)
        else:
            sys.stdout.write('[colab] Error propagating: 403 {"success": false}\n')
        return True

    runtime.colab_request_hook = drivefs_hook
    for out in runtime.execute_code(code, allow_stdin=allow_stdin, timeout=timeout):
        if "text" in out:
            sys.stdout.write(out["text"])
        else:
            sys.stderr.write("".join(out["traceback"]) + "\n")
    sys.stdout.flush()


def drivemount(session, drive):
    """`colab drivemount -s SESSION`, with the runtime's Drive record from the fake state."""
    code = "from google.colab import drive\ndrive.mount('/content/drive')"
    sys.stdout.write("[colab] Mounting Google Drive to '/content/drive' on %s...\n" % session)
    run_automation(session, "drivemount", code, allow_stdin=True, path="/content/drive",
                   timeout=600, drive=drive)
