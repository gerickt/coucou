// The frame sequences rendered from the original engine (windows/src/mochi/
// engine.ts at 081a2ad). Every event below is a call on BotEngine's own public
// API, at the moment the original app made it:
//
// - state loops: BotEngine.setState(state), as the desktop window did on every
//   snapshot (windows/src/desktop/main.ts); lengths follow the engine's own
//   periods so loops close cleanly (badge dots 1/2.4 s, approval bounce
//   π/5.2 s, search scan 2π/2.6 s, dizzy eyes 2π/9 s, sleep breath and z's).
// - "enter" one-shots: the state change's own tween (approval hop, error shake,
//   finished roll + the desktop's happy 1.2 s, dizzy double roll).
// - reactions: slap() on click, triggerEmote("love") after a 1.9 s hover,
//   greet() on launch, and the dizzy recovery (setState back + happy) of
//   island.ts handleDizzy() after 3.3 s.
//
// Outfit matrix: "none" gets every sequence (all seven emotes and the gulp);
// each wardrobe outfit gets the state loops, the enters and the reactions the
// widgets play (slap, love, greet, dizzy). The bar gets the state loops.

export const OUTFITS = [
  "none", "partyHat", "beanie", "crown", "sunglasses", "roundGlasses",
  "bow", "scarf", "witchHat", "pumpkin", "santaHat", "bunnyEars",
];

export const STATES = [
  "idle", "working", "thinking", "searching", "approval", "question",
  "error", "finished", "ratelimit", "sleeping", "dizzy",
];

const DOTS = 1 / 2.4;
const BOUNCE = Math.PI / 5.2;
const SCAN = (2 * Math.PI) / 2.6;
const SPIRAL = (2 * Math.PI) / 9;
const BREATH = (2 * Math.PI) / 1.8;

/** Loop length (s) and base frame rate per state. */
const LOOPS = {
  idle: [5, 12],
  working: [3 * DOTS, 12],
  thinking: [3 * DOTS, 12],
  searching: [SCAN, 12],
  approval: [2 * BOUNCE, 15],
  question: [5, 12],
  error: [5, 12],
  finished: [5, 12],
  ratelimit: [2.6, 12],
  // Two breaths; the original desktop window drew a sleeping Mochi at 4 fps.
  sleeping: [2 * BREATH, 4],
  dizzy: [2 * SPIRAL, 15],
};

const REACTION_FPS = 15;

/** One-shots: [length s, base state, events]. */
const ENTERS = {
  "approval.enter": [0.8, "idle", [[0, "setState", "approval"]]],
  "error.enter": [0.8, "idle", [[0, "setState", "error"]]],
  "finished.enter": [1.8, "idle", [[0, "setState", "finished"], [0, "triggerEmote", "happy", 1.2]]],
  "dizzy.enter": [1.4, "idle", [[0, "setState", "dizzy"]]],
  "dizzy.exit": [1.6, "dizzy", [[0, "setState", "idle"], [0, "triggerEmote", "happy"]]],
};

const REACTIONS = {
  slap: [1.0, "idle", [[0, "slap"]]],
  love: [2.4, "idle", [[0, "triggerEmote", "love"]]],
  greet: [2.2, "idle", [[0, "greet"]]],
};

const EXTRAS = {
  happy: [2.0, "idle", [[0, "triggerEmote", "happy"]]],
  surprised: [2.0, "idle", [[0, "triggerEmote", "surprised"]]],
  proud: [2.4, "idle", [[0, "triggerEmote", "proud"]]],
  wink: [2.4, "idle", [[0, "triggerEmote", "wink"]]],
  yawn: [2.6, "idle", [[0, "triggerEmote", "yawn"]]],
  annoyed: [1.2, "idle", [[0, "triggerEmote", "annoyed"]]],
  // The file drop: mouth slot open (morph 1), gulp, happy, back to Mochi.
  gulp: [2.2, "idle", [[-0.7, "animateMorph", 1], [0, "gulp"], [0, "triggerEmote", "happy"], [0, "animateMorph", 0]]],
};

/** Stable per-name seed, so every outfit gets the same motion. */
function seedOf(s) {
  let h = 2166136261;
  for (const ch of s) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  return h >>> 0;
}

const MODES = {
  // 120 px window at 1.5x: sharp up to a 180 px widget.
  desktop: { scale: 1.5 },
  // 32 px bar icon at 2x, at two thirds of the desktop frame rate.
  bar: { scale: 2, fpsFactor: 2 / 3 },
};

function loopSeq(mode, outfit, state) {
  const [len, baseFps] = LOOPS[state];
  const fps = state === "sleeping" ? baseFps : baseFps * (MODES[mode].fpsFactor ?? 1);
  const frames = Math.max(1, Math.round(len * fps));
  return {
    name: `${outfit}.${state}`, mode, scale: MODES[mode].scale, outfit, base: state,
    kind: "loop", warm: 3, calmStart: true, frames, fps: frames / len,
    seed: seedOf(`${mode}.${state}`), events: [],
  };
}

function oneShot(mode, outfit, name, [len, base, events]) {
  const frames = Math.round(len * REACTION_FPS);
  return {
    name: `${outfit}.${name}`, mode, scale: MODES[mode].scale, outfit, base,
    kind: "once", warm: 3, calmStart: true, frames, fps: REACTION_FPS,
    seed: seedOf(`${mode}.${name}`), events,
  };
}

export const SEQUENCES = [];
for (const outfit of OUTFITS) {
  for (const s of STATES) SEQUENCES.push(loopSeq("desktop", outfit, s));
  for (const [n, d] of Object.entries(ENTERS)) SEQUENCES.push(oneShot("desktop", outfit, n, d));
  for (const [n, d] of Object.entries(REACTIONS)) SEQUENCES.push(oneShot("desktop", outfit, n, d));
  if (outfit === "none") for (const [n, d] of Object.entries(EXTRAS)) SEQUENCES.push(oneShot("desktop", outfit, n, d));
  for (const s of STATES) SEQUENCES.push(loopSeq("bar", outfit, s));
}
