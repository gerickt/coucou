// Renders Mochi's frame sequences with the ORIGINAL engine in headless Chrome.
//
//   node render.mjs <out-dir> [--only <name-substring>]
//   node render.mjs --compare <work-dir> <frames-dir> <out.png>
//
// Serves the vendored TypeScript (types stripped by node:module, nothing else
// changed) to a headless Chrome over a loopback HTTP server, drives BotEngine
// through page/driver.js via the DevTools protocol, and writes raw PNG frames
// plus raw-manifest.json into <out-dir>. optimize.py turns them into
// mochi/frames/.

import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { stripTypeScriptTypes } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { SEQUENCES } from "./sequences.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CHROME = process.env.CHROME || "/usr/bin/google-chrome-stable";

// ── Loopback server ─────────────────────────────────────────────────────────

function tsModule(path) {
  return stripTypeScriptTypes(readFileSync(path, "utf8"));
}

function route(url, extraRoot) {
  const p = decodeURIComponent(new URL(url, "http://x").pathname);
  if (p.startsWith("/page/")) {
    const f = join(HERE, p);
    const type = f.endsWith(".js") ? "text/javascript" : "text/html";
    return [type, readFileSync(f)];
  }
  if (p === "/core/sound") return ["text/javascript", tsModule(join(HERE, "shims/sound.ts"))];
  const m = /^\/(mochi|core)\/([a-z-]+)$/.exec(p);
  if (m) return ["text/javascript", tsModule(join(HERE, "vendor", m[1], `${m[2]}.ts`))];
  if (extraRoot && p.startsWith("/extra/")) {
    const f = join(extraRoot, p.slice("/extra/".length));
    if (!f.startsWith(extraRoot)) return null;
    return [f.endsWith(".png") ? "image/png" : "application/octet-stream", readFileSync(f)];
  }
  return null;
}

function serve(extraRoot) {
  return new Promise((ok) => {
    const server = createServer((req, res) => {
      let hit = null;
      try { hit = route(req.url, extraRoot); } catch { hit = null; }
      if (!hit) { res.writeHead(404); res.end(); return; }
      res.writeHead(200, { "content-type": hit[0], "cache-control": "no-store" });
      res.end(hit[1]);
    });
    server.listen(0, "127.0.0.1", () => ok(server));
  });
}

// ── Chrome + DevTools protocol ──────────────────────────────────────────────

async function launchChrome() {
  const profile = mkdtempSync(join(tmpdir(), "render-mochi-"));
  const proc = spawn(CHROME, [
    "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
    "--hide-scrollbars", "--force-device-scale-factor=1", "--font-render-hinting=none",
    "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  const wsUrl = await new Promise((ok, fail) => {
    let buf = "";
    const timer = setTimeout(() => fail(new Error("chrome did not start")), 20000);
    proc.stderr.on("data", (d) => {
      buf += d;
      const m = /DevTools listening on (ws:\/\/\S+)/.exec(buf);
      if (m) { clearTimeout(timer); ok(m[1]); }
    });
    proc.on("exit", (c) => fail(new Error(`chrome exited ${c}`)));
  });
  const ws = new WebSocket(wsUrl);
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
  let id = 0;
  const pending = new Map();
  const listeners = [];
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { ok, fail } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) fail(new Error(JSON.stringify(msg.error)));
      else ok(msg.result);
    } else if (msg.method) {
      for (const l of listeners) l(msg);
    }
  };
  const send = (method, params = {}, sessionId) =>
    new Promise((ok, fail) => {
      const mid = ++id;
      pending.set(mid, { ok, fail });
      ws.send(JSON.stringify({ id: mid, method, params, sessionId }));
    });
  const close = () => {
    try { ws.close(); } catch { /* closing */ }
    proc.kill("SIGTERM");
    setTimeout(() => rmSync(profile, { recursive: true, force: true }), 500);
  };
  return { send, listeners, close };
}

