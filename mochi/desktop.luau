--!nonstrict
-- The big Mochi, the [[desktop_widget]] entry.
--
-- Drawn from nested rounded ui boxes (Noctalia has no canvas): a soft body
-- with a shaded belly, two ink eyes whose shape follows the state, a badge
-- (!, ?, Zz, working dots), a ground shadow and the tool name underneath.
-- Animated per frame: idle bob, blinks, squash on every state change, eye
-- scans while searching, bounce while waiting for approval, breathing asleep.
--
-- Interactions (original island.ts): click = annoyed, three clicks within a
-- second = dizzy for 3.3 s, hovering 2 s = love, click while a request waits =
-- opens the approval panel.
--
-- Pure view: the state comes from the service through noctalia.state("mochi").

local PANEL = "coucou/mochi:approval"

-- ── Palette (original engine.ts) ─────────────────────────────────────────────

local BASE_TOP = { 0.929, 0.929, 0.937 } -- #EDEDEF
local BASE_BOTTOM = { 0.769, 0.773, 0.792 } -- #C4C5CA
local INK = "#1A1412"
local HEART = "#F25C7A"

local STATES = {
  idle = { color = { 0.902, 0.914, 0.933 }, tint = 0, eye = "pill" },
  working = { color = { 0.231, 0.62, 1 }, tint = 0.72, eye = "pill", badge = "dots" },
  thinking = { color = { 0.545, 0.361, 0.965 }, tint = 0.72, eye = "pill", badge = "dots", look = true },
  searching = { color = { 0.388, 0.396, 0.949 }, tint = 0.72, eye = "pill", badge = "dots", scans = true },
  approval = { color = { 0.961, 0.647, 0.141 }, tint = 0.78, eye = "wide", badge = "bang", bounces = true },
  question = { color = { 0.133, 0.827, 0.933 }, tint = 0.75, eye = "pill", badge = "question" },
  error = { color = { 0.957, 0.314, 0.369 }, tint = 0.78, eye = "flat", badge = "dot" },
  finished = { color = { 0.204, 0.831, 0.6 }, tint = 0.35, eye = "happy", badge = "dot" },
  ratelimit = { color = { 0.984, 0.573, 0.235 }, tint = 0.72, eye = "tired", badge = "dot", sweat = true },
  sleeping = { color = { 0.58, 0.635, 0.722 }, tint = 0.32, eye = "closed", zz = true, breathes = true },
  dizzy = { color = { 0.957, 0.447, 0.714 }, tint = 0.7, eye = "spiral" },
}

local function clamp01(v) return math.max(0, math.min(1, v)) end

local function hex(c, alpha)
  local s = string.format("#%02X%02X%02X", math.floor(clamp01(c[1]) * 255 + 0.5),
    math.floor(clamp01(c[2]) * 255 + 0.5), math.floor(clamp01(c[3]) * 255 + 0.5))
  if alpha then s ..= string.format("%02X", math.floor(clamp01(alpha) * 255 + 0.5)) end
  return s
end

local function mix(a, b, t)
  return { a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t }
end

-- ── Sounds of the interactions ───────────────────────────────────────────────

local loaded = {}
for _, name in { "slap", "love", "dizzy" } do
  noctalia.sound.load(name, "sounds/" .. name .. ".wav", function(ok) loaded[name] = ok end)
end

local function play(name)
  -- The global sound switch is a plugin-level setting, readable from here too.
  if loaded[name] and noctalia.getConfig("sound_enabled") ~= false then noctalia.sound.play(name) end
end

-- ── Model ────────────────────────────────────────────────────────────────────

local snap = { state = "idle", tool = "", paused = false, daemon = false }
local shownState = "idle"
local color = { 0.902, 0.914, 0.933 }
local tint = 0

local t = 0 -- seconds since start
local sinceRender = 1
local nextBlink = 2
local blinkAt = -1
local squashAt = -10
local emote = nil -- { eye = "heart" | "line", untilT = seconds }
local dizzyUntil = -1
local clicks = {}
local hoverSince = nil
local lovedThisHover = false

local function readSnap(v)
  if type(v) == "table" then snap = v end
end

readSnap(noctalia.state.get("mochi"))
noctalia.state.watch("mochi", readSnap)

local function squash() squashAt = t end
local function blink() blinkAt = t end

-- Piecewise-linear keyframes: { {value, ms}, ... } starting from 1.
local function keyframes(start, frames)
  local ms = (t - start) * 1000
  local from = 1
  for _, f in frames do
    if ms <= f[2] then return from + (f[1] - from) * (ms / f[2]) end
    ms -= f[2]
    from = f[1]
  end
  return from
end

-- ── Interactions ─────────────────────────────────────────────────────────────

function onMochiClick()
  table.insert(clicks, t)
  while #clicks > 0 and t - clicks[1] > 1.0 do table.remove(clicks, 1) end
  if #clicks >= 3 then
    clicks = {}
    dizzyUntil = t + 3.3
    emote = nil
    squash()
    play("dizzy")
    return
  end
  local state = snap.state
  if state == "approval" or state == "question" then
    noctalia.togglePanel(PANEL)
    return
  end
  emote = { eye = "line", untilT = t + 1.2 }
  squash()
  play("slap")
