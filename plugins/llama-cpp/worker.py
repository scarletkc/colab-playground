#!/usr/bin/env python3
"""llama.cpp deployment worker for Colab Playground (runs on the Colab VM).

Started detached by the host. Steps: detect the GPU -> pick the GGUF
quantisation that fits (``quant = auto``) -> download llama.cpp's prebuilt CUDA
build and the model in parallel -> start ``llama-server`` (KV f16 -> q8_0 and
MTP on -> off fallbacks) -> verify it really uses the GPU -> warm up -> report
``ready`` -> keep monitoring the server.

Protocol (see docs/PLUGINS.md):
  * inputs arrive as ``PLUGIN_INPUT_<ID>``; a saved Hugging Face token as
    ``PLUGIN_HF_TOKEN`` (optional); with Google Drive on, the plugin's Drive
    folder as ``PLUGIN_DRIVE_DIR``, where the models are kept and read from;
  * progress is written atomically to ``$PLUGIN_STATUS_FILE``;
  * activity touches ``$PLUGIN_HEARTBEAT`` (read by the VM watchdog).

Hard-won lessons kept from the original deploy script (docs/COLAB_NOTES.md):
prebuilt CUDA binaries instead of a 10+ minute source build;
``LD_LIBRARY_PATH=<bindir>:/usr/lib64-nvidia`` or CUDA silently falls back to
CPU; never port 8080; detached ``Popen(start_new_session=True)``; a warm-up
request before reporting ready; a VRAM-used check against silent CPU fallback.

Also usable locally for the shared fitting test vectors:
    python3 worker.py --fit-self-test crates/core/tests/data/quant_fit_vectors.json
"""

import json
import os
import re
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

E = os.environ.get
RUN_DIR = E("PLUGIN_RUN_DIR", "/root/.colab-playground/runs/manual")
STATUS_FILE = E("PLUGIN_STATUS_FILE", os.path.join(RUN_DIR, "status.json"))
HEARTBEAT = E("PLUGIN_HEARTBEAT", "")
BIN_ROOT = "/content/llama-bin"
SERVER_LOG = os.path.join(RUN_DIR, "server.log")
FALLBACK_TAG = "b12430"  # verified build, used when no newer one can be fetched
NVIDIA_LIB = "/usr/lib64-nvidia"  # Colab keeps libcuda.so.1 here, off the default path

REPO = E("PLUGIN_INPUT_REPO", "").strip()
QUANT = E("PLUGIN_INPUT_QUANT", "auto").strip() or "auto"
CTX = int(float(E("PLUGIN_INPUT_CTX", "0") or 0))
PORT = int(float(E("PLUGIN_INPUT_PORT", "8000") or 8000))
ALIAS = E("PLUGIN_INPUT_ALIAS", "").strip()
KV = (E("PLUGIN_INPUT_KV", "auto") or "auto").strip()
MMPROJ = (E("PLUGIN_INPUT_MMPROJ", "auto") or "auto").strip()
MTP = (E("PLUGIN_INPUT_MTP", "auto") or "auto").strip()
TAG = (E("PLUGIN_INPUT_LLAMA_TAG", "latest") or "latest").strip()
API_KEY = E("PLUGIN_INPUT_API_KEY", "")
# With Google Drive turned on for the run, the model files are kept in this plugin's Drive
# folder, one folder per repository, and llama-server reads them from there: a later run of the
# same model downloads nothing. Without it they go to the runtime's disk, wiped when it stops.
DRIVE_DIR = E("PLUGIN_DRIVE_DIR", "")
MODELS = os.path.join(DRIVE_DIR, "models", *REPO.split("/")) if DRIVE_DIR and REPO else "/content/models"
EXTRA = E("PLUGIN_INPUT_EXTRA_ARGS", "")
HF_TOKEN = E("PLUGIN_HF_TOKEN", "")
SECRETS = [s for s in (API_KEY, HF_TOKEN) if s]

