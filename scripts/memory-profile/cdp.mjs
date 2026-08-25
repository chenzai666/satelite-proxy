#!/usr/bin/env node
// CDP helper for WebView2 remote debugging (port 9222).
// Modes:
//   node cdp.mjs targets
//   node cdp.mjs snap <label> [--nogc]     one sample (json line, GC by default)
//   node cdp.mjs soak <label> <seconds> <intervalSec>   alternating raw/gc samples
//   node cdp.mjs eval "<js expr>"
//   node cdp.mjs buttons "<selector>"
//   node cdp.mjs clickN "<selector>" <index>
//   node cdp.mjs clickSel "<css>"
//   node cdp.mjs gc
import process from "node:process";

const PORT = Number(process.env.CDP_PORT || 9222);
const base = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function listPages() {
  for (let attempt = 0; attempt < 90; attempt++) {
    try {
      const r = await fetch(`${base}/json/list`);
      if (r.ok) {
        const list = await r.json();
        const pages = list.filter((t) => t.type === "page");
        if (pages.length) return pages;
      }
    } catch {}
    await sleep(1000);
  }
  throw new Error(`no CDP page target on port ${PORT}`);
}

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    ws.addEventListener("message", (ev) => {
      let msg;
      try { msg = JSON.parse(ev.data); } catch { return; }
      if (msg.id && this.pending.has(msg.id)) {
        const p = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        clearTimeout(p.timer);
        p.resolve(msg);
      }
    });
  }
  static async connect(url) {
    const ws = new WebSocket(url);
    const cdp = new Cdp(ws);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    });
    return cdp;
  }
  send(method, params = {}, timeoutMs = 20000) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP timeout: ${method}`));
      }, timeoutMs);
      this.pending.set(id, { resolve: (m) => { clearTimeout(timer); resolve(m); }, timer });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  close() { try { this.ws.close(); } catch {} }
}

const pageExpr = `(() => {
  const pm = performance.memory || {};
  return {
    url: location.href,
    heapUsed: pm.usedJSHeapSize ?? null,
    heapTotal: pm.totalJSHeapSize ?? null,
    domNodes: document.getElementsByTagName('*').length,
    visibility: document.visibilityState,
    nav: (document.querySelector('.topnav-item.active') || document.querySelector('.simple-tab.active, [class*=tab].active'))?.textContent?.trim() || null,
    canvasW: document.querySelector('canvas')?.width || 0,
    vw: innerWidth, vh: innerHeight,
    canvases: document.getElementsByTagName('canvas').length,
    iframes: document.getElementsByTagName('iframe').length,
    imgs: document.getElementsByTagName('img').length,
    ts: Date.now(),
  };
})()`;

async function metrics(cdp) {
  const m = await cdp.send("Performance.getMetrics").catch(() => null);
  const out = {};
  if (m && m.result && m.result.metrics) {
    for (const x of m.result.metrics) out["m_" + x.name] = x.value;
  }
  return out;
}

async function sample(cdp, { gc }) {
  if (gc) await cdp.send("HeapProfiler.collectGarbage", {}, 30000).catch(() => {});
  await sleep(300);
  const ev = await cdp.send("Runtime.evaluate", { expression: pageExpr, returnByValue: true });
  const val = (ev.result && ev.result.result && ev.result.result.value) || {};
  const perf = await metrics(cdp);
  return { ...val, ...perf, gc: !!gc };
}

async function pickPage() {
  const pages = await listPages();
  return pages.find((p) => /1420|tauri:\/\//.test(p.url)) || pages[0];
}

async function main() {
  const mode = process.argv[2];
  if (mode === "targets") {
    const pages = await listPages();
    console.log(JSON.stringify(pages.map((p) => ({ id: p.id, url: p.url, ws: !!p.webSocketDebuggerUrl }))));
    return;
  }
  const page = await pickPage();
  const cdp = await Cdp.connect(page.webSocketDebuggerUrl);
  try {
    await cdp.send("Performance.enable").catch(() => {});
    if (mode === "snap") {
      const label = process.argv[3] || "";
      const s = await sample(cdp, { gc: !process.argv.includes("--nogc") });
      console.log(`SNAP ${label} ` + JSON.stringify(s));
    } else if (mode === "soak") {
      const label = process.argv[3] || "soak";
      const total = Number(process.argv[4] || 300);
      const interval = Number(process.argv[5] || 60);
      const t0 = Date.now();
      while (Date.now() - t0 < total * 1000) {
        const raw = await sample(cdp, { gc: false });
        console.log(`SOAK ${label} raw ` + JSON.stringify(raw));
        const gced = await sample(cdp, { gc: true });
        console.log(`SOAK ${label} gc  ` + JSON.stringify(gced));
        await sleep(interval * 1000);
      }
    } else if (mode === "eval") {
      const expr = process.argv[3];
      const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
      const res = r.result || {};
      if (res.exceptionDetails) console.log("EXC " + JSON.stringify(res.exceptionDetails).slice(0, 500));
      const v = res.result && res.result.value;
      console.log(typeof v === "object" ? JSON.stringify(v) : String(v));
    } else if (mode === "buttons") {
      const sel = process.argv[3] || "button";
      const expr = `JSON.stringify(Array.from(document.querySelectorAll(${JSON.stringify(sel)})).map((b, i) => ({ i, t: (b.textContent || '').trim().slice(0, 24), c: String(b.className).slice(0, 44), vis: !!(b.offsetWidth || b.offsetHeight) })))`;
      const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true });
      console.log(r.result?.result?.value);
    } else if (mode === "clickN" || mode === "clickSel") {
      const sel = process.argv[3];
      let expr;
      if (mode === "clickN") {
        const n = Number(process.argv[4] || 0);
        expr = `(() => { const els = Array.from(document.querySelectorAll(${JSON.stringify(sel)})); const e = els[${n}]; if (!e) return 'MISSING count=' + els.length; e.click(); return 'CLICKED of ' + els.length; })()`;
      } else {
        expr = `(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return 'MISSING'; e.click(); return 'CLICKED'; })()`;
      }
      const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true });
      console.log(r.result?.result?.value);
    } else if (mode === "realclick") {
      // Trusted-input click via Input.dispatchMouseEvent at element center.
      const sel = process.argv[3];
      const nth = Number(process.argv[4] || 0);
      const rectExpr = `(() => { const els = Array.from(document.querySelectorAll(${JSON.stringify(sel)})); const e = els[${nth}]; if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, h: r.height }; })()`;
      const rr = await cdp.send("Runtime.evaluate", { expression: rectExpr, returnByValue: true });
      const rect = rr.result?.result?.value;
      if (!rect) { console.log("MISSING"); return; }
      const x = Math.round(rect.x), y = Math.round(rect.y);
      await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
      await sleep(80);
      await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
      await sleep(60);
      await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
      console.log(`REALCLICK ${sel}[${nth}] at ${x},${y}`);
    } else if (mode === "rclick") {
      // React-props click: call onClick directly (input pipeline may be occluded in WebView2).
      const sel = process.argv[3];
      const nth = Number(process.argv[4] || 0);
      const expr = `(() => {
        const els = Array.from(document.querySelectorAll(${JSON.stringify(sel)}));
        const e = els[${nth}];
        if (!e) return 'MISSING count=' + els.length;
        const rk = Object.keys(e).find(k => k.startsWith('__reactProps'));
        if (!rk) return 'NO_PROPS ' + e.className;
        const p = e[rk];
        const handler = p.onClick || p.onPointerDown || p.onMouseDown;
        if (typeof handler !== 'function') return 'NO_HANDLER keys=' + Object.keys(p).join(',');
        handler({ currentTarget: e, target: e, preventDefault(){}, stopPropagation(){}, clientX: 0, clientY: 0, button: 0, pointerType: 'mouse', isPrimary: true });
        return 'REACTCLICK ' + e.className;
      })()`;
      const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true });
      console.log(r.result?.result?.value);
    } else if (mode === "shot") {
      const out = process.argv[3];
      await cdp.send("Page.enable").catch(() => {});
      const r = await cdp.send("Page.captureScreenshot", { format: "png" });
      const { writeFileSync } = await import("node:fs");
      writeFileSync(out, Buffer.from(r.result.data, "base64"));
      console.log("SHOT saved " + out);
    } else if (mode === "gc") {
      await cdp.send("HeapProfiler.collectGarbage", {}, 30000);
      console.log("GC done");
    } else {
      console.log("unknown mode: " + mode);
    }
  } finally {
    cdp.close();
  }
}
main().catch((e) => { console.error("ERR", e.message); process.exit(1); });