end

function onMochiHover(state)
  if state == "true" then
    hoverSince = t
    lovedThisHover = false
  else
    hoverSince = nil
  end
end

-- ── Drawing ──────────────────────────────────────────────────────────────────

local function eye(shape, side, S, open)
  local w, h = S * 0.11, S * 0.17
  if shape == "wide" then
    w, h = S * 0.135, S * 0.21
  elseif shape == "tired" then
    h = S * 0.085
  end
  if shape == "happy" then
    return ui.label({ text = "^", fontSize = S * 0.3, fontWeight = "bold", color = INK })
  elseif shape == "heart" then
    return ui.label({ text = "♥", fontSize = S * 0.22, color = HEART })
  elseif shape == "spiral" then
    return ui.label({ text = "@", fontSize = S * 0.2, fontWeight = "bold", color = INK })
  elseif shape == "line" then
    return ui.label({ text = if side == "left" then ">" else "<", fontSize = S * 0.24, fontWeight = "bold", color = INK })
  elseif shape == "closed" or shape == "flat" then
    local th = if shape == "closed" then S * 0.025 else S * 0.04
    return ui.box({ width = w * 1.3, height = math.max(2, th), radius = th / 2, fill = INK })
  end
  local eh = math.max(2, h * open)
  if open < 0.5 then
    return ui.box({ width = w, height = eh, radius = math.min(w, eh) / 2, fill = INK })
  end
  -- An open eye with its little highlight.
  local hl = w * 0.32
  return ui.column({ width = w, height = eh, radius = w / 2, fill = INK, padding = w * 0.16, align = "start" }, {
    ui.box({ width = hl, height = hl, radius = hl / 2, fill = "#FFFFFFD9" }),
  })
end

local function mouth(state, S, eyeShape)
  if eyeShape == "heart" or state == "finished" then
    return ui.box({ width = S * 0.11, height = S * 0.035, radius = S * 0.0175, fill = INK })
  elseif state == "approval" or state == "dizzy" then
    local d = S * 0.06
    return ui.box({ width = d, height = d, radius = d / 2, fill = INK })
  elseif state == "error" then
    return ui.box({ width = S * 0.08, height = S * 0.02, radius = S * 0.01, fill = INK })
  end
  return ui.box({ width = S * 0.06, height = S * 0.035, opacity = 0 })
end

local function circleBadge(text, c, b)
  return ui.column({ width = b, height = b, radius = b / 2, fill = hex(c), align = "center", justify = "center" }, {
    ui.label({ text = text, fontSize = b * 0.62, fontWeight = "heavy", color = "#FFFFFF", textAlign = "center" }),
  })
end

local function badge(cfg, state, S)
  local b = S * 0.2
  local c = cfg.color
  if cfg.badge == "bang" then
    return circleBadge("!", c, b * (1 + 0.08 * math.sin(t * 8)))
  elseif cfg.badge == "question" then
    return circleBadge("?", c, b)
  elseif cfg.badge == "dots" then
    local d = b * 0.3
    local dots = {}
    for i = 1, 3 do
      local a = 0.3 + 0.7 * math.sin(t * 5 - i * 0.9) ^ 2
      table.insert(dots, ui.box({ width = d, height = d, radius = d / 2, fill = hex(c), opacity = a }))
    end
    return ui.row({ gap = d * 0.6, align = "center" }, dots)
  elseif cfg.badge == "dot" then
    local d = b * 0.45
    return ui.box({ width = d, height = d, radius = d / 2, fill = hex(c) })
  elseif cfg.zz then
    return ui.label({ text = "Zz", fontSize = b * 0.7, fontWeight = "bold", color = hex(c), opacity = 0.5 + 0.5 * math.sin(t * 1.6) ^ 2 })
  end
  return ui.box({ width = 1, height = 1, opacity = 0 })
end

local function stateText()
  if snap.paused then return noctalia.tr("state.paused") end
  if not snap.daemon then return noctalia.tr("state.offline") end
  local showTool = noctalia.getConfig("show_tool") ~= false
  if showTool and type(snap.tool) == "string" and snap.tool ~= "" and shownState ~= "idle" and shownState ~= "sleeping" then
    return snap.tool
  end
  return noctalia.tr("state." .. shownState)
end