# ------------------------------------------------------------- fitting
# Mirrors crates/core/src/quant.rs; both are checked against
# crates/core/tests/data/quant_fit_vectors.json.
RESERVE_MIB = 1024.0
COMPUTE_MIB = 512.0
COMPUTE_FRACTION = 0.05
BUDGET_FRACTION = 0.97
FALLBACK_KV_BYTES_PER_TOKEN = 128 * 1024
FALLBACK_CTX = 32768
MIN_AUTO_CTX = 8192
KV_FACTOR = {"f16": 1.0, "q8_0": 34.0 / 64.0, "q4_0": 18.0 / 64.0}
FULL_PRECISION = ("F16", "BF16", "F32")
LOW_BIT_PREFIXES = ("Q3", "Q2", "Q1", "IQ3", "IQ2", "IQ1", "TQ")


def is_quant_token(token):
    u = token.upper()
    if not u or not all(c.isascii() and (c.isalnum() or c == "_") for c in u):
        return False
    if u in ("F16", "F32", "BF16") or u.startswith("MXFP4"):
        return True
    for prefix in ("IQ", "TQ", "Q"):
        if u.startswith(prefix):
            rest = u[len(prefix):]
            return bool(rest) and rest[0].isdigit()
    return False


def strip_shard_suffix(stem):
    tail = stem[-15:]
    if len(stem) >= 15 and tail[0] == "-" and tail[1:6].isdigit() and tail[6:10] == "-of-" and tail[10:15].isdigit():
        return stem[:-15]
    return stem


def quant_label(path):
    base = path.rsplit("/", 1)[-1]
    if not base.lower().endswith(".gguf"):
        return None
    tokens = re.split(r"[-.]", strip_shard_suffix(base[:-5]))
    for i in range(len(tokens) - 1, -1, -1):
        if is_quant_token(tokens[i]):
            label = tokens[i].upper()
            if i > 0 and tokens[i - 1].lower() == "ud":
                return "UD-" + label
            return label
    return None


def is_mmproj(path):
    return "mmproj" in path.rsplit("/", 1)[-1].lower()


def group(files):
    """files: [(path, size)] -> (variants largest first, mmproj files)."""
    by_label, mmproj = {}, []
    for path, size in files:
        if not path.lower().endswith(".gguf"):
            continue
        if is_mmproj(path):
            mmproj.append((path, size))
            continue
        label = quant_label(path)
        if not label:
            continue
        v = by_label.setdefault(label, {"quant": label, "files": [], "size": 0})
        v["files"].append(path)
        v["size"] += size
    for v in by_label.values():
        v["files"].sort()
    variants = sorted(by_label.values(), key=lambda v: (-v["size"], v["quant"]))
    return variants, mmproj


def is_low_bit(v):
    q = v["quant"][3:] if v["quant"].startswith("UD-") else v["quant"]
    return q.startswith(LOW_BIT_PREFIXES)


def estimate_mib(weights, mmproj, kv_bytes_per_token, ctx, kv):
    mib = 1024.0 * 1024.0
    w = weights / mib
    return w + mmproj / mib + kv_bytes_per_token * KV_FACTOR[kv] * ctx / mib + RESERVE_MIB + COMPUTE_MIB + COMPUTE_FRACTION * w


def ctx_ladder(inp):
    if inp["ctx"] > 0:
        return [inp["ctx"]], inp["ctx"]
    mx = inp.get("model_ctx") or FALLBACK_CTX
    ladder, c = [mx], mx
    while c // 2 >= MIN_AUTO_CTX:
        c //= 2
        ladder.append(c)
    return ladder, mx


def try_fit(v, inp, ctx, max_ctx):
    kvb = inp.get("kv_bytes_per_token") or FALLBACK_KV_BYTES_PER_TOKEN
    budget = inp["vram_mib"] * BUDGET_FRACTION
    for kv in ([inp["kv"]] if inp.get("kv") else ["f16", "q8_0"]):
        est = estimate_mib(v["size"], inp.get("mmproj_bytes", 0), kvb, ctx, kv)
        if est <= budget:
            return {"quant": v["quant"], "kv": kv, "ctx": ctx, "estimate_mib": int(est + 0.999),
                    "budget_mib": int(budget), "ctx_reduced": ctx < max_ctx,
                    "kv_reduced": not inp.get("kv") and kv != "f16", "files": v["files"]}
    return None


def fit_auto(variants, inp):
    quantised = [v for v in variants if v["quant"] not in FULL_PRECISION]
    pool = quantised or list(variants)
    normal = [v for v in pool if not is_low_bit(v)]
    ladder, max_ctx = ctx_ladder(inp)
    for tier in (normal, pool):
        for ctx in ladder:
            for v in tier:
                choice = try_fit(v, inp, ctx, max_ctx)
                if choice:
                    return choice, None
    return None, []


