--!nonstrict
-- Mini Mochi in the bar, the [[widget]] entry.
--
-- The original Mochi in the state's loop (frames rendered from the original
-- engine by tools/render-mochi, see sprite.luau), wearing the chosen outfit, a
-- dot when a request waits, and optionally the running tool. Dances while
-- music plays. Left click opens
-- the approval panel; right click (manifest action) pauses or resumes Mochi.
--
-- Pure view of noctalia.state("mochi"). Bar widgets may not receive
-- state.watch callbacks on every build, so update() also polls. The update
-- timer follows the next frame change (still frames poll at 400 ms).

local okSprite, sprite = pcall(require, "./sprite.luau")
if not okSprite then sprite = require("./sprite") end
sprite.setup(noctalia)

local PANEL = "coucou/mochi:approval"
local ICON = 26

local STATES = {
  idle = true, working = true, thinking = true, searching = true, approval = true,
  question = true, error = true, finished = true, ratelimit = true, sleeping = true,
}

local DOT = {
  approval = "#F5A524",
  question = "#22D3EE",
}

local player = sprite.player("bar")
local snap = { state = "idle", tool = "", paused = false, daemon = false, rev = -1 }
local lastTree = nil
local lastMs = noctalia.nowMs()
local outfitCheckMs = -1

local function baseState()
  if snap.paused then return "sleeping" end
  local s = snap.state
  if type(s) ~= "string" or not STATES[s] then return "idle" end
  return s
end

local function render()
  local state = baseState()
  local frame = player:frame()
  local tool = snap.tool
  local showTool = noctalia.getConfig("show_tool") ~= false and type(tool) == "string" and tool ~= ""
    and (state == "working" or state == "searching" or state == "approval" or state == "question")
  local key = table.concat({ tostring(frame), tostring(snap.pending ~= nil), if showTool then tool else "", tostring(snap.rev) }, "|")
  if key == lastTree then return end
  lastTree = key

  local children = {}
  if frame then
    table.insert(children, ui.image({
      path = frame, width = ICON, height = ICON, fit = "contain",
      opacity = if snap.paused or not snap.daemon then 0.6 else 1,
    }))
  end
  if snap.pending then
    table.insert(children, ui.box({ width = 6, height = 6, radius = 3, fill = DOT[state] or "#F5A524" }))
  end
  if showTool then
    table.insert(children, ui.label({ text = tool, maxWidth = 120, maxLines = 1 }))
  end
  barWidget.render(ui.row({ gap = 6, align = "center" }, children))

  local tip = noctalia.tr("state." .. (if snap.paused then "paused" elseif not snap.daemon then "offline" else state))
  if type(snap.project) == "string" and snap.project ~= "" then tip ..= " · " .. snap.project end
  barWidget.setTooltip(tip)
end

local function tick()
  local now = noctalia.nowMs()
  local secs = math.clamp((now - lastMs) / 1000, 0, 0.5)
  lastMs = now
  if now >= outfitCheckMs then
    outfitCheckMs = now + 1000
    player:setOutfit(sprite.resolveOutfit(noctalia.getConfig("outfit")))
  end
  -- Dances while music plays, by the compact island's rules (core/spotify.ts
  -- islandDances); the frames have the dance over the idle body.
  local loop = baseState()
  if loop == "idle" and snap.music == true and player:has("dance") then loop = "dance" end
  player:setLoop(loop)
  player:step(secs)
  render()
  noctalia.setUpdateInterval(math.floor(math.clamp(player:due() * 1000, 50, 400)))
end

local function refresh(v)
  if type(v) == "table" then snap = v end
  tick()
end

noctalia.state.watch("mochi", refresh)

function update()
  refresh(noctalia.state.get("mochi"))
end

function onClick()
  noctalia.togglePanel(PANEL)
end

refresh(noctalia.state.get("mochi"))
