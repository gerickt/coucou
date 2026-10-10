--!nonstrict
-- The big Mochi, the [[desktop_widget]] entry.
--
-- Mochi is the ORIGINAL character: every picture is a frame the original
-- engine drew (tools/render-mochi renders windows/src/mochi/engine.ts and
-- outfits.ts from 081a2ad, unmodified, into frames/). This entry only picks
-- which sequence plays, the way the original app called the engine:
--
-- - each agent state has its loop; error and finished first play the state
--   change's own tween (shake, roll + happy, desktop-logic.ts updateState); an
--   alert (approval, question) makes him surprised (desktop-logic.ts
--   beginRetract) over the alert's own body;
-- - click = slap (squash, annoyed eyes); three slaps within 1.7 s = dizzy for
--   3.3 s, then back to the state with a happy face (island.ts handleDizzy);
-- - hovering 1.9 s = love (at most every 6 s, island.ts scheduleLove); slap
--   and love are drawn over the current state's body when frames exist;
-- - a click while a request waits opens the approval panel;
-- - Mochi greets when the widget starts, with the launch sound and the user's
--   first name (identity.rs rules, from the passwd GECOS field);
-- - he falls asleep after 120 s without agent activity once the pointer is
--   away (desktop-logic.ts shouldSleep); hover, a click or agent work wakes
--   him; paused, he sleeps;
-- - he dances while music plays (the service's playerctl follow), in the
--   dance states (core/spotify.ts desktopDances); dancing keeps him awake;
-- - the outfit setting dresses him ("auto" follows the seasons), or the outfit
--   kept in the wardrobe panel; choosing one makes him proud with a pop
--   (island.ts chooseOutfit), and the wardrobe's hover tries one on.
--
-- No random idle emotes: the original's main Mochi has none (the engine's
-- doMiniBehaviorLoop only runs for mini bots).
--
-- With the plugin setting mochi_surface = "overlay" (the default) Mochi lives
-- on the coucou-mochi overlay above the windows instead: this widget, on the
-- Bottom layer under them, then draws nothing and stops ticking ("both"
-- keeps the two).
--
-- Pure view: the state comes from the service through noctalia.state("mochi").
-- Frames change at each sequence's own rate: per-frame ticks only while the
-- next change is close, otherwise one update() timer set to the next change.

local okSprite, sprite = pcall(require, "./sprite.luau")
if not okSprite then sprite = require("./sprite") end
sprite.setup(noctalia)

local PANEL = "coucou/mochi:approval"
local DEFAULT_SIZE = 160

local STATES = {
  idle = true, working = true, thinking = true, searching = true, approval = true,
  question = true, error = true, finished = true, ratelimit = true, sleeping = true,
}

-- State changes with a one-shot first: the engine's setState() tween, or the
-- surprise of an alert going up while he is on the desktop.
local ENTERS = { approval = "approval.alert", question = "question.alert", error = "error.enter", finished = "finished.enter" }

-- desktop-logic.ts: SLEEP_TIMEOUT, POINTER_MEMORY_MS.
local SLEEP_AFTER = 120
local POINTER_MEMORY = 4
-- core/spotify.ts DANCE_STATES; the frames have the dance over the idle body.
local DANCE_STATES = { idle = true, working = true, thinking = true, searching = true, finished = true }
local GREETING_SECS = 3.5

-- ── Sounds of the interactions ───────────────────────────────────────────────

