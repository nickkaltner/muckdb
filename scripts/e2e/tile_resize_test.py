"""Verify the tile height grip: drag a chart tile taller, persists across
reload, double-click resets; markdown tiles resize too.

Requirements: a running daemon (`muckdb start`) with the demo session loaded
(`./demo.sh`), and chromium on PATH. Run with:

    uv run --with websockets python scripts/e2e/tile_resize_test.py
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

PORT = int(os.environ.get("E2E_CDP_PORT", "9337"))
BASE = os.environ.get("MUCKDB_URL", "http://localhost:11000")

DRAG = """
  ((name, dy) => {
    const grip = document.querySelector(`[data-grip="${name}"]`);
    const r = grip.getBoundingClientRect();
    const x = r.left + 10, y = r.top + 10;
    grip.dispatchEvent(new PointerEvent('pointerdown', {pointerId: 1, clientX: x, clientY: y, bubbles: true}));
    grip.dispatchEvent(new PointerEvent('pointermove', {pointerId: 1, clientX: x, clientY: y + dy, bubbles: true}));
    grip.dispatchEvent(new PointerEvent('pointerup', {pointerId: 1, clientX: x, clientY: y + dy, bubbles: true}));
  })
"""


async def main():
    profile = tempfile.mkdtemp(prefix="muckdb-griptest-")
    chrome = subprocess.Popen(
        ["chromium", "--headless=new", "--disable-gpu",
         f"--remote-debugging-port={PORT}", f"--user-data-dir={profile}",
         "--window-size=1400,1000", "about:blank"],
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
            await asyncio.sleep(3)

            H = "document.querySelector('[data-tile=\"revenue\"] .panel-chart').getBoundingClientRect().height"
            h0 = await js(H)
            check("chart tile default height", abs(h0 - 300) < 5, str(h0))

            # Drag the 'revenue' chart 180px taller.
            await js(DRAG + "('revenue', 180)")
            await asyncio.sleep(0.5)
            h1 = await js(H)
            check("drag grows chart by ~180px", abs(h1 - (h0 + 180)) < 10, f"{h0} -> {h1}")
            canvas_h = await js("document.querySelector('[data-tile=\"revenue\"] canvas').getBoundingClientRect().height")
            check("chart canvas follows", abs(canvas_h - h1) < 30, str(canvas_h))

            # Markdown tile shrinks with overflow.
            MH = "document.querySelector('[data-tile=\"intro\"] .panel-body .md').getBoundingClientRect().height"
            m0 = await js(MH)
            await js(DRAG + "('intro', -Math.round(%f - 150))" % m0)
            await asyncio.sleep(0.3)
            m1 = await js(MH)
            check("markdown tile shrinks", m1 < m0 - 50 and m1 >= 80, f"{m0} -> {m1}")

            # Persists across reload.
            await cmd("Page.navigate", url=f"{BASE}/session/demo/")
            await asyncio.sleep(3)
            h2 = await js(H)
            check("chart height survives reload", abs(h2 - h1) < 10, f"{h1} vs {h2}")

            # Double-click resets.
            await js("document.querySelector('[data-grip=\"revenue\"]').dispatchEvent(new MouseEvent('dblclick', {bubbles: true}))")
            await asyncio.sleep(1)
            h3 = await js(H)
            check("double-click resets to default", abs(h3 - 300) < 5, str(h3))

            failed = [r for r in results if not r[1]]
            print(f"\n{len(results) - len(failed)}/{len(results)} passed")
            sys.exit(1 if failed else 0)
    finally:
        chrome.terminate()


asyncio.run(main())
