// Drives the original BotEngine (vendor/mochi/engine.ts, unmodified) on a
// virtual clock and returns PNG frames. Only the environment is replaced:
// performance.now, Math.random and setTimeout. The engine's own update() and
// draw() produce every pixel.
"use strict";

(() => {
  let clockMs = 1000;
  let rngState = 1;
  let timers = [];
  let timerSeq = 0;

  // mulberry32: small, fast and stable across Chrome versions.
  function rand() {
    rngState = (rngState + 0x6d2b79f5) | 0;
    let t = rngState;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }

  performance.now = () => clockMs;
  Math.random = rand;
  window.setTimeout = (fn, ms) => {
    const id = ++timerSeq;
    timers.push({ id, at: clockMs + Math.max(0, Number(ms) || 0), fn });
    return id;
  };
  window.clearTimeout = (id) => {
    timers = timers.filter((t) => t.id !== id);
  };

  function runTimers() {
    for (;;) {
      let next = null;
      for (const t of timers) if (t.at <= clockMs && (!next || t.at < next.at || (t.at === next.at && t.id < next.id))) next = t;
      if (!next) return;
      timers = timers.filter((t) => t !== next);
      next.fn();
    }
  }

  // Framing of the original windows (see render.mjs for the numbers' origin).
  const FRAMES = {
    // windows/src/desktop/main.ts: a 120 px window, Mochi drawn 96 px wide
    // with a 12 px side margin, particleOverhang 0.
    desktop: { side: 120, drawW: 96, dy: 0 },
    // The bar: the same drawing, framed tighter so the body fills the icon,
    // lowered a little so a hat still fits above it.
    bar: { side: 32, drawW: 27, dy: 3 },
  };

  function makeCanvas(spec) {
    const f = FRAMES[spec.mode];
    const c = document.createElement("canvas");
    c.width = Math.round(f.side * spec.scale);
    c.height = Math.round(f.side * spec.scale);
    return { c, f };
  }

  function drawFrame(engine, ctx, f, scale) {
    const sideOffset = (f.side - f.drawW) / 2;
    ctx.setTransform(scale, 0, 0, scale, sideOffset * scale, f.dy * scale);
    ctx.clearRect(-sideOffset, -f.dy, f.side, f.side);
    ctx.save();
    engine.applyDance(ctx, f.drawW, f.side);
    engine.draw(ctx, f.drawW, f.side);
    ctx.restore();
  }

  function call(engine, ev) {
    const [, method, ...args] = ev;
    if (method === "set") engine[args[0]] = args[1];
    else engine[method](...args);
  }

  // Steps the engine by `secs` in substeps of at most 1/30 s (the desktop
  // window's frame pace), firing virtual timers on the way.
  function advance(engine, secs) {
    let left = secs;
    while (left > 1e-9) {
      const dt = Math.min(1 / 30, left);
      clockMs += dt * 1000;
      runTimers();
      engine.update(dt);
      left -= dt;
    }
  }

  window.renderSequence = (spec) => {
    clockMs = 1000;
    rngState = spec.seed >>> 0;
    timers = [];
    timerSeq = 0;

    const engine = new window.__BotEngine();
    engine.particleOverhang = 0;
    engine.setState(spec.base, true);
    engine.setOutfit(spec.outfit, false);

    const events = (spec.events || []).slice().sort((a, b) => a[0] - b[0]);
    const pre = events.filter((e) => e[0] < 0);
    const post = events.filter((e) => e[0] >= 0);

    // Warm up, firing the negative-time events at their moment.
    let warmed = 0;
    for (const ev of pre) {
      const at = spec.warm + ev[0];
      if (at > warmed) { advance(engine, at - warmed); warmed = at; }
      call(engine, ev);
    }
    if (spec.warm > warmed) advance(engine, spec.warm - warmed);

    // Loops start on a calm frame (no blink or squash in flight) so the seam is clean.
    if (spec.calmStart) {
      let waited = 0;
      while (engine.tweens.size > 0 && waited < 6) { advance(engine, 1 / 30); waited += 1 / 30; }
    }

    const { c, f } = makeCanvas(spec);
    const ctx = c.getContext("2d");
    const frames = [];
    const n = spec.frames;
    let tRel = 0;
    let ei = 0;
    for (let i = 0; i < n; i++) {
      const target = i / spec.fps;
      // Fire events as the clock passes them.
      while (tRel < target - 1e-9) {
        const nextEv = ei < post.length ? post[ei][0] : Infinity;
        const stop = Math.min(target, nextEv);
        if (stop > tRel) { advance(engine, stop - tRel); tRel = stop; }
        while (ei < post.length && post[ei][0] <= tRel + 1e-9) call(engine, post[ei++]);
      }
      while (ei < post.length && post[ei][0] <= tRel + 1e-9) call(engine, post[ei++]);
      drawFrame(engine, ctx, f, spec.scale);
      frames.push(c.toDataURL("image/png").slice("data:image/png;base64,".length));
    }
    return frames;
  };
})();
