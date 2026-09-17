"""Restore shadow-managed worktree files from the published objects.

Kept in sync with tools/gen-index.py SHADOW_URL_BASE: each committed
.shadow/refs/<path>.ref records the content-addressed object (sha256) and
size of the published file <path>; fetch it and verify the length.
"""
import concurrent.futures
import pathlib
import tomllib
import urllib.request

BASE = "https://azurice-shadow.tos-cn-beijing.volces.com/ranim-one-shot/objects"

refs_dir = pathlib.Path(".shadow/refs")
jobs = []
for ref in refs_dir.rglob("*.ref"):
    data = tomllib.loads(ref.read_text())
    kind, _, hexd = str(data["oid"]).partition(":")
    if kind != "sha256" or len(hexd) != 64:
        raise SystemExit(f"unparsable oid in {ref}")
    # .shadow/refs/<run>/output/<file>.ref -> worktree file <run>/output/<file>
    dest = ref.relative_to(refs_dir).with_suffix("")
    url = f"{BASE}/{kind}/{hexd[:2]}/{hexd[2:]}"
    jobs.append((dest, url, int(data.get("size", 0))))


def fetch(job):
    dest, url, size = job
    dest.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url) as resp:
        payload = resp.read()
    if size and len(payload) != size:
        raise SystemExit(f"{dest}: expected {size} bytes, got {len(payload)}")
    dest.write_bytes(payload)
    return dest


with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    for dest in pool.map(fetch, jobs):
        print(f"restored {dest}")