def fit_fixed(variants, quant, inp):
    v = next((x for x in variants if x["quant"].lower() == quant.lower()), None)
    if v is None:
        return None, None
    ladder, max_ctx = ctx_ladder(inp)
    for ctx in ladder:
        choice = try_fit(v, inp, ctx, max_ctx)
        if choice:
            return choice, None
    fitting = [w["quant"] for w in variants if any(try_fit(w, inp, c, max_ctx) for c in ladder)]
    return None, fitting


def fit_self_test(path):
    with open(path, encoding="utf-8") as fh:
        vectors = json.load(fh)
    failures = 0
    for vec in vectors:
        variants, _ = group([(p, s) for p, s in vec["files"]])
        inp = dict(vec["input"])
        if vec["quant"] == "auto":
            choice, fitting = fit_auto(variants, inp)
        else:
            choice, fitting = fit_fixed(variants, vec["quant"], inp)
        exp = vec.get("expect")
        if exp:
            got = choice and (choice["quant"], choice["kv"], choice["ctx"])
            ok = got == (exp["quant"], exp["kv"], exp["ctx"])
        else:
            got = fitting
            ok = choice is None and fitting == vec.get("expect_error_fitting")
        print("%s %s -> %s" % ("ok  " if ok else "FAIL", vec["name"], got))
        failures += 0 if ok else 1
    return 1 if failures else 0


# ------------------------------------------------------------- status

_current_stage = ["detect"]


def redact(text):
    for s in SECRETS:
        if len(s) >= 4:
            text = text.replace(s, "***")
    return text


def status(state, stage=None, message="", progress=None, outputs=None):
    if stage:
        _current_stage[0] = stage
    doc = {"state": state, "stage": _current_stage[0], "message": redact(message)}
    if progress is not None:
        doc["progress"] = max(0.0, min(1.0, float(progress)))
    if outputs is not None:
        doc["outputs"] = outputs
    print("[%s] %s/%s: %s" % (time.strftime("%H:%M:%S"), state, doc["stage"], doc["message"]), flush=True)
    tmp = STATUS_FILE + ".tmp"
    with open(tmp, "w") as fh:
        json.dump(doc, fh)
    os.replace(tmp, STATUS_FILE)


def fail(message):
    status("failed", message=message)
    sys.exit(1)


def touch_heartbeat():
    if not HEARTBEAT:
        return
    try:
        os.makedirs(os.path.dirname(HEARTBEAT), exist_ok=True)
        with open(HEARTBEAT, "a"):
            pass
        os.utime(HEARTBEAT, None)
    except OSError:
        pass


def heartbeat_while(event, every=30):
    def loop():
        while not event.wait(every):
            touch_heartbeat()
    touch_heartbeat()
    threading.Thread(target=loop, daemon=True).start()


# ------------------------------------------------------------- helpers


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def http_json(url, data=None, timeout=30, headers=None):
    h = {"Content-Type": "application/json", **(headers or {})}
    body = json.dumps(data).encode() if data is not None else None
    req = urllib.request.Request(url, data=body, headers=h)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def hf_headers():
    return {"Authorization": "Bearer " + HF_TOKEN} if HF_TOKEN else {}


def api_headers():
    return {"Authorization": "Bearer " + API_KEY} if API_KEY else {}


def gpu_info():
    try:
        out = subprocess.run(["nvidia-smi", "--query-gpu=name,memory.total", "--format=csv,noheader,nounits"],
                             capture_output=True, text=True, check=True).stdout.strip().splitlines()[0]
        name, mem = [x.strip() for x in out.split(",")[:2]]
        return name, int(float(mem))
    except Exception:  # noqa: BLE001
        return None, 0


def gpu_used_mib():
    try:
        out = subprocess.run(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"],
                             capture_output=True, text=True, check=True).stdout.strip().splitlines()[0]
        return int(float(out.strip()))
    except Exception:  # noqa: BLE001
        return 0


# ------------------------------------------------------------- GGUF header

GGUF_SCALARS = {0: "<B", 1: "<b", 2: "<H", 3: "<h", 4: "<I", 5: "<i", 6: "<f", 7: "<?", 10: "<Q", 11: "<q", 12: "<d"}