async function openPage(cdp, url, width = 800, height = 600) {
  const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
  const s = (m, p) => cdp.send(m, p, sessionId);
  await s("Page.enable");
  await s("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
  await s("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });
  await s("Page.navigate", { url });
  const evaluate = async (expression) => {
    const r = await s("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 800));
    return r.result.value;
  };
  for (let i = 0; i < 200; i++) {
    if (await evaluate("window.__ready === true").catch(() => false)) return { s, evaluate };
    await new Promise((ok) => setTimeout(ok, 50));
  }
  throw new Error("engine page did not load");
}

// ── Commands ────────────────────────────────────────────────────────────────

async function render(outDir, only) {
  mkdirSync(outDir, { recursive: true });
  const server = await serve(null);
  const port = server.address().port;
  const cdp = await launchChrome();
  try {
    const page = await openPage(cdp, `http://127.0.0.1:${port}/page/index.html`);
    const list = SEQUENCES.filter((q) => !only || q.name.includes(only));
    const raw = [];
    for (const q of list) {
      const frames = await page.evaluate(`window.renderSequence(${JSON.stringify(q)})`);
      const dir = join(outDir, q.mode, q.name);
      mkdirSync(dir, { recursive: true });
      const files = frames.map((b64, i) => {
        const f = join(q.mode, q.name, `${String(i).padStart(3, "0")}.png`);
        writeFileSync(join(outDir, f), Buffer.from(b64, "base64"));
        return f;
      });
      raw.push({ ...q, files });
    }
    writeFileSync(join(outDir, "raw-manifest.json"), JSON.stringify(raw, null, 1));
    console.log(`rendered ${list.length} sequences, ${raw.reduce((n, q) => n + q.files.length, 0)} frames`);
  } finally {
    cdp.close();
    server.close();
  }
}

// Side by side: the shipped PNG (left) next to a fresh canvas the original
// engine draws at the same virtual instant (right), one row per sequence, plus
// the largest per-channel difference measured in the page.
async function compare(workDir, framesDir, outPng) {
  const root = resolve(framesDir);
  const manifest = JSON.parse(readFileSync(join(workDir, "index.json"), "utf8"));
  const server = await serve(root);
  const port = server.address().port;
  const cdp = await launchChrome();
  try {
    const picks = manifest.sequences.filter((q) => q.mode === "desktop" && (q.outfit === "none" || q.name.endsWith(".idle")));
    const W = 2 * 260 + 220;
    const rowH = 250;
    const page = await openPage(cdp, `http://127.0.0.1:${port}/page/index.html`, W, rowH * picks.length);
    const result = await page.evaluate(`(async () => {
      const picks = ${JSON.stringify(picks)};
      const SEQ = ${JSON.stringify(Object.fromEntries(SEQUENCES.map((q) => [q.mode + ":" + q.name, q])))};
      document.body.style.cssText = "margin:0;background:#2b2d31;font:12px monospace;color:#ddd";
      let worst = 0;
      const rows = [];
      for (const q of picks) {
        const spec = SEQ[q.mode + ":" + q.name];
        const mid = Math.floor(spec.frames / 2);
        const fresh = window.renderSequence({ ...spec, frames: mid + 1 })[mid];
        const row = document.createElement("div");
        row.style.cssText = "display:flex;align-items:center;gap:20px;height:${rowH}px";
        const a = new Image(); a.src = "/extra/" + q.frames[mid];
        const b = new Image(); b.src = "data:image/png;base64," + fresh;
        await Promise.all([a.decode(), b.decode()]);
        const diff = (() => {
          const c = document.createElement("canvas"); c.width = a.width; c.height = a.height;
          const x = c.getContext("2d");
          // Composited over the page grey, as it is seen on a desktop.
          const grab = (img) => {
            x.fillStyle = "#2b2d31"; x.fillRect(0, 0, c.width, c.height);
            x.drawImage(img, 0, 0);
            return x.getImageData(0, 0, c.width, c.height).data;
          };
          const da = grab(a), db = grab(b);
          let m = 0, sum = 0, over = 0;
          for (let i = 0; i < da.length; i += 4) {
            const d = Math.max(Math.abs(da[i] - db[i]), Math.abs(da[i + 1] - db[i + 1]), Math.abs(da[i + 2] - db[i + 2]));
            m = Math.max(m, d); sum += d; if (d > 16) over++;
          }
          return { max: m, mean: +(sum / (da.length / 4)).toFixed(3), over16: over };
        })();
        worst = Math.max(worst, diff.max);
        const label = document.createElement("div");
        label.style.cssText = "width:200px;padding-left:8px";
        label.textContent = q.name + " #" + mid + " max " + diff.max + " mean " + diff.mean;
        row.append(label, a, b);
        document.body.append(row);
        rows.push({ name: q.name, frame: mid, ...diff });
      }
      return { worst, rows };
    })()`);
    const shot = await page.s("Page.captureScreenshot", { format: "png", captureBeyondViewport: true });
    writeFileSync(outPng, Buffer.from(shot.data, "base64"));
    console.log(JSON.stringify(result));
  } finally {
    cdp.close();
    server.close();
  }
}

const args = process.argv.slice(2);
if (args[0] === "--compare") {
  await compare(args[1], args[2], args[3]);
} else {
  const outDir = args[0];
  if (!outDir) { console.error("usage: node render.mjs <out-dir> [--only name]"); process.exit(2); }
  const oi = args.indexOf("--only");
  await render(outDir, oi > 0 ? args[oi + 1] : null);
}
process.exit(0);
