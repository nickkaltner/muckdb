"""Exercise the export button (real download) and the import file-input change
handler (real File object) in headless Chromium, round-tripping the demo
session. The imported session is removed afterwards.

Requirements: a running daemon (`muckdb start`) with the demo session loaded
(`./demo.sh`), chromium on PATH, and the muckdb binary (target/debug or PATH).
Run with:

    uv run --with websockets python scripts/e2e/export_import_test.py
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

PORT = int(os.environ.get("E2E_CDP_PORT", "9335"))
BASE = os.environ.get("MUCKDB_URL", "http://localhost:11000")


def muckdb_bin():
    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    debug = os.path.join(repo, "target", "debug", "muckdb")
    return debug if os.path.exists(debug) else "muckdb"


async def main():
    profile = tempfile.mkdtemp(prefix="muckdb-uitest-")
    dldir = tempfile.mkdtemp(prefix="muckdb-uidl-")
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
                ws_url = next(p for p in pages if p["type"] == "page")["webSocketDebuggerUrl"]
                break
            except Exception:
                time.sleep(0.2)
        assert ws_url, "no CDP page"

        async with websockets.connect(ws_url, max_size=64 * 1024 * 1024) as ws:
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
                r = await cmd("Runtime.evaluate", expression=expr,
                              returnByValue=True, awaitPromise=True)
                if r.get("exceptionDetails"):
                    raise RuntimeError(r["exceptionDetails"])
                return r["result"].get("value")

            results = []

            def check(name, ok, detail=""):
                results.append((name, ok, detail))
                print(f"{'PASS' if ok else 'FAIL'}  {name}  {detail}")

            await cmd("Page.enable")
            await cmd("Browser.setDownloadBehavior", behavior="allow", downloadPath=dldir)
            await cmd("Page.navigate", url=f"{BASE}/session/demo/")
            await asyncio.sleep(2.5)

            # Export button visible + click downloads demo.muckdb.
            vis = await js("(() => { const b = document.getElementById('export-btn'); if (!b) return 'missing'; const r = b.getBoundingClientRect(); return r.width > 0 ? 'visible' : 'hidden'; })()")
            check("export button visible", vis == "visible", str(vis))
            await js("document.getElementById('export-btn').click()")
            got = None
            for _ in range(40):
                files = [f for f in os.listdir(dldir) if f.endswith(".muckdb")]
                if files:
                    got = files[0]
                    break
                await asyncio.sleep(0.5)
            check("export downloads demo.muckdb", got == "demo.muckdb", str(got))

            # Import: feed the exported bytes through the REAL file input +
            # change handler, then confirm the toast and the URL.
            imported = await js("""
              (async () => {
                const r = await fetch('/api/session/export?id=demo');
                const blob = await r.blob();
                const dt = new DataTransfer();
                dt.items.add(new File([blob], 'demo.muckdb'));
                const input = document.getElementById('import-file');
                input.files = dt.files;
                input.dispatchEvent(new Event('change'));
                for (let i = 0; i < 60; i++) {
                  await new Promise(res => setTimeout(res, 500));
                  const t = document.getElementById('toast');
                  if (t && /imported session|import failed/.test(t.textContent)) return t.textContent;
                }
                return 'timeout';
              })()
            """)
            check("import toast", str(imported).startswith("imported session "), str(imported))
            sid = str(imported).replace("imported session ", "").strip()
            await asyncio.sleep(1.5)
            path = await js("location.pathname")
            check("URL is imported session", path == f"/session/{sid}/", f"{path} vs {sid}")
            panels = await js("document.querySelectorAll('#panels .panel').length")
            check("imported dashboard has panels", isinstance(panels, int) and panels > 5, str(panels))

            # Charts against the imported db actually load (canvas present).
            charts = await js("document.querySelectorAll('#panels canvas').length")
            check("imported charts render", isinstance(charts, int) and charts > 0, str(charts))

            failed = [r for r in results if not r[1]]
            print(f"\n{len(results) - len(failed)}/{len(results)} passed")
            if sid and sid != "timeout":
                subprocess.run([muckdb_bin(), "session", "rm", sid], check=False)
                print(f"cleaned up imported session {sid}")
            sys.exit(1 if failed else 0)
    finally:
        chrome.terminate()


asyncio.run(main())
