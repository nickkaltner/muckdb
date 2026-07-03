"""Verify the contents-sidebar resizer: drag changes width, persists across
reload, double-click resets.

Requirements: a running daemon (`muckdb start`) with the demo session loaded
(`./demo.sh`), and chromium on PATH. Run with:

    uv run --with websockets python scripts/e2e/toc_resize_test.py
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

PORT = int(os.environ.get("E2E_CDP_PORT", "9336"))
BASE = os.environ.get("MUCKDB_URL", "http://localhost:11000")


async def main():
    profile = tempfile.mkdtemp(prefix="muckdb-rztest-")
    chrome = subprocess.Popen(
        ["chromium", "--headless=new", "--disable-gpu",
         f"--remote-debugging-port={PORT}", f"--user-data-dir={profile}",
         "--window-size=1600,900", "about:blank"],
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

        async with websockets.connect(ws_url, max_size=32 * 1024 * 1024) as ws:
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
                r = await cmd("Runtime.evaluate", expression=expr, returnByValue=True, awaitPromise=True)
                if r.get("exceptionDetails"):
                    raise RuntimeError(r["exceptionDetails"])
                return r["result"].get("value")

            results = []

            def check(name, ok, detail=""):
                results.append((name, ok, detail))
                print(f"{'PASS' if ok else 'FAIL'}  {name}  {detail}")

            await cmd("Page.enable")
            await cmd("Page.navigate", url=f"{BASE}/session/demo/")
            await asyncio.sleep(2.5)

            # Open the TOC if not already (1600px window shows the toggle).
            await js("document.getElementById('view-sessions').classList.contains('toc-open') || document.getElementById('toc-toggle').click()")
            await asyncio.sleep(0.3)
            w0 = await js("document.getElementById('session-toc').getBoundingClientRect().width")
            check("toc visible with default width", isinstance(w0, (int, float)) and w0 > 100, str(w0))

            # Simulate a drag: pointerdown on the resizer, move 150px left, up.
            neww = await js("""
              (() => {
                const rz = document.getElementById('toc-resizer');
                const r = rz.getBoundingClientRect();
                const x = r.left + 3, y = r.top + 100;
                rz.dispatchEvent(new PointerEvent('pointerdown', {pointerId: 1, clientX: x, clientY: y, bubbles: true}));
                rz.dispatchEvent(new PointerEvent('pointermove', {pointerId: 1, clientX: x - 150, clientY: y, bubbles: true}));
                rz.dispatchEvent(new PointerEvent('pointerup', {pointerId: 1, clientX: x - 150, clientY: y, bubbles: true}));
                return document.getElementById('session-toc').getBoundingClientRect().width;
              })()
            """)
            check("drag widens the sidebar ~150px", abs(neww - (w0 + 150)) < 12, f"{w0} -> {neww}")
            saved = await js("localStorage.getItem('muckdb.tocw')")
            check("width persisted", saved is not None and abs(int(saved) - int(neww)) < 12, str(saved))

            # Reload → width restored.
            await cmd("Page.navigate", url=f"{BASE}/session/demo/")
            await asyncio.sleep(2)
            await js("document.getElementById('view-sessions').classList.contains('toc-open') || document.getElementById('toc-toggle').click()")
            await asyncio.sleep(0.3)
            w2 = await js("document.getElementById('session-toc').getBoundingClientRect().width")
            check("width survives reload", abs(w2 - neww) < 12, f"{neww} vs {w2}")

            # Double-click resets.
            w3 = await js("""
              (() => {
                const rz = document.getElementById('toc-resizer');
                rz.dispatchEvent(new MouseEvent('dblclick', {bubbles: true}));
                return document.getElementById('session-toc').getBoundingClientRect().width;
              })()
            """)
            check("double-click resets to default", abs(w3 - w0) < 12, f"{w3} vs default {w0}")

            failed = [r for r in results if not r[1]]
            print(f"\n{len(results) - len(failed)}/{len(results)} passed")
            sys.exit(1 if failed else 0)
    finally:
        chrome.terminate()


asyncio.run(main())