local function render()
  local S = tonumber(noctalia.getConfig("size")) or 120
  local cfg = STATES[shownState] or STATES.idle

  -- Body motion.
  local bob, sx, sy = 0, 1, 1
  if snap.paused then
    bob = 0
  elseif cfg.bounces then
    bob = math.abs(math.sin(t * 5.5)) * S * 0.07
  elseif cfg.breathes then
    sy = 1 + math.sin(t * 1.6) * 0.03
    sx = 1 - math.sin(t * 1.6) * 0.015
  else
    bob = (math.sin(t * 2.1) * 0.5 + 0.5) * S * 0.035
  end
  sy *= keyframes(squashAt, { { 0.78, 70 }, { 1.1, 130 }, { 1, 170 } })
  sx *= keyframes(squashAt, { { 1.16, 70 }, { 0.95, 130 }, { 1, 170 } })

  -- Eyes: emotes win over the state, blinks over open eyes.
  local shape = cfg.eye
  if emote then shape = emote.eye end
  if snap.paused then shape = "closed" end
  local open = keyframes(blinkAt, { { 0.06, 70 }, { 1, 130 } })
  local lookX, lookY = 0, 0
  if cfg.scans then lookX = math.sin(t * 2.4) * S * 0.06 end
  if cfg.look then lookX, lookY = S * 0.04, -S * 0.03 end
  if shownState == "dizzy" then lookX = math.sin(t * 9) * S * 0.03 end

  local bw, bh = S * sx, S * 0.86 * sy
  local radius = math.min(bw, bh) * 0.46
  local top = mix(BASE_TOP, color, tint * 0.45)
  local belly = mix(BASE_BOTTOM, color, tint * 0.6)
  local maxBob = S * 0.08
  local slack = math.max(0, S * 0.86 * 1.15 - bh) -- room the squash may take

  local eyeRowH = S * 0.24
  local eyesTop = bh * 0.28 + lookY
  local scanBase = S * 0.08

  local body = ui.column({
    width = bw, height = bh, radius = radius, fill = hex(top),
    border = if tint > 0.05 then hex(color, 0.55) else "#00000014",
    borderWidth = math.max(1, S * 0.018),
    align = "center",
    onClick = "onMochiClick",
    onHover = "onMochiHover",
  }, {
    ui.box({ height = math.max(0, eyesTop), width = 1, opacity = 0 }),
    ui.row({ height = eyeRowH, align = "center", justify = "center", gap = 0 }, {
      ui.box({ width = math.max(0, scanBase + lookX), height = 1, opacity = 0 }),
      eye(shape, "left", S, open),
      ui.box({ width = S * 0.16, height = 1, opacity = 0 }),
      eye(shape, "right", S, open),
      ui.box({ width = math.max(0, scanBase - lookX), height = 1, opacity = 0 }),
    }),
    ui.row({ height = S * 0.08, align = "center", justify = "center" }, { mouth(shownState, S, shape) }),
    ui.spacer({ flexGrow = 1 }),
    ui.box({ width = bw * 0.94, height = bh * 0.3, radius = radius * 0.9, fill = hex(belly, 0.85) }),
  })

  local sweat = if cfg.sweat
    then ui.box({ width = S * 0.06, height = S * 0.09, radius = S * 0.03, fill = "#7FC8FFDD", opacity = 1 - (t * 0.8 % 1) })
    else ui.box({ width = 1, height = 1, opacity = 0 })

  local shadowW = bw * 0.72 * (1 - bob / math.max(1, maxBob) * 0.25)
  desktopWidget.render(ui.column({ width = S * 1.5, align = "center", gap = 0, opacity = if snap.paused then 0.55 else 1 }, {
    ui.row({ width = S * 1.2, height = S * 0.26, justify = "space_between", align = "end" }, {
      sweat,
      badge(cfg, shownState, S),
    }),
    ui.box({ height = maxBob - bob + slack, width = 1, opacity = 0 }),
    body,
    ui.box({ height = bob + S * 0.02, width = 1, opacity = 0 }),
    ui.box({ width = shadowW, height = S * 0.05, radius = S * 0.025, fill = "#00000038" }),
    ui.label({
      text = stateText(), fontSize = math.max(10, S * 0.11), fontWeight = "semibold",
      color = "on_surface", textAlign = "center", maxWidth = S * 1.5, maxLines = 1,
    }),
  }))
end

-- ── Frame loop ───────────────────────────────────────────────────────────────

function onFrameTick(dt)
  local secs = (tonumber(dt) or 16) / 1000
  t += secs

  local state = snap.state or "idle"
  if t < dizzyUntil then state = "dizzy" end
  if state ~= shownState then
    shownState = state
    squash()
    blink()
  end
  local cfg = STATES[shownState] or STATES.idle
  -- Colours ease toward the state's, as in the original.
  local k = 1 - 0.002 ^ secs
  color = mix(color, cfg.color, k)
  tint += (cfg.tint - tint) * k

  if emote and t > emote.untilT then emote = nil end
  if hoverSince and not lovedThisHover and t - hoverSince > 2 then
    lovedThisHover = true
    emote = { eye = "heart", untilT = t + 2.5 }
    play("love")
  end
  if t > nextBlink then
    if shownState ~= "sleeping" and shownState ~= "dizzy" then
      blink()
      if math.random() < 0.22 then nextBlink = t + 0.23 else nextBlink = t + 2.2 + math.random() * 3.2 end
    else
      nextBlink = t + 2.2
    end
  end

  -- About 30 frames a second is plenty for this kind of motion.
  sinceRender += secs
  if sinceRender >= 0.033 then
    sinceRender = 0
    render()
  end
end

render()
desktopWidget.setNeedsFrameTick(true)
