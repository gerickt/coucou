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
// - P4 additions, also BotEngine calls the original app made: proud on an
//   outfit change (island.ts chooseOutfit), surprised when an alert goes up
//   while he is on the desktop (desktop-logic.ts beginRetract, played over the
//   alert state's own setState), the dance (setDancing(true) while music plays,
//   desktop/main.ts and core/spotify.ts desktopDances), and the slap and love
//   reactions drawn over the CURRENT state's body ("<state>.slap",
//   "<state>.love") instead of the idle one.
//
// Outfit matrix: "none" gets every sequence (all seven emotes and the
// reactions over every state); each wardrobe outfit gets the state loops,
// the enters, the reactions the widgets play (slap, love, greet, dizzy), proud,
// the alerts, the gulp and the dance. The bar gets the state loops and the dance. Kept
// out for size (mochi/frames stays under ~25 MB): the per-state reactions of
// the wardrobe outfits (the player falls back to the idle-body reaction).

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

// Every outfit: proud (outfit change) and the alerts (surprised + setState).
const DRESSED = {
  proud: [2.4, "idle", [[0, "triggerEmote", "proud"]]],
  "approval.alert": [2.0, "idle", [[0, "setState", "approval"], [0, "triggerEmote", "surprised"]]],
  "question.alert": [2.0, "idle", [[0, "setState", "question"], [0, "triggerEmote", "surprised"]]],
  // The file drop (island.ts swallow): mouth slot open (morph 1), gulp, happy, back to Mochi.
  gulp: [2.2, "idle", [[-0.7, "animateMorph", 1], [0, "gulp"], [0, "triggerEmote", "happy"], [0, "animateMorph", 0]]],
};

/** States whose body the "none" outfit's slap and love are drawn over. */
const SLAP_OVER = ["working", "thinking", "searching", "error", "finished", "ratelimit"];
const LOVE_OVER = ["working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit"];

const EXTRAS = {
  happy: [2.0, "idle", [[0, "triggerEmote", "happy"]]],
  surprised: [2.0, "idle", [[0, "triggerEmote", "surprised"]]],
  wink: [2.4, "idle", [[0, "triggerEmote", "wink"]]],
  yawn: [2.6, "idle", [[0, "triggerEmote", "yawn"]]],
  annoyed: [1.2, "idle", [[0, "triggerEmote", "annoyed"]]],
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

// The dance: 112 BPM, its sway closes every two beats (danceTransform). The
// level is full before the first frame (warm-up > 0.3 s fade-in). The loop is
// one sway, so a blink inside it would repeat every 1.07 s: the next blink is
// pushed past the clip (the engine still draws every pixel).
const DANCE_LEN = (2 * 60) / 112;
const DANCE_FPS = 20;

function danceSeq(mode, outfit) {
  const fps = DANCE_FPS * (MODES[mode].fpsFactor ?? 1);
  const frames = Math.max(1, Math.round(DANCE_LEN * fps));
  return {
    name: `${outfit}.dance`, mode, scale: MODES[mode].scale, outfit, base: "idle",
    kind: "loop", warm: 3, calmStart: true, frames, fps: frames / DANCE_LEN,
    seed: seedOf(`${mode}.dance`),
    events: [[-2.9, "setDancing", true], [-0.01, "set", "nextBlink", 1e9], [0, "set", "nextBlink", 1e9]],
  };
}

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

// P14 gaze: the overlay's eyes follow the pointer. The original desktop window
// set engine.lookX / lookY from the cursor every frame (desktop/main.ts draw,
// desktop-logic.ts gaze: tanh of the distance, x right +, y up +); here each
// state loop gets eight still directions, set before the warm-up so the
// engine's own easing has settled. Same seed as the plain loop, so blinks fall
// on the same frames and the overlay can swap variants without a hitch.
export const GAZE_STATES = ["idle", "working"];
const D = 0.7;
export const GAZE = {
  n: [0, 0.9], ne: [D, D], e: [0.9, 0], se: [D, -D],
  s: [0, -0.9], sw: [-D, -D], w: [-0.9, 0], nw: [-D, D],
};

function gazeSeq(mode, outfit, state, dir) {
  const [x, y] = GAZE[dir];
  const q = loopSeq(mode, outfit, state);
  return {
    ...q,
    name: `${outfit}.${state}.look.${dir}`,
    events: [[-2.95, "set", "lookX", x], [-2.95, "set", "lookY", y]],
  };
}

export const SEQUENCES = [];
for (const outfit of OUTFITS) {
  for (const s of STATES) SEQUENCES.push(loopSeq("desktop", outfit, s));
  for (const s of GAZE_STATES) for (const dir of Object.keys(GAZE)) SEQUENCES.push(gazeSeq("desktop", outfit, s, dir));
  for (const [n, d] of Object.entries(ENTERS)) SEQUENCES.push(oneShot("desktop", outfit, n, d));
  for (const [n, d] of Object.entries(REACTIONS)) SEQUENCES.push(oneShot("desktop", outfit, n, d));
  for (const [n, d] of Object.entries(DRESSED)) SEQUENCES.push(oneShot("desktop", outfit, n, d));
  SEQUENCES.push(danceSeq("desktop", outfit));
  if (outfit === "none") {
    for (const [n, d] of Object.entries(EXTRAS)) SEQUENCES.push(oneShot("desktop", outfit, n, d));
    for (const st of SLAP_OVER) SEQUENCES.push(oneShot("desktop", outfit, `${st}.slap`, [REACTIONS.slap[0], st, REACTIONS.slap[2]]));
    for (const st of LOVE_OVER) SEQUENCES.push(oneShot("desktop", outfit, `${st}.love`, [REACTIONS.love[0], st, REACTIONS.love[2]]));
  }
  for (const s of STATES) SEQUENCES.push(loopSeq("bar", outfit, s));
  SEQUENCES.push(danceSeq("bar", outfit));
}
