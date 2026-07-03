"""Drive headless Chromium through muckdb's navigation flows via CDP and
assert the Back button behaves: one history entry per navigation.

Requirements: a running daemon (`muckdb start`) with the demo session loaded
(`./demo.sh`), and chromium on PATH. Run with:

    uv run --with websockets python scripts/e2e/nav_test.py
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

PORT = int(os.environ.get("E2E_CDP_PORT", "9333"))
BASE = os.environ.get("MUCKDB_URL", "http://localhost:11000")


async def main():
    profile = tempfile.mkdtemp(prefix="muckdb-navtest-")
    chrome = subprocess.Popen(
        [
            "chromium",
            "--headless=new",
            "--disable-gpu",
            f"--remote-debugging-port={PORT}",
            f"--user-data-dir={profile}",
            "about:blank",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        ws_url = None
        for _ in range(50):
            try:
                pages = json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json"))
                page = next(p for p in pages if p["type"] == "page")
                ws_url = page["webSocketDebuggerUrl"]
                break
            except Exception:
                time.sleep(0.2)
        assert ws_url, "no CDP page target"

        async with websockets.connect(ws_url, max_size=20 * 1024 * 1024) as ws:
            mid = [0]

            async def cmd(method, **params):
                mid[0] += 1
                await ws.send(json.dumps({"id": mid[0], "method": method, "params": params}))
                while True:
                    msg = json.loads(await ws.recv())
                    if msg.get("id") == mid[0]:
                        if "error" in msg:
                            raise RuntimeError(f"{method}: {msg['error']}")
                        return msg.get("result", {})

            async def js(expr):
                r = await cmd(
                    "Runtime.evaluate",
                    expression=expr,
                    returnByValue=True,
                    awaitPromise=True,
                )
                if r.get("exceptionDetails"):
                    raise RuntimeError(r["exceptionDetails"])
                return r["result"].get("value")

            async def settle(ms=700):
                await asyncio.sleep(ms / 1000)

            await cmd("Page.enable")
            await cmd("Page.navigate", url=f"{BASE}/session/demo/")
            await settle(2000)

            results = []

            def check(name, ok, detail=""):
                results.append((name, ok, detail))
                print(f"{'PASS' if ok else 'FAIL'}  {name}  {detail}")

            # -- Flow A: session -> explore is ONE entry; Back returns to demo --
            h0 = await js("history.length")
            await js("document.querySelector('[data-explore]').click()")
            await settle(1500)
            path = await js("location.pathname")
            h1 = await js("history.length")
            check("explore lands on /db/", str(path).startswith("/db/"), path)
            check("explore adds exactly one entry", h1 == h0 + 1, f"{h0} -> {h1}")
            await js("history.back()")
            await settle(1500)
            back_path = await js("location.pathname")
            back_sess = await js("document.querySelector('#session-combo .cv-name') && document.querySelector('#session-combo .cv-name').textContent")
            check("Back returns to /session/demo/", back_path == "/session/demo/", back_path)
            check("Back restores the demo session", "demo" in str(back_sess), str(back_sess))

            # -- Flow B: ledger -> db chip; Back returns to /ledger --
            await js("document.querySelector('.tab[data-tab=\"ledger\"]').click()")
            await settle()
            await js("document.querySelector('[data-open-db]').click()")
            await settle(1500)
            path = await js("location.pathname")
            check("ledger db chip lands on /db/", str(path).startswith("/db/"), path)
            await js("history.back()")
            await settle(1000)
            path = await js("location.pathname")
            check("Back returns to /ledger", path == "/ledger", path)

            # -- Flow C: ledger § session link switches tab + loads session --
            has_sess_link = await js("!!document.querySelector('[data-session]')")
            if has_sess_link:
                sid = await js("document.querySelector('[data-session]').dataset.session")
                await js("document.querySelector('[data-session]').click()")
                await settle(1500)
                tab = await js("document.querySelector('.tab.active').dataset.tab")
                path = await js("location.pathname")
                check("§ link opens sessions tab", tab == "sessions", str(tab))
                check("§ link URL is /session/<id>/", str(path).startswith("/session/"), path)
                await js("history.back()")
                await settle(1000)
                path = await js("location.pathname")
                check("Back from § link returns to /ledger", path == "/ledger", path)
            else:
                print("SKIP  no [data-session] link in ledger")

            failed = [r for r in results if not r[1]]
            print(f"\n{len(results) - len(failed)}/{len(results)} passed")
            sys.exit(1 if failed else 0)
    finally:
        chrome.terminate()


asyncio.run(main())