class _Truncated(Exception):
    pass


def parse_gguf_metadata(data):
    """Parses as many metadata key/values as `data` holds (partial is fine)."""
    pos = [0]

    def take(n):
        if pos[0] + n > len(data):
            raise _Truncated()
        b = data[pos[0]:pos[0] + n]
        pos[0] += n
        return b

    def unpack(fmt):
        return struct.unpack(fmt, take(struct.calcsize(fmt)))[0]

    def string():
        return take(unpack("<Q")).decode("utf-8", "replace")

    def value(t):
        if t in GGUF_SCALARS:
            return unpack(GGUF_SCALARS[t])
        if t == 8:
            return string()
        if t == 9:
            et, n = unpack("<I"), unpack("<Q")
            if et == 8:
                for _ in range(n):
                    string()
                return None
            if et in GGUF_SCALARS and n <= 4096:
                return [value(et) for _ in range(n)]
            take(n * struct.calcsize(GGUF_SCALARS.get(et, "<B")))
            return None
        raise _Truncated()

    meta = {}
    try:
        if take(4) != b"GGUF":
            return meta
        if unpack("<I") < 2:
            return meta
        unpack("<Q")
        count = unpack("<Q")
        for _ in range(count):
            key = string()
            meta[key] = value(unpack("<I"))
    except _Truncated:
        pass
    return meta


