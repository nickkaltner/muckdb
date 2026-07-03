"""Verify "remove this db": a dead database's error offers a remove button
that drops it from the databases list (a ledger tombstone; using the db again
resurfaces it). Self-contained — creates its own db, deletes the file, then
exercises the flow.

Requirements: a running daemon (`muckdb start`), chromium on PATH, and the
muckdb binary (target/debug or PATH). Run with:

    uv run --with websockets python scripts/e2e/forget_db_test.py
"""

import asyncio
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request

import websockets

PORT = int(os.environ.get("E2E_CDP_PORT", "9341"))
BASE = os.environ.get("MUCKDB_URL", "http://localhost:11000")


def muckdb_bin():
    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    debug = os.path.join(repo, "target", "debug", "muckdb")
    return debug if os.path.exists(debug) else "muckdb"


def make_dead_db():
    """Register a db in the ledger, then delete the file; returns (id, path)."""
    d = tempfile.mkdtemp(prefix="muckdb-forgettest-")
    path = os.path.join(d, "forgetme.duckdb")
    subprocess.run([muckdb_bin(), path, "-c", "CREATE TABLE t(x INT);"],
                   check=True, stdout=subprocess.DEVNULL)
    os.remove(path)
    dbs = json.loads(subprocess.run([muckdb_bin(), "ls", "databases"],
                                    check=True, capture_output=True).stdout)
    return next(db["id"] for db in dbs if db["path"] == path), path


async def main():
    dead_id, dead_path = make_dead_db()
    profile = tempfile.mkdtemp(prefix="muckdb-forget-")
    chrome = subprocess.Popen(
        ["chromium", "--headless=new", "--disable-gpu",
         f"--remote-debugging-port={PORT}", f"--user-data-dir={profile}",
         "--window-size=1300,800", "about:blank"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        ws_url = None
        for _ in range(50):
            try:
                pages = json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json"))
                ws_url = next(p for p in pages if p["type"] == "page")["webSocketDebuggerUrl"]
                break
            except Exception:
                time.sleep(0.2)
        assert ws_url

        async with websockets.connect(ws_url, max_size=64 * 1024 * 1024) as ws:
            mid = [0]

            async def cmd(method, **params):
                mid[0] += 1
                await ws.send(json.dumps({"id": mid[0], "method": method, "params": params}))
                while True:
                    m = json.loads(await ws.recv())
                    if m.get("id") == mid[0]:
                        return m.get("result", {})

            async def js(expr):
                r = await cmd("Runtime.evaluate", expression=expr, returnByValue=True, awaitPromise=True)
                return r["result"].get("value")

            await cmd("Page.enable")
            await cmd("Page.navigate", url=f"{BASE}/db/{dead_id}/")
            await asyncio.sleep(2.5)
            checks = []

            def check(n, ok, d=""):
                checks.append(ok)
                print(("PASS" if ok else "FAIL"), n, d)

            err = await js("(document.querySelector('#results .note.err')||{}).textContent||''")
            check("error names the path", "does not exist:" in err, err[:80])
            check("remove button shown", await js("!!document.querySelector('[data-forget-db]')"))
            await js("document.querySelector('[data-forget-db]').click()")
            await asyncio.sleep(1.5)
            toast = await js("(document.getElementById('toast')||{}).textContent||''")
            check("toast confirms", toast.startswith("removed "), toast)
            # The removed db's own error must be gone (the app may show the
            # reset note, another db's error + button, or a working table).
            err2 = await js("(document.querySelector('#results .note.err')||{}).textContent||''")
            check("removed db's error cleared", dead_path not in err2, err2[:60])
            dbs = json.loads(subprocess.run([muckdb_bin(), "ls", "databases"],
                                            check=True, capture_output=True).stdout)
            check("db gone from the list", all(d["id"] != dead_id for d in dbs))
            print(f"{sum(checks)}/{len(checks)} passed")
            sys.exit(0 if all(checks) else 1)
    finally:
        chrome.terminate()


asyncio.run(main())
