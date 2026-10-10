--!nonstrict
-- Mini Mochi in the bar, the [[widget]] entry.
--
-- A small rounded body with two eyes, tinted by the state, a dot when a
-- request waits, and optionally the running tool. Left click opens the
-- approval panel; right click (manifest action) pauses or resumes Mochi.
--
-- Pure view of noctalia.state("mochi"). Bar widgets may not receive
-- state.watch callbacks on every build, so update() also polls.

local PANEL = "coucou/mochi:approval"
local INK = "#10131A"
local BASE = { 0.929, 0.929, 0.937 }

local COLORS = {
  idle = { 0.902, 0.914, 0.933 },
  working = { 0.231, 0.62, 1 },
  thinking = { 0.545, 0.361, 0.965 },
  searching = { 0.388, 0.396, 0.949 },
  approval = { 0.961, 0.647, 0.141 },
  question = { 0.133, 0.827, 0.933 },
  error = { 0.957, 0.314, 0.369 },
  finished = { 0.204, 0.831, 0.6 },
  ratelimit = { 0.984, 0.573, 0.235 },
  sleeping = { 0.58, 0.635, 0.722 },
}

local function hex(c, alpha)
  local s = string.format("#%02X%02X%02X", math.floor(c[1] * 255 + 0.5), math.floor(c[2] * 255 + 0.5), math.floor(c[3] * 255 + 0.5))
  if alpha then s ..= string.format("%02X", math.floor(alpha * 255 + 0.5)) end
  return s
end

local function mix(a, b, t)
  return { a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t }
end

local snap = { state = "idle", tool = "", paused = false, daemon = false, rev = -1 }
local renderedRev = -2
local ticks = 0
local blinking = false

local function eyes(state, paused)
  if paused or state == "sleeping" or blinking then
    local line = ui.box({ width = 4, height = 1.5, radius = 1, fill = INK })
    return { line, line }
  end
  if state == "finished" then
    local happy = ui.label({ text = "^", fontSize = 9, fontWeight = "bold", color = INK })
    return { happy, happy }
  end
  local h = if state == "approval" then 6 else 5
  local e = ui.box({ width = 3, height = h, radius = 1.5, fill = INK })
  return { e, e }
end

local function render()
  local state = snap.state or "idle"
  local c = COLORS[state] or COLORS.idle
  local body = mix(BASE, c, if state == "idle" then 0 else 0.45)
  local pair = eyes(state, snap.paused)

  local children = {
    ui.column({
      width = 20, height = 16, radius = 7, fill = hex(body),
      border = if state == "idle" then "#00000022" else hex(c, 0.8), borderWidth = 1,
      align = "center", justify = "center",
      opacity = if snap.paused or not snap.daemon then 0.5 else 1,
    }, {
      ui.row({ gap = 4, align = "center", justify = "center" }, pair),
    }),
  }
  if snap.pending then
    table.insert(children, ui.box({ width = 6, height = 6, radius = 3, fill = hex(c) }))
  end
  local tool = snap.tool
  if noctalia.getConfig("show_tool") ~= false and type(tool) == "string" and tool ~= "" and state ~= "idle" and state ~= "sleeping" then
    table.insert(children, ui.label({ text = tool, maxWidth = 120, maxLines = 1 }))
  end
  barWidget.render(ui.row({ gap = 6, align = "center" }, children))

  local tip = noctalia.tr("state." .. (if snap.paused then "paused" elseif not snap.daemon then "offline" else state))
  if type(snap.project) == "string" and snap.project ~= "" then tip ..= " · " .. snap.project end
  barWidget.setTooltip(tip)
end

local function refresh(v)
  if type(v) == "table" then snap = v end
  if snap.rev ~= renderedRev then
    renderedRev = snap.rev
    render()
  end
end

noctalia.state.watch("mochi", refresh)

function update()
  refresh(noctalia.state.get("mochi"))
  -- A blink every few seconds, one tick long.
  ticks += 1
  local was = blinking
  blinking = ticks % 9 == 0
  if blinking ~= was then render() end
end

function onClick()
  noctalia.togglePanel(PANEL)
end

noctalia.setUpdateInterval(400)
refresh(noctalia.state.get("mochi"))
render()
