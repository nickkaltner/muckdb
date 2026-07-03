"""Simulate the macOS symptom: clipboard.write() that never settles. The
deadline must fire, the PNG must download instead, and the button must
return to a clickable state.

Requirements: a running daemon (`muckdb start`) with the demo session loaded
(`./demo.sh`), and chromium on PATH. Run with:

    uv run --with websockets python scripts/e2e/copy_image_hang_test.py
"""
import asyncio, json, subprocess, sys, tempfile, time, urllib.request, os
import websockets
PORT=int(os.environ.get("E2E_CDP_PORT","9343")); BASE=os.environ.get("MUCKDB_URL","http://localhost:11000")
async def main():
    profile = tempfile.mkdtemp(prefix="muckdb-hang-")
    dldir = tempfile.mkdtemp(prefix="muckdb-hangdl-")
    chrome = subprocess.Popen(["chromium","--headless=new","--disable-gpu",f"--remote-debugging-port={PORT}",f"--user-data-dir={profile}","--window-size=1300,800","about:blank"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        ws_url=None
        for _ in range(50):
            try:
                pages=json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json"))
                ws_url=next(p for p in pages if p["type"]=="page")["webSocketDebuggerUrl"]; break
            except Exception: time.sleep(0.2)
        async with websockets.connect(ws_url,max_size=64*1024*1024) as ws:
            mid=[0]
            async def cmd(method,**params):
                mid[0]+=1
                await ws.send(json.dumps({"id":mid[0],"method":method,"params":params}))
                while True:
                    m=json.loads(await ws.recv())
                    if m.get("id")==mid[0]: return m.get("result",{})
            async def js(expr):
                r=await cmd("Runtime.evaluate",expression=expr,returnByValue=True,awaitPromise=True)
                return r["result"].get("value")
            await cmd("Page.enable")
            await cmd("Browser.setDownloadBehavior",behavior="allow",downloadPath=dldir)
            await cmd("Page.navigate",url=f"{BASE}/session/demo/")
            await asyncio.sleep(2.5)
            # Make every clipboard.write hang forever — the macOS symptom.
            await js("navigator.clipboard.write = () => new Promise(() => {})")
            await js("document.querySelector('[data-copyimg]').click()")
            checks=[]
            def check(n,ok,d=""): checks.append(ok); print(("PASS" if ok else "FAIL"),n,d)
            # Wait for the render + both deadlines (6s + 4s) with margin.
            state=None
            for _ in range(70):
                state=await js("(() => { const b = document.querySelector('[data-copyimg]'); if (b.classList.contains('flash-ok')) return 'ok'; if (b.classList.contains('flash-err')) return 'err'; return b.classList.contains('busy') ? 'busy' : 'idle'; })()")
                if state in ("ok","err"): break
                await asyncio.sleep(0.5)
            check("button recovers (not stuck busy)", state in ("ok","err"), str(state))
            toastTxt=await js("(document.getElementById('toast')||{}).textContent||''")
            check("toast explains the timeout", "timed out" in toastTxt and "downloaded" in toastTxt, toastTxt[:90])
            got=[f for f in os.listdir(dldir) if f.endswith(".png")]
            check("PNG downloaded as fallback", bool(got), str(got))
            clickable=await js("!document.querySelector('[data-copyimg]').classList.contains('busy')")
            check("button clickable again", bool(clickable))
            print(f"{sum(checks)}/{len(checks)} passed")
            sys.exit(0 if all(checks) else 1)
    finally: chrome.terminate()
asyncio.run(main())