-- The global sound switch is a plugin-level setting, readable from here too.
local okBank, soundbank = pcall(require, "./soundbank.luau")
if not okBank then soundbank = require("./soundbank") end
local okIdentity, identity = pcall(require, "./identity.luau")
if not okIdentity then identity = require("./identity") end
-- Muted from the shortcut (the service's "mute" pref) or the setting.
local function soundOn()
  local st = noctalia.state.get("mochi")
  return noctalia.getConfig("sound_enabled") ~= false and not (type(st) == "table" and st.muted == true)
end
local play = soundbank.new(noctalia, { "slap", "annoyed", "love", "dizzy", "hover", "pop", "greeting" }, soundOn)

-- ── Model ────────────────────────────────────────────────────────────────────

local player = sprite.player("desktop")
local snap = { state = "idle", tool = "", paused = false, daemon = false }
local t = 0 -- seconds since start
local shownState = nil
local dizzy = false
local slapTimes = {}
local hoverSince = nil
local toolsUntil = -1
local hovering = false -- pointer over Mochi or his buttons; shows the buttons
local lovedAt = -100
local timers = {} -- { at = seconds, fn = function }
local outfitCheckAt = -1
local outfitChoice = nil -- the outfit setting as last read
local dirty = true
local lastTree = nil
local lastAgentActive = 0 -- seconds; the start counts as activity
local lastPointer = -100
local asleep = false
local firstName = nil

local function later(secs, fn)
  table.insert(timers, { at = t + secs, fn = fn })
end

local function agentState()
  local s = snap.state
  if type(s) ~= "string" or not STATES[s] then return "idle" end
  return s
end

local function dancing()
  return snap.music == true and not snap.paused and DANCE_STATES[agentState()] == true
end

local function baseState()
  if snap.paused or asleep then return "sleeping" end
  return agentState()
end

-- The loop for a state: the dance replaces the idle loop while music plays.
local function loopFor(s)
  if s == "idle" and dancing() and player:has("dance") then return "dance" end
  return s
end

-- ── First name (identity.rs first_name_from, identity.luau) ─────────────────

firstNameFrom = identity.firstNameFrom

local function readFirstName()
  identity.read(noctalia, function(name)
    firstName = name
    dirty = true
  end)
end

-- The tool name only makes sense while the agent is busy with it.
local TOOL_STATES = { working = true, searching = true, approval = true, question = true }

local function stateText()
  if firstName and t < GREETING_SECS then return noctalia.tr("desktop.greeting", { name = firstName }) end
  if snap.paused then return noctalia.tr("state.paused") end
  if not snap.daemon then return noctalia.tr("state.offline") end
  local s = if dizzy then "dizzy" else (shownState or "idle")
  local showTool = noctalia.getConfig("show_tool") ~= false
  if showTool and type(snap.tool) == "string" and snap.tool ~= "" and TOOL_STATES[s] then
    return snap.tool
  end
  return noctalia.tr("state." .. s)
end

local function sync()
  if t >= outfitCheckAt then
    -- Settings (outfit, size, show_tool) are re-read once a second.
    outfitCheckAt = t + 1
    dirty = true
    -- The wardrobe's kept outfit (the service owns it) wins over the setting;
    -- an outfit the pointer rests on in the wardrobe is tried on.
    local choice = if type(snap.outfit_choice) == "string" then snap.outfit_choice else noctalia.getConfig("outfit")
    local outfit = if type(snap.preview) == "string" and snap.preview ~= "" then snap.preview else sprite.resolveOutfit(choice)
    if outfit ~= player.outfit then
      player:setOutfit(outfit)
      dirty = true
    end
    -- A new choice in the settings: pop, and he is proud of it.
    if outfitChoice ~= nil and choice ~= outfitChoice and not dizzy then
      play("pop")
      player:once("proud")
    end
    outfitChoice = choice
  end
  -- Agent work and the dance count as activity (desktop/main.ts updateSleep).
  if (agentState() ~= "idle" and agentState() ~= "sleeping") or dancing() then lastAgentActive = t end
  local sleep = not dizzy and t - lastAgentActive > SLEEP_AFTER and t - lastPointer >= POINTER_MEMORY
  if sleep ~= asleep then
    asleep = sleep
    dirty = true
  end
  if dizzy then return end
  local s = baseState()
  if s ~= shownState then
    local first = shownState == nil
    shownState = s
    -- A reaction drawn over the previous state's body would now be wrong.
    if not first then player:cancel() end
    player:setLoop(loopFor(s))
    if ENTERS[s] and not first then player:once(ENTERS[s]) end
    dirty = true
  else
    player:setLoop(loopFor(s))
  end
end

-- The pointer is near him: awake, and he stays so for POINTER_MEMORY.
local function notePointer()
  lastPointer = t
  if asleep then
    asleep = false
    dirty = true
  end
end

-- A reaction over the current state's body when the frames have it.
local function react(name)
  local s = shownState or "idle"
  if s ~= "idle" and player:once(s .. "." .. name) then return true end
  return player:once(name)
end

-- ── Interactions ─────────────────────────────────────────────────────────────

local function beginDizzy()
  dizzy = true
  slapTimes = {}
  hoverSince = nil
  play("dizzy")
  player:setLoop("dizzy")
  player:once("dizzy.enter")
  dirty = true
  later(3.3, function()
    dizzy = false
    shownState = baseState()
    player:setLoop(loopFor(shownState))
    player:once("dizzy.exit")
    dirty = true
  end)
end

function onMochiClick()
  notePointer()
  sync()
  local s = baseState()
  if not dizzy and (s == "approval" or s == "question") then
    noctalia.togglePanel(PANEL)
    return
  end
  if dizzy then return end
  table.insert(slapTimes, t)
  while #slapTimes > 0 and t - slapTimes[1] >= 1.7 do table.remove(slapTimes, 1) end
  if #slapTimes >= 3 then
    beginDizzy()
  else
    play("slap")
    later(0.06, function() play("annoyed") end)
    react("slap")
    dirty = true
  end
  wake()
end

function onMochiHover(state)
  local on = state == true or state == "true"
  hovering = on
  notePointer()
  if on and not hoverSince then
    if t - lovedAt >= 6 then play("hover") end
    hoverSince = t
  elseif not on then
    hoverSince = nil
    toolsUntil = t + 3
  end
  wake()
end

-- Desktop widgets take no right click on images, and a plugin cannot move its
-- own surface, so hovering Mochi shows small buttons: settings (left or right
-- click), agents (the Agents panel) and move, which opens Noctalia's desktop
-- widget editor.
local function msg(...)
  noctalia.runAsync({ "noctalia", "msg", ... }, function() end, 3000)
end

function onSettingsClick()
  msg("settings-open-plugin", "coucou/mochi")
end

function onAgentsClick()
  msg("panel-open", "coucou/mochi:agents")
end

function onChatClick()
  msg("panel-toggle", "coucou/mochi:chat")
end

function onMoveClick()
  msg("desktop-widgets-edit")
end

-- ── Drawing ──────────────────────────────────────────────────────────────────

local function render()
  local S = tonumber(noctalia.getConfig("size")) or DEFAULT_SIZE
  local frame = player:frame()
  local label = stateText()
  local tools = hovering or t < toolsUntil
  local key = tostring(frame) .. "|" .. label .. "|" .. S .. "|" .. tostring(tools)
  if key == lastTree then return end
  lastTree = key
  local picture = if frame
    then ui.image({ path = frame, width = S, height = S, fit = "contain", onClick = "onMochiClick", onHover = "onMochiHover" })
    else ui.box({ width = S, height = S, onClick = "onMochiClick", onHover = "onMochiHover" })
  desktopWidget.render(ui.column({ width = S, align = "center", gap = 0 }, {
    picture,
    ui.row({ width = S, align = "center", justify = "center", gap = 4 }, {
      ui.button({ glyph = "settings", variant = "ghost", controlSize = "sm", visible = tools,
        enabled = tools, onClick = "onSettingsClick", onRightClick = "onSettingsClick", onHover = "onMochiHover" }),
      ui.button({ glyph = "plug-connected", variant = "ghost", controlSize = "sm", visible = tools,
        enabled = tools, onClick = "onAgentsClick", onRightClick = "onAgentsClick", onHover = "onMochiHover",
        tooltip = noctalia.tr("desktop.agents") }),
      ui.button({ glyph = "message-circle", variant = "ghost", controlSize = "sm", visible = tools,
        enabled = tools, onClick = "onChatClick", onRightClick = "onChatClick", onHover = "onMochiHover",
        tooltip = noctalia.tr("desktop.chat") }),
      ui.label({
        text = label, fontSize = math.max(10, S * 0.08), fontWeight = "semibold",
        color = "on_surface", textAlign = "center", maxWidth = if tools then math.max(40, S - 148) else S, maxLines = 1,
      }),
      ui.button({ glyph = "arrows-move", variant = "ghost", controlSize = "sm", visible = tools,
        enabled = tools, onClick = "onMoveClick", onRightClick = "onSettingsClick", onHover = "onMochiHover" }),
    }),
  }))
end

-- ── Clock ────────────────────────────────────────────────────────────────────

local ticking = false
local lastStepMs = noctalia.nowMs()

local function setTicking(on)
  if on == ticking then return end
  ticking = on
  desktopWidget.setNeedsFrameTick(on)
end

-- Next moment something changes: a frame, a timer, the hover's love.
local function schedule()
  local due = player:due()
  for _, tm in timers do due = math.min(due, tm.at - t) end
  -- The moment he may fall asleep, and the end of the greeting's name.
  if not asleep then due = math.min(due, math.max(0.2, math.max(lastAgentActive + SLEEP_AFTER, lastPointer + POINTER_MEMORY) - t + 0.01)) end
  if firstName and t < GREETING_SECS then due = math.min(due, GREETING_SECS - t) end
  if hoverSince and t - lovedAt >= 6 then due = math.min(due, hoverSince + 1.9 - t) end
  if due < 0.12 then
    setTicking(true)
  else
    setTicking(false)
    noctalia.setUpdateInterval(math.floor(math.clamp(due * 1000, 50, 1000)))
  end
end

-- The overlay alone shows him: nothing here, a settings check once a second.
-- Read at most once a second (step runs per frame), and only when the
-- coucou-mochi binary is installed: without it the widget stays.
local overlayOnlyAt = -math.huge
local overlayOnlyNow = false

local function overlayOnly()
  local now = noctalia.nowMs()
  if now - overlayOnlyAt >= 1000 then
    overlayOnlyAt = now
    local installed = not noctalia.commandExists or noctalia.commandExists("coucou-mochi")
    overlayOnlyNow = installed and noctalia.getConfig("mochi_surface") == "overlay"
  end
  return overlayOnlyNow
end

local function stepAside()
  if lastTree ~= "aside" then
    lastTree = "aside"
    desktopWidget.render(ui.box({ width = 1, height = 1 }))
  end
  setTicking(false)
  noctalia.setUpdateInterval(1000)
end

local function step(secs)
  if overlayOnly() then return stepAside() end
  if lastTree == "aside" then dirty = true end
  t += secs
  local i = 1
  while i <= #timers do
    local tm = timers[i]
    if tm.at <= t then
      table.remove(timers, i)
      tm.fn()
    else
      i += 1
    end
  end
  sync()
  if hoverSince and not dizzy and t - hoverSince >= 1.9 and t - lovedAt >= 6 then
    lovedAt = t
    hoverSince = nil
    if react("love") then play("love") end
    dirty = true
  end
  if player:step(secs) then dirty = true end
  if dirty then
    dirty = false
    render()
  end
  schedule()
end

function wake()
  local now = noctalia.nowMs()
  local secs = math.clamp((now - lastStepMs) / 1000, 0, 0.5)
  lastStepMs = now
  step(secs)
end

function onFrameTick(dt)
  lastStepMs = noctalia.nowMs()
  step(math.clamp((tonumber(dt) or 16) / 1000, 0, 0.5))
end

function update()
  if ticking then return end
  wake()
end

-- A one-shot the service asks for (the gulp when a file goes to the chat);
-- the one already there when the widget starts is not replayed.
local reactionRev = nil

local function readReaction()
  local r = snap.reaction
  local rev = if type(r) == "table" then r.rev else nil
  if reactionRev ~= nil and rev ~= nil and rev ~= reactionRev and not dizzy and type(r.name) == "string" then
    notePointer()
    react(r.name)
  end
  reactionRev = rev or reactionRev or 0
end

local function readSnap(v)
  if type(v) == "table" then
    -- A wardrobe preview or a kept outfit shows at once, not at the next second.
    if v.preview ~= snap.preview or v.outfit_choice ~= snap.outfit_choice then outfitCheckAt = -1 end
    snap = v
  end
  readReaction()
  dirty = true
  wake()
end

-- ── Start: hello! ────────────────────────────────────────────────────────────

if type(noctalia.state.get("mochi")) == "table" then snap = noctalia.state.get("mochi") end
readReaction()
sync()
if not snap.paused and not overlayOnly() and player:once("greet") then play("greeting") end
readFirstName()
noctalia.state.watch("mochi", readSnap)
step(0)