def kv_params(meta):
    """(kv bytes per token at f16, model context, has MTP layers)."""
    arch = meta.get("general.architecture")
    has_mtp = any("nextn_predict_layers" in k or "mtp_num_hidden_layers" in k for k in meta)
    if not isinstance(arch, str):
        return None, None, has_mtp

    def g(name):
        return meta.get("%s.%s" % (arch, name))

    ctx = g("context_length")
    blocks, heads, kv_heads, embd = g("block_count"), g("attention.head_count"), g("attention.head_count_kv"), g("embedding_length")
    if isinstance(heads, list):
        heads = max(heads) if heads else None
    key_len = g("attention.key_length") or (embd // heads if embd and heads else None)
    val_len = g("attention.value_length") or key_len
    if not blocks or not key_len:
        return None, ctx, has_mtp
    if isinstance(kv_heads, list):
        per_layer = sum(int(h) for h in kv_heads) * (key_len + val_len)
        return per_layer * 2, ctx, has_mtp
    kv_heads = kv_heads or heads
    if not kv_heads:
        return None, ctx, has_mtp
    interval = g("full_attention_interval")
    layers = blocks // interval if isinstance(interval, int) and interval > 0 else blocks
    return layers * kv_heads * (key_len + val_len) * 2, ctx, has_mtp


def read_gguf_header(repo, path, nbytes=16 * 1024 * 1024):
    url = "https://huggingface.co/%s/resolve/main/%s" % (repo, urllib.parse.quote(path))
    req = urllib.request.Request(url, headers={"Range": "bytes=0-%d" % (nbytes - 1), **hf_headers()})
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return parse_gguf_metadata(r.read(nbytes))
    except Exception as exc:  # noqa: BLE001
        print("GGUF header unavailable (%s); using conservative estimates" % type(exc).__name__, flush=True)
        return {}


# ------------------------------------------------------------- llama.cpp


def candidate_tags():
    """Newest few bNNNN build tags, then the verified fallback.

    Not releases/latest (it points at a v1.0.0 without assets) and not the GitHub
    API (Colab's shared egress IPs get rate-limited); `git ls-remote` has
    neither problem. The newest tag may still be uploading, so try several.
    """
    if TAG != "latest":
        return [TAG]
    tags = []
    try:
        out = subprocess.run(["git", "ls-remote", "--tags", "--refs", "https://github.com/ggml-org/llama.cpp", "b*"],
                             capture_output=True, text=True, timeout=60).stdout
        nums = sorted({int(m.group(1)) for m in re.finditer(r"refs/tags/b(\d+)$", out, re.M)}, reverse=True)
        tags = ["b%d" % n for n in nums[:5]]
    except Exception:  # noqa: BLE001
        pass
    return tags + ([FALLBACK_TAG] if FALLBACK_TAG not in tags else [])


def fetch_llama(tag):
    base = "https://github.com/ggml-org/llama.cpp/releases/download/" + tag
    names = ("llama-%s-bin-ubuntu-cuda-12.8-x64.tar.gz" % tag, "cudart-llama-%s-bin-ubuntu-cuda-12.8-x64.tar.gz" % tag)
    for name in names:
        if subprocess.run(["curl", "-fsSL", "--retry", "3", "-o", os.path.join(BIN_ROOT, name), base + "/" + name]).returncode:
            return False
    for name in names:
        run(["tar", "xzf", os.path.join(BIN_ROOT, name), "-C", BIN_ROOT])
        os.remove(os.path.join(BIN_ROOT, name))
    return True


def install_llama(result):
    """Prebuilt CUDA build (a source build wastes 10+ GPU minutes)."""
    try:
        os.makedirs(BIN_ROOT, exist_ok=True)
        tag = None
        for cand in candidate_tags():
            if os.path.exists("%s/llama-%s/llama-server" % (BIN_ROOT, cand)) or fetch_llama(cand):
                tag = cand
                break
            print("llama.cpp %s prebuilt download failed, trying the next build" % cand, flush=True)
        if not tag:
            raise RuntimeError("no prebuilt llama.cpp build could be downloaded")
        bindir = "%s/llama-%s" % (BIN_ROOT, tag)
        if not os.path.exists(bindir + "/llama-server"):
            found = [r for r, _, fs in os.walk(BIN_ROOT) if "llama-server" in fs and tag in r]
            if not found:
                raise RuntimeError("llama-server not found after extracting %s" % tag)
            bindir = found[0]
        # The cudart archive (libcudart/libcublas) unpacks separately; put it next to llama-server.
        for r, _, fs in os.walk(BIN_ROOT):
            if "cudart" in os.path.basename(r) and tag in r:
                for f in fs:
                    if ".so" in f and not os.path.exists(os.path.join(bindir, f)):
                        run(["cp", "-a", os.path.join(r, f), os.path.join(bindir, f)])
        result["tag"], result["bindir"] = tag, bindir
    except Exception as exc:  # noqa: BLE001
        result["error"] = "installing llama.cpp failed: %s" % exc


def server_env(bindir):
    paths = [bindir, NVIDIA_LIB, os.environ.get("LD_LIBRARY_PATH", "")]
    env = {k: v for k, v in os.environ.items() if not k.startswith("PLUGIN_")}
    env["LD_LIBRARY_PATH"] = ":".join(p for p in paths if p)
    return env


# ------------------------------------------------------------- model


def list_repo_files():
    try:
        tree = http_json("https://huggingface.co/api/models/%s/tree/main?recursive=true" % REPO, headers=hf_headers())
    except urllib.error.HTTPError as exc:
        hint = " (private or gated: save a Hugging Face token in Settings)" if exc.code in (401, 403) else ""
        fail("cannot read Hugging Face repo %s: HTTP %d%s" % (REPO, exc.code, hint))
    except Exception as exc:  # noqa: BLE001
        fail("cannot read Hugging Face repo %s (%s)" % (REPO, type(exc).__name__))
    files = []
    for f in tree:
        if f.get("type") == "file" and f.get("path", "").lower().endswith(".gguf"):
            size = (f.get("lfs") or {}).get("size") or f.get("size") or 0
            files.append((f["path"], int(size)))
    return files


def pick_mmproj(mmproj):
    if MMPROJ == "none" or not mmproj:
        return None
    cands = [p for p, _ in mmproj]
    if MMPROJ != "auto":
        cands = [p for p in cands if MMPROJ.lower() in p.lower()]
        if not cands:
            fail("no vision projector matches %r" % MMPROJ)
    # f16 is the most compatible (a T4 has no native bf16), then bf16.
    cands.sort(key=lambda p: (0 if re.search(r"[-_]f16", p.lower()) else 1 if "bf16" in p.lower() else 2, p))
    return cands[0]


def dir_size(path):
    total = 0
    for r, _, fs in os.walk(path):
        for f in fs:
            try:
                total += os.path.getsize(os.path.join(r, f))
            except OSError:
                pass
    return total


def have_file(name, size):
    """The file is already in MODELS with the size the repository lists (an earlier run's
    download, kept on Drive). A file of another size is downloaded again."""
    try:
        return size > 0 and os.path.getsize(os.path.join(MODELS, name)) == size
    except OSError:
        return False


def check_drive_space(need):
    os.makedirs(MODELS, exist_ok=True)
    try:
        free = shutil.disk_usage(MODELS).free
    except OSError:
        return
    if free < need:
        fail("Google Drive has %.1f GB free; this model needs %.1f GB. Free some space or turn "
             "Google Drive off for this deployment." % (free / 1e9, need / 1e9))


def download(files, total):
    run([sys.executable, "-m", "pip", "install", "-q", "-U", "huggingface_hub[hf_xet]"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.makedirs(MODELS, exist_ok=True)
    start = dir_size(MODELS)
    done = threading.Event()

    def progress():
        while not done.wait(10):
            got = max(0, dir_size(MODELS) - start)
            status("running", "download", "%.1f / %.1f GB" % (got / 1e9, total / 1e9), progress=got / total if total else None)
    threading.Thread(target=progress, daemon=True).start()
    env = {k: v for k, v in os.environ.items() if not k.startswith("PLUGIN_")}
    if HF_TOKEN:
        env["HF_TOKEN"] = HF_TOKEN
    try:
        with open(os.path.join(RUN_DIR, "download.log"), "w") as log:
            run(["hf", "download", REPO, *files, "--local-dir", MODELS], stdout=log, stderr=subprocess.STDOUT, env=env)
    except subprocess.CalledProcessError:
        fail("model download failed, see download.log in the run directory")
    finally:
        done.set()


# ------------------------------------------------------------- server

_server = [None]


def port_busy(port):
    with socket.socket() as s:
        s.settimeout(1)
        return s.connect_ex(("127.0.0.1", port)) == 0


def kill_server():
    subprocess.run(["pkill", "-x", "llama-server"], check=False)
    for _ in range(30):
        if subprocess.run(["pgrep", "-x", "llama-server"], capture_output=True).returncode != 0:
            return
        time.sleep(1)


def start_server(bindir, model, mm, alias, ctx, kv, mtp):
    cmd = [bindir + "/llama-server", "-m", model, "-c", str(ctx), "-np", "1", "-ngl", "999", "-fa", "on", "--jinja",
           "--alias", alias, "--cache-type-k", kv, "--cache-type-v", kv, "--host", "127.0.0.1", "--port", str(PORT)]
    if mm:
        cmd += ["--mmproj", mm]
    if mtp:
        cmd += ["--spec-type", "draft-mtp"]
    if API_KEY:
        cmd += ["--api-key", API_KEY]
    cmd += shlex.split(EXTRA)
    with open(os.path.join(RUN_DIR, "server.cmd"), "w") as fh:
        fh.write(redact(shlex.join(cmd)) + "\n")
    # start_new_session: detached from the worker and the kernel.
    p = subprocess.Popen(cmd, env=server_env(bindir), stdout=open(SERVER_LOG, "w"), stderr=subprocess.STDOUT,
                         stdin=subprocess.DEVNULL, start_new_session=True)
    _server[0] = p
    return p


def wait_healthy(p, timeout=1200):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if p.poll() is not None:
            return False
        try:
            if http_json("http://127.0.0.1:%d/health" % PORT, timeout=5).get("status") == "ok":
                return True
        except Exception:  # noqa: BLE001
            pass
        touch_heartbeat()
        time.sleep(3)
    p.kill()
    return False


def log_tail(n=6):
    try:
        with open(SERVER_LOG, errors="replace") as fh:
            return redact(" | ".join(line.strip() for line in fh.readlines()[-n:]))
    except OSError:
        return ""


def chat(max_tokens, content):
    return http_json("http://127.0.0.1:%d/v1/chat/completions" % PORT,
                     {"messages": [{"role": "user", "content": content}], "max_tokens": max_tokens},
                     timeout=900, headers=api_headers())


def default_alias():
    name = REPO.rsplit("/", 1)[-1]
    name = re.sub(r"[-_]gguf$", "", name, flags=re.I)
    if "_" in name:  # bartowski style "Org_Model"
        name = name.split("_", 1)[1]
    return name.lower()


def on_sigterm(*_):
    p = _server[0]
    if p is not None and p.poll() is None:
        try:
            os.killpg(p.pid, signal.SIGTERM)
        except OSError:
            pass
    try:
        status("failed", message="stopped")
    except OSError:
        pass
    os._exit(143)


def monitor():
    """After `ready`: touch the heartbeat while requests are being served and
    report a crashed server."""
    while True:
        time.sleep(30)
        p = _server[0]
        if p is not None and p.poll() is not None:
            fail("llama-server exited: %s" % log_tail())
        try:
            slots = http_json("http://127.0.0.1:%d/slots" % PORT, timeout=10, headers=api_headers())
            if isinstance(slots, list) and any(s.get("is_processing") for s in slots if isinstance(s, dict)):
                touch_heartbeat()
        except Exception:  # noqa: BLE001
            pass


# ------------------------------------------------------------- main


def plan(variants, mmproj_bytes, vram, gguf, kept=()):
    """kept: the variants whose files are already in MODELS (on Drive). With quant = auto the
    best of them that fits is used, so a deployment run again downloads nothing even when it got
    another GPU; only when none of them fits is the best variant overall chosen."""
    kvb, model_ctx, _ = kv_params(gguf)
    inp = {"vram_mib": vram, "ctx": CTX, "model_ctx": model_ctx, "kv_bytes_per_token": kvb,
           "mmproj_bytes": mmproj_bytes, "kv": None if KV == "auto" else KV}
    if QUANT.lower() == "auto":
        choice = fit_auto(list(kept), inp)[0] if kept else None
        if choice:
            choice["kept"] = True
        else:
            choice, _ = fit_auto(variants, inp)
        if not choice:
            fail("no quantisation of %s fits in %d MiB of GPU memory" % (REPO, vram))
    else:
        choice, fitting = fit_fixed(variants, QUANT, inp)
        if choice is None and fitting is None:
            fail("%s has no %s files; available: %s" % (REPO, QUANT, ", ".join(v["quant"] for v in variants) or "none"))
        if choice is None:
            fail("%s does not fit in %d MiB; quantisations that would fit: %s" % (QUANT, vram, ", ".join(fitting) or "none"))
    reasons = []
    if choice.get("kept"):
        reasons.append("already on Google Drive")
    reasons.append("estimated %d of %d MiB" % (choice["estimate_mib"], choice["budget_mib"]))
    if choice["kv_reduced"]:
        reasons.append("KV cache quantised to fit")
    if choice["ctx_reduced"]:
        reasons.append("context reduced to %d to fit" % choice["ctx"])
    if kvb is None:
        reasons.append("model header unavailable, conservative KV estimate")
    choice["reason"] = "; ".join(reasons)
    return choice


def cpu_plan(variants, kept=()):
    normal = [v for v in variants if v["quant"] not in FULL_PRECISION and not is_low_bit(v)] or variants
    if QUANT.lower() == "auto":
        v = next((x for x in normal if x["quant"] == "Q4_K_M"), normal[-1])
        v = next((x for x in normal if x in kept), v)
    else:
        v = next((x for x in variants if x["quant"].lower() == QUANT.lower()), None)
        if v is None:
            fail("%s has no %s files" % (REPO, QUANT))
    return {"quant": v["quant"], "kv": "f16" if KV == "auto" else KV, "ctx": CTX or 8192, "files": v["files"],
            "reason": "no GPU: CPU run", "estimate_mib": 0, "budget_mib": 0}


def main():
    os.makedirs(RUN_DIR, exist_ok=True)
    signal.signal(signal.SIGTERM, on_sigterm)
    busy = threading.Event()
    heartbeat_while(busy)
    if not REPO:
        fail("missing model repository")
    if PORT == 8080:
        fail("VM port 8080 belongs to Colab; choose another port")

    status("running", "detect", "checking the GPU")
    gpu, vram = gpu_info()
    status("running", "detect", "%s, %d MiB" % (gpu, vram) if gpu else "no GPU (CPU run)")

    llama = {}
    installer = threading.Thread(target=install_llama, args=(llama,), daemon=True)
    installer.start()

    status("running", "plan", "listing GGUF files")
    repo_files = list_repo_files()
    sizes = dict(repo_files)
    variants, mmproj = group(repo_files)
    if not variants:
        fail("%s has no GGUF model files" % REPO)
    mm_file = pick_mmproj(mmproj)
    mm_size = dict(mmproj).get(mm_file, 0) if mm_file else 0
    smallest = variants[-1]["files"][0]
    gguf = read_gguf_header(REPO, smallest)
    kept = [v for v in variants if all(have_file(f, sizes.get(f, 0)) for f in v["files"])] if DRIVE_DIR else []
    choice = plan(variants, mm_size, vram, gguf, kept) if gpu else cpu_plan(variants, kept)
    status("running", "plan", "%s, KV %s, context %d (%s)" % (choice["quant"], choice["kv"], choice["ctx"], choice["reason"]))

    files = choice["files"] + ([mm_file] if mm_file else [])
    missing = [f for f in files if not have_file(f, sizes.get(f, 0))]
    total = sum(sizes.get(f, 0) for f in missing)
    where = "on Google Drive" if DRIVE_DIR else "on the runtime"
    if not missing:
        status("running", "download", "already %s, nothing to download" % where, progress=1.0)
    else:
        if DRIVE_DIR:
            check_drive_space(total)
        kept = len(files) - len(missing)
        status("running", "download", "downloading %.1f GB%s" % (
            total / 1e9, " (%d file(s) already %s)" % (kept, where) if kept else ""), progress=0.0)
        download(missing, total)
    installer.join()
    if "error" in llama:
        fail(llama["error"])
    bindir = llama["bindir"]

    if gpu:  # without libcuda the prebuilt build silently falls back to CPU
        out = subprocess.run([bindir + "/llama-server", "--list-devices"], env=server_env(bindir),
                             capture_output=True, text=True).stdout
        if "CUDA0" not in out:
            fail("llama.cpp does not see the GPU (check %s/libcuda.so.1): %s" % (NVIDIA_LIB, out.strip()[-200:]))

    model = os.path.join(MODELS, choice["files"][0])
    mm = os.path.join(MODELS, mm_file) if mm_file else None
    alias = ALIAS or default_alias()
    kv_opts = [choice["kv"]] + (["q8_0"] if KV == "auto" and choice["kv"] == "f16" else [])
    has_mtp = kv_params(gguf)[2]
    mtp_opts = ([True, False] if has_mtp else [False]) if MTP == "auto" else [MTP == "on"]

    kill_server()
    if port_busy(PORT):
        fail("port %d is already in use on the VM; choose another port" % PORT)

    ok, kv, mtp = False, kv_opts[0], False
    for mtp in mtp_opts:
        for kv in kv_opts:
            status("running", "server", "starting llama-server (KV %s, MTP %s)" % (kv, "on" if mtp else "off"))
            p = start_server(bindir, model, mm, alias, choice["ctx"], kv, mtp)
            if wait_healthy(p):
                ok = True
                break
            print("start failed: %s" % log_tail(), flush=True)
            kill_server()
        if ok:
            break
    if not ok:
        fail("llama-server did not start: %s" % log_tail())

    used = gpu_used_mib()
    if gpu and used < 1024:
        fail("the server is up but uses only %d MiB of GPU memory: it is running on the CPU" % used)

    status("running", "warmup", "warming up (the first request triggers CUDA JIT, about a minute)")
    try:
        chat(16, "hi")
        r = chat(256, "Introduce yourself in three sentences.")
        tps = r.get("timings", {}).get("predicted_per_second", 0)
    except Exception as exc:  # noqa: BLE001
        fail("warm-up request failed: %s" % type(exc).__name__)
    try:
        n_ctx = http_json("http://127.0.0.1:%d/props" % PORT, timeout=10,
                          headers=api_headers())["default_generation_settings"].get("n_ctx", choice["ctx"])
    except Exception:  # noqa: BLE001
        n_ctx = choice["ctx"]
    outputs = {"model_id": alias, "quant": choice["quant"], "file": choice["files"][0], "mmproj": mm_file,
               "n_ctx": n_ctx, "kv": kv, "mtp": mtp, "gpu": gpu, "vram_used_mib": gpu_used_mib(),
               "vram_total_mib": vram, "tok_per_s": round(tps, 1), "llama_cpp": llama["tag"],
               "fit_reason": choice["reason"], "api_path": "/v1"}
    busy.set()
    status("ready", "warmup", "%s ready: context %s, KV %s, about %s tok/s" % (alias, n_ctx, kv, outputs["tok_per_s"]),
           progress=1.0, outputs=outputs)
    monitor()


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--fit-self-test":
        sys.exit(fit_self_test(sys.argv[2]))
    try:
        main()
    except SystemExit:
        raise
    except Exception as exc:  # noqa: BLE001
        fail("worker error: %s" % type(exc).__name__)
