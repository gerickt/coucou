--!nonstrict
-- Mochi's brain, the headless [[service]] entry.
--
-- Follows `coucou-daemon --follow` (one JSON line per hook event), runs the
-- state machine of the original app (windows/src/island/hooks.ts), plays the
-- sounds the original plays on those events, and publishes one snapshot to
-- noctalia.state("mochi") that the desktop widget, the bar widget and the
-- panel all render. Also follows the MPRIS players (playerctl, when there is
-- one) so Mochi dances while music plays (core/spotify.ts desktopDances).
--
-- Sleep is the desktop widget's (desktop-logic.ts shouldSleep), as in the
-- original: the island, here the bar, never dozes off.
--
-- Permission requests: the card is acknowledged at once (the daemon only waits
-- for a human once acknowledged, within 800 ms) and the approval panel opens.
-- Nothing is ever approved here: decisions come only from a click in the panel.
--
-- The chat (chat.luau is its view): one turn at a time through
-- `coucou-daemon chat --request <file>`, whose JSON lines stream the answer.
-- The daemon looks the API key up itself; it never reaches this script. Mochi
-- thinks while a turn runs, as the original island did.
--
-- The coucou-mochi overlay (P14): Noctalia's desktop widgets sit under the
-- windows, so Mochi also floats above them on a layer-shell surface drawn by
-- the coucou-mochi binary. With the `mochi_surface` setting at "overlay" (the
-- default) or "both", this service starts it (`--managed`: it exits with the
-- shell, or when this service stops writing) and checks every 30 s that it
-- still runs; "desktop_widget" asks it to quit. Mochi's state and the
-- overlay's settings go to $XDG_RUNTIME_DIR/coucou-mochi.json, written when
-- they change and every 20 s as a heartbeat. The overlay plays its own
-- interaction sounds (slap, love…); the agent sounds stay here.
--
-- Sessions (P9): every session is a pill with its agent's colour and a badge
-- (approval, finished, error) as the original island's; Stop keeps the agent's
-- final line for the finished card and posts a notification with it (the
-- original surfaced its finished card); "Open terminal" runs
-- `coucou-daemon open-terminal <session>`, which focuses the session's window.
--
-- Shortcuts and wardrobe (P10): the original's global shortcuts are IPC
-- events here, bound in Hyprland (README). Noctalia has no API to write a
-- plugin setting, so what they change (mute, Mochi sent off the desktop, the
-- outfit kept in the wardrobe) is kept in pluginDataDir/prefs.json; a kept
-- outfit gives way as soon as the outfit setting itself is changed.
--
-- IPC: noctalia msg plugin coucou/mochi:daemon all <event> [payload]
--   toggle-pause | pause | resume | open | agents | chat | attach <absolute path>
--   island (open/close the panel) | alert | terminal [session] | next | prev
--   mute | desktop | wardrobe | menu | focus <session> | dismiss
--   outfit-preview <outfit|""> | outfit-keep <selection>

local KEY = "mochi"
local CHAT_KEY = "mochi.chat"
local CHAT_CMD = "mochi.chat.cmd"
local PANEL = "coucou/mochi:approval"
local AGENTS_PANEL = "coucou/mochi:agents"
local CHAT_PANEL = "coucou/mochi:chat"
local DAEMON = "coucou-daemon"

-- Agents whose permission requests can be answered (coucou-hook reply.rs).
local APPROVAL_AGENTS = { [""] = true, ["claude-desktop"] = true, codex = true, copilot = true, muse = true }
-- Only Claude Code asks AskUserQuestion through a permission request.
local QUESTION_AGENTS = { [""] = true, ["claude-desktop"] = true }
local SEARCH_TOOLS = {
  Grep = true, Glob = true, LS = true, WebSearch = true, WebFetch = true, ToolSearch = true,
  grep = true, glob = true, search = true, google_web_search = true, web_fetch = true,
}
-- The original's sound triggers (hooks.ts and island.ts): SessionStart plays
-- "work", a rate limit "rate", Stop "finish", StopFailure "error", a card
-- "approval" or "question" as the island opens ("open"); a decision "approve"
-- (allow, answer) or "blip" (deny, back to the terminal), then "close"; coming
-- back from pause "peek". Thinking, searching and sleeping play nothing there.
-- The chat plays "send", then "finish" or "error" (chat.ts); a file Mochi
-- swallows plays "approve" (island.ts swallow).
local SOUNDS = { "work", "rate", "finish", "error", "approval", "question", "open", "approve", "blip", "close", "peek", "send" }

local FINISHED_HOLD_MS = 5200
local FINISHED_CARD_MS = 30 * 1000
local SESSION_FORGET_MS = 60 * 60 * 1000
local STREAM_STALE_MS = 45 * 1000
local MAX_SESSIONS = 8
-- The daemon gives up after 300 s; past this a silent turn is over.
local CHAT_STALE_MS = 320 * 1000

local function now() return noctalia.nowMs() end

local okPills, pills = pcall(require, "./pills.luau")
if not okPills then pills = require("./pills") end
local okIdentity, identity = pcall(require, "./identity.luau")
if not okIdentity then identity = require("./identity") end

local WARDROBE_PANEL = "coucou/mochi:wardrobe"
local MENU_PANEL = "coucou/mochi:menu"

-- ── Prefs the shortcuts and the wardrobe change (no setting-write API) ───────

local prefs = { muted = false, away = false, outfit = nil } -- outfit = { choice, setting }
local PREFS_FILE = "prefs.json"

local function prefsPath()
  local dir = noctalia.pluginDataDir and noctalia.pluginDataDir()
  if type(dir) ~= "string" or dir == "" then return nil end
  return dir .. "/" .. PREFS_FILE
end

local function loadPrefs()
  local path = prefsPath()
  if not path then return end
  local text = noctalia.readFile(path)
  if type(text) ~= "string" then return end
  local ok, v = pcall(noctalia.json.decode, text)
  if not ok or type(v) ~= "table" then return end
  prefs.muted = v.muted == true
  prefs.away = v.away == true
  if type(v.outfit) == "table" and type(v.outfit.choice) == "string" then
    prefs.outfit = { choice = v.outfit.choice, setting = if type(v.outfit.setting) == "string" then v.outfit.setting else "auto" }
  end
end

local function savePrefs()
  local path = prefsPath()
  if not path or type(noctalia.writeFile) ~= "function" then return end
  local body = noctalia.json.encode({ muted = prefs.muted, away = prefs.away, outfit = prefs.outfit or false })
  if body then noctalia.writeFile(path, body) end
end

local function outfitSetting()
  local v = noctalia.getConfig("outfit")
  return if type(v) == "string" then v else "auto"
end

--- The outfit selection in force: the one kept in the wardrobe, until the
--- outfit setting is changed after it.
local function outfitChoice()
  local kept = prefs.outfit
  if kept and kept.setting == outfitSetting() then return kept.choice end
  return outfitSetting()
end

loadPrefs()
local shownChoice = outfitChoice()

local view = {
  state = "idle",
  tool = "",
  agent = "",
  project = "",
  paused = false,
  daemon = false,
  pending = nil,
  sessions = {},
  rev = 0,
  stateSince = now(),
  music = false,
  reaction = nil, -- { name, rev }: a one-shot for the desktop Mochi (the gulp)
  finished = nil, -- the finished card: { session, agent, project, line, at }
  preview = nil, -- an outfit tried on in the wardrobe
}
-- session id -> { id, agent, project, cwd, state, tool, updated, finishedAt, final, badge }
local sessions = {}
local focus = nil -- the session Mochi is showing

-- ── Sounds ───────────────────────────────────────────────────────────────────

local okBank, soundbank = pcall(require, "./soundbank.luau")
if not okBank then soundbank = require("./soundbank") end
local okWardrobe, wardrobe = pcall(require, "./wardrobe.luau")
if not okWardrobe then wardrobe = require("./wardrobe") end
local function soundOn() return noctalia.getConfig("sound_enabled") ~= false and not prefs.muted end
local play = soundbank.new(noctalia, SOUNDS, soundOn)

-- ── Publishing ───────────────────────────────────────────────────────────────

local function sessionList()
  local list = {}
  for _, s in sessions do
    local badge = s.badge
    if view.pending and view.pending.session == s.id then badge = "approval" end
    table.insert(list, {
      id = s.id, agent = s.agent, project = s.project, cwd = s.cwd, state = s.state, tool = s.tool,
      updated = s.updated, final = s.final, badge = badge, focused = s.id == focus,
      name = pills.agentName(s.agent), color = pills.agentColor(s.agent),
    })
  end
  table.sort(list, function(a, b) return a.updated > b.updated end)
  while #list > MAX_SESSIONS do table.remove(list) end
  return list
end

local publishOverlay -- below: the overlay's copy of the snapshot

local function publish()
  view.rev += 1
  view.sessions = sessionList()
  noctalia.state.set(KEY, {
    state = view.state,
    tool = view.tool,
    agent = view.agent,
    project = view.project,
    paused = view.paused,
    daemon = view.daemon,
    music = view.music,
    pending = view.pending,
    reaction = view.reaction,
    sessions = view.sessions,
    finished = view.finished,
    focus = focus,
    muted = prefs.muted,
    away = prefs.away,
    outfit_choice = outfitChoice(),
    preview = view.preview,
    rev = view.rev,
  })
  publishOverlay()
end

local chat = {
  messages = {}, -- { role, content, failed? }
  live = "", -- the answer so far
  busy = false,
  error = "",
  file = nil, -- { name, path }: rides with the first question
  pid = nil,
  rev = 0,
  lastLine = 0,
}

-- Mochi shows the pending card first, then a chat turn running, else the
-- session that spoke last.
local function recompute()
  local before = view.state
  if view.pending then
    view.state = view.pending.kind
    view.tool = view.pending.tool or ""
    view.agent = view.pending.agent or ""
    view.project = view.pending.project or ""
  elseif chat.busy then
    view.state = "thinking"
    view.tool = ""
  elseif focus and sessions[focus] then
    local s = sessions[focus]
    view.state = s.state
    view.tool = s.tool
    view.agent = s.agent
    view.project = s.project
  else
    view.state = "idle"
    view.tool = ""
  end
  if view.state ~= before then view.stateSince = now() end
  publish()
end

-- ── Daemon commands (argv, never a shell) ────────────────────────────────────

local function daemon(args)
  local argv = { DAEMON }
  for _, a in args do table.insert(argv, a) end
  noctalia.runAsync(argv, function(res)
    if res.exitCode ~= 0 then
      noctalia.log("mochi: " .. table.concat(argv, " ") .. " failed: " .. (res.stderr or ""))
    end
  end, 3000)
end

local function safeId(id)
  return type(id) == "string" and id:match("^[%w%-]+$") ~= nil
end

local function openPanel(context, id)
  local argv = { "noctalia", "msg", "panel-open", id or PANEL }
  if context then table.insert(argv, context) end
  noctalia.runAsync(argv, nil)
end

-- ── Hook events ──────────────────────────────────────────────────────────────

local function basename(path)
  if type(path) ~= "string" or path == "" then return "" end
  return path:match("([^/]+)/*$") or path
end

local function session(p)
  local id = p.session_id
  if type(id) ~= "string" or id == "" then id = (p.coucou_agent or "claude") .. ":" .. (p.cwd or "") end
  local s = sessions[id]
  if not s then
    s = { id = id, agent = "", project = "", cwd = "", state = "idle", tool = "", updated = now() }
    sessions[id] = s
  end
  s.agent = p.coucou_agent or ""
  local project = basename(p.cwd)
  if project ~= "" then
    s.project = project
    s.cwd = p.cwd
  end
  s.updated = now()
  focus = id
  return s
end

-- What the approval card shows: the command, the file, the URL…
local function approvalTarget(tool, input)
  if type(input) ~= "table" then return "" end
  for _, k in { "command", "file_path", "path", "url", "pattern", "query", "prompt", "description" } do
    local v = input[k]
    if type(v) == "string" and v ~= "" then return v end
  end
  return ""
end

local function askedQuestions(agent, tool, input)
  if not QUESTION_AGENTS[agent] or tool ~= "AskUserQuestion" or type(input) ~= "table" then return nil end
  local qs = input.questions
  if type(qs) ~= "table" or #qs == 0 then return nil end
  local out = {}
  for _, q in qs do
    if type(q) ~= "table" or type(q.question) ~= "string" or type(q.options) ~= "table" then return nil end
    local options = {}
    for _, o in q.options do
      if type(o) == "table" and type(o.label) == "string" then
        table.insert(options, { label = o.label, description = if type(o.description) == "string" then o.description else "" })
      end
    end
    if #options == 0 then return nil end
    table.insert(out, {
      question = q.question,
      header = if type(q.header) == "string" then q.header else "",
      multiSelect = q.multiSelect == true,
      options = options,
    })
  end
  return out
end

-- True when the card was taken; false when the request went back to the
-- terminal (or another card already holds the view).
local function onPermission(p, s)
  local id = p.request_id
  if p.coucou_declined or not safeId(id) then return false end
  local agent = p.coucou_agent or ""
  if not APPROVAL_AGENTS[agent] or view.paused or (view.pending and view.pending.id ~= id) then
    daemon({ "decline", id })
    return false
  end
  local tool = p.tool_name or "Tool"
  local questions = askedQuestions(agent, tool, p.tool_input)
  daemon({ "ack", id })
  view.pending = {
    id = id,
    kind = if questions then "question" else "approval",
    tool = tool,
    target = approvalTarget(tool, p.tool_input),
    agent = agent,
    project = s.project,
    cwd = p.cwd or "",
    session = s.id,
    questions = questions,
  }
  s.state = view.pending.kind
  s.tool = tool
  recompute()
  play(view.pending.kind)
  play("open")
  openPanel(id)
  return true
end

-- Stop: the finished card (shown by the panel) and a notification with the
-- agent's final line. A card waiting for an answer is never covered: then the
-- session only gets its badge (hooks.ts).
local function onFinished(s)
  if view.pending then
    s.badge = "finished"
    return
  end
  s.badge = nil
  view.finished = { session = s.id, agent = s.agent, project = s.project, line = s.final or "", at = now() }
  if noctalia.getConfig("notify_finished") ~= false and type(noctalia.notify) == "function" then
    local title = noctalia.tr("finished.title", { agent = pills.agentName(s.agent) })
    if s.project ~= "" then title ..= " · " .. s.project end
    noctalia.notify(title, s.final or noctalia.tr("finished.none"))
  end
end

local function onHook(p)
  local name = p.hook_event_name or ""
  if view.paused then
    if name == "PermissionRequest" and safeId(p.request_id) and not p.coucou_declined then
      daemon({ "decline", p.request_id })
    end
    return
  end
  local previousFocus = focus
  local s = session(p)
  if name == "SessionStart" then
    s.state = "idle"
    play("work")
  elseif name == "UserPromptSubmit" then
    s.state = "thinking"
    s.tool = ""
    -- The final message stays on the card until the next turn starts.
    s.final = nil
    s.badge = nil
    if view.finished and view.finished.session == s.id then view.finished = nil end
  elseif name == "PreToolUse" then
    s.tool = p.tool_name or ""
    s.state = if SEARCH_TOOLS[s.tool] then "searching" else "working"
  elseif name == "PostToolUse" or name == "PostToolUseFailure" then
    s.state = "working"
  elseif name == "Notification" then
    local message = tostring(p.message or "")
    local lower = message:lower()
    if lower:find("rate limit", 1, true) or lower:find("limite d", 1, true) then
      s.state = "ratelimit"
      play("rate")
    elseif message:sub(-1) == "?" then
      s.state = "question"
    end
  elseif name == "Stop" then
    s.state = "finished"
    s.finishedAt = now()
    -- Claude Code puts the turn's answer in the Stop payload; other agents
    -- report it the same way or as `message` (hooks.ts).
    local line = pills.toOneLine(p.last_assistant_message or p.message or "")
    s.final = if line ~= "" then line else nil
    play("finish")
    onFinished(s)
  elseif name == "Interrupt" then
    s.state = "idle"
    s.badge = nil
  elseif name == "StopFailure" then
    s.state = "error"
    s.badge = "error"
    play("error")
  elseif name == "SessionEnd" then
    sessions[s.id] = nil
    if focus == s.id then focus = nil end
    if view.finished and view.finished.session == s.id then view.finished = nil end
  elseif name == "PermissionRequest" then
    -- A request that went back to the terminal does not take Mochi over.
    if not onPermission(p, s) then focus = previousFocus end
    return
  else
    -- Subagent events and the like keep the current state.
  end
  recompute()
end

local function onResolved(msg)
  local pending = view.pending
  if not pending or pending.id ~= msg.request_id then return end
  view.pending = nil
  local outcome = msg.outcome
  local s = sessions[pending.session]
  if s then focus = s.id end
  if s and (s.state == "approval" or s.state == "question") then
    s.state = if outcome == "allow" or outcome == "answered" then "working" else "idle"
  end
  if outcome == "allow" or outcome == "answered" then
    play("approve")
  elseif outcome == "deny" or outcome == "declined" then
    play("blip")
  end
  play("close")
  recompute()
end

local function onHello(msg)
  view.daemon = true
  -- A request still waiting from before a reload: show it again.
  local first = type(msg.pending) == "table" and msg.pending[1]
  if first and not view.pending and not view.paused then
    onHook(first)
  else
    publish()
  end
end

local function handleLine(line)
  local ok, msg = pcall(noctalia.json.decode, line)
  if not ok or type(msg) ~= "table" then return end
  local kind = msg.type
  if kind == "hook" and type(msg.payload) == "table" then
    view.daemon = true
    onHook(msg.payload)
  elseif kind == "resolved" then
    onResolved(msg)
  elseif kind == "hello" then
    onHello(msg)
  elseif kind == "ping" then
    if not view.daemon then
      view.daemon = true
      publish()
    end
  elseif kind == "daemon" then
    local was = view.daemon
    view.daemon = msg.running == true
    if not view.daemon and view.pending then view.pending = nil end
    if was ~= view.daemon then recompute() end
  end
end

-- ── The stream ───────────────────────────────────────────────────────────────

local generation = 0
local lastLine = now()

local function startStream()
  generation += 1
  local mine = generation
  lastLine = now()
  local ok = noctalia.runStream(DAEMON .. " --follow", function(line)
    -- Lines from a stream we already replaced are ignored.
    if mine ~= generation then return end
    lastLine = now()
    handleLine(line)
  end)
  if not ok then noctalia.log("mochi: could not start " .. DAEMON .. " --follow") end
end

-- ── Chat ─────────────────────────────────────────────────────────────────────

local chatGeneration = 0
local lastChatSeq = nil

local function publishChat()
  chat.rev += 1
  noctalia.state.set(CHAT_KEY, {
    messages = chat.messages,
    live = chat.live,
    busy = chat.busy,
    error = chat.error,
    file = chat.file,
    rev = chat.rev,
  })
end

local function shellQuote(s)
  return "'" .. s:gsub("'", "'\\''") .. "'"
end

local function runtimeDir()
  local dir = noctalia.getenv("XDG_RUNTIME_DIR")
  if type(dir) == "string" and dir:sub(1, 1) == "/" and not dir:find("'", 1, true) then return dir end
  return nil
end

local function endTurn()
  chat.busy = false
  chat.pid = nil
  chat.live = ""
  chatGeneration += 1 -- later lines of that turn are ignored
end

local function onChatLine(line)
  local ok, msg = pcall(noctalia.json.decode, line)
  if not ok or type(msg) ~= "table" then return end
  chat.lastLine = now()
  if msg.type == "start" then
    if type(msg.pid) == "number" then chat.pid = msg.pid end
    return
  elseif msg.type == "delta" and type(msg.text) == "string" then
    chat.live = msg.text
  elseif msg.type == "done" and type(msg.text) == "string" then
    table.insert(chat.messages, { role = "assistant", content = msg.text })
    endTurn()
    play("finish")
    recompute()
  elseif msg.type == "error" then
    -- A failed question stays on screen but is not sent again as history.
    local last = chat.messages[#chat.messages]
    if last and last.role == "user" then last.failed = true end
    chat.error = if type(msg.message) == "string" then msg.message else "Error"
    endTurn()
    play("error")
    recompute()
  else
    return
  end
  publishChat()
end

local function chatSend(c)
  local question = if type(c.question) == "string" then c.question:match("^%s*(.-)%s*$") else ""
  if chat.busy or question == "" then return end
  local dir = runtimeDir()
  if not dir then
    chat.error = "XDG_RUNTIME_DIR is not set."
    publishChat()
    return
  end
  local history = {}
  for _, m in chat.messages do
    if not m.failed then table.insert(history, { role = m.role, content = m.content }) end
  end
  local turn = {
    provider = tostring(c.provider or ""),
    model = tostring(c.model or ""),
    base_url = if type(c.base_url) == "string" then c.base_url else "",
    history = history,
    question = question,
  }
  -- The dropped file rides with the first question only (chat.ts).
  if chat.file and #history == 0 then turn.file = chat.file end
  local body = noctalia.json.encode(turn)
  local path = dir .. "/coucou-chat-" .. math.floor(now()) .. "-" .. math.random(1, 1e9) .. ".json"
  local written = body and noctalia.writeFile(path, body)
  if not written then
    chat.error = "Could not hand the question to coucou-daemon."
    publishChat()
    return
  end
  table.insert(chat.messages, { role = "user", content = question })
  chat.error = ""
  chat.live = ""
  chat.busy = true
  chat.pid = nil
  chat.lastLine = now()
  chatGeneration += 1
  local mine = chatGeneration
  play("send")
  recompute()
  publishChat()
  local started = noctalia.runStream(DAEMON .. " chat --request " .. shellQuote(path), function(line)
    if mine == chatGeneration then onChatLine(line) end
  end)
  if not started then
    chat.messages[#chat.messages].failed = true
    chat.error = "coucou-daemon could not be started."
    endTurn()
    recompute()
    publishChat()
  end
end

-- Stop: the daemon's process goes, and curl with it. What came so far stays.
local function chatStop()
  if not chat.busy then return end
  if chat.pid then noctalia.runAsync({ "kill", "-TERM", tostring(math.floor(chat.pid)) }, function() end, 2000) end
  local partial = chat.live
  endTurn()
  if partial ~= "" then table.insert(chat.messages, { role = "assistant", content = partial }) end
  recompute()
  publishChat()
end

local function chatReset()
  chatStop()
  chat.messages = {}
  chat.error = ""
  chat.file = nil
  publishChat()
end

-- A file for the chat: a new conversation about it, and Mochi swallows it.
local function chatAttach(c)
  if type(c.path) ~= "string" or c.path:sub(1, 1) ~= "/" or type(c.name) ~= "string" then return end
  chatReset()
  chat.file = { name = c.name, path = c.path }
  view.reaction = { name = "gulp", rev = ((view.reaction and view.reaction.rev) or 0) + 1 }
  play("approve")
  publishChat()
  publish()
end

function onChatCommand(c)
  if type(c) ~= "table" or c.seq == nil or c.seq == lastChatSeq then return end
  lastChatSeq = c.seq
  if c.op == "send" then
    chatSend(c)
  elseif c.op == "stop" then
    chatStop()
  elseif c.op == "reset" then
    chatReset()
  elseif c.op == "attach" then
    chatAttach(c)
  elseif c.op == "detach" then
    chat.file = nil
    publishChat()
  end
end

noctalia.state.watch(CHAT_CMD, onChatCommand)

-- ── The coucou-mochi overlay ─────────────────────────────────────────────────

local OVERLAY = "coucou-mochi"
local OVERLAY_FILE = "coucou-mochi.json"
local OVERLAY_HEARTBEAT_MS = 20 * 1000
local OVERLAY_CHECK_MS = 30 * 1000

local function surface()
  -- Sent off the desktop with the shortcut: back in the bar only, as the
  -- original's Mochi went back into the island.
  if prefs.away then return "desktop_widget" end
  local s = noctalia.getConfig("mochi_surface")
  if s == "desktop_widget" or s == "both" then return s end
  return "overlay"
end

local function overlaySettings()
  local size = tonumber(noctalia.getConfig("overlay_size")) or 120
  local layer = noctalia.getConfig("overlay_layer")
  return {
    sound_enabled = soundOn(),
    mochi_surface = surface(),
    overlay_size = math.clamp(math.floor(size), 64, 256),
    overlay_layer = if layer == "overlay" then "overlay" else "top",
    overlay_follow_cursor = noctalia.getConfig("overlay_follow_cursor") ~= false,
    overlay_peek = noctalia.getConfig("overlay_peek") == true,
    overlay_open_on_hover = noctalia.getConfig("overlay_open_on_hover") == true,
    overlay_show_text = noctalia.getConfig("overlay_show_text") ~= false,
  }
end

-- What the overlay writes under Mochi, as the desktop widget does: the
-- greeting at launch, then the tool while the agent is busy with it.
local TOOL_STATES = { working = true, searching = true, approval = true, question = true }
local firstName = nil

-- The tool while the agent is busy with it; else what is wrong or waiting
-- ("Paused", "Daemon offline", "Approval needed"…). Idle and sleeping say nothing.
local QUIET = { idle = true, sleeping = true, thinking = true }
local function overlayLabel()
  if noctalia.getConfig("overlay_show_text") == false then return "" end
  if view.paused then return noctalia.tr("state.paused") end
  if not view.daemon then return noctalia.tr("state.offline") end
  if TOOL_STATES[view.state] and type(view.tool) == "string" and view.tool ~= "" then return view.tool end
  if QUIET[view.state] then return "" end
  return noctalia.tr("state." .. view.state)
end

local function overlayGreeting()
  if noctalia.getConfig("overlay_show_text") == false or not firstName then return "" end
  return noctalia.tr("desktop.greeting", { name = firstName })
end

local overlayText = nil
local overlayWrittenAt = -math.huge

function publishOverlay()
  local dir = runtimeDir()
  if not dir or type(noctalia.writeFile) ~= "function" then return end
  local choice = outfitChoice()
  local body = {
    state = view.state,
    paused = view.paused,
    daemon = view.daemon,
    music = view.music,
    reaction = view.reaction,
    outfit = view.preview or wardrobe.resolveOutfit(choice),
    outfit_choice = choice,
    label = overlayLabel(),
    greeting = overlayGreeting(),
    settings = overlaySettings(),
  }
  local text = noctalia.json.encode(body)
  local t = now()
  if text == overlayText and t - overlayWrittenAt < OVERLAY_HEARTBEAT_MS then return end
  overlayText = text
  overlayWrittenAt = t
  body.ts = t
  local out = noctalia.json.encode(body)
  if out then noctalia.writeFile(dir .. "/" .. OVERLAY_FILE, out) end
end

local overlayCheckAt = 0
local overlayQuitSent = false

local function startOverlay()
  local dir = noctalia.pluginDir
  if type(dir) == "function" then dir = dir() end
  if type(dir) ~= "string" or dir == "" or dir:find("'", 1, true) then return end
  -- exec: the overlay's parent is the shell itself, so --managed ends with it.
  local ok = noctalia.runStream("exec " .. OVERLAY .. " --managed --plugin-dir " .. shellQuote(dir), function(line)
    noctalia.log("mochi overlay: " .. tostring(line))
  end)
  if not ok then noctalia.log("mochi: could not start " .. OVERLAY) end
end

local function ensureOverlay()
  if not (noctalia.commandExists and noctalia.commandExists(OVERLAY)) then return end
  if surface() == "desktop_widget" then
    -- It also quits by itself on reading the setting; this covers an old one.
    if not overlayQuitSent then
      overlayQuitSent = true
      noctalia.runAsync({ OVERLAY, "--quit" }, function() end, 3000)
    end
    overlayCheckAt = 0
    return
  end
  overlayQuitSent = false
  local t = now()
  if t < overlayCheckAt then return end
  overlayCheckAt = t + OVERLAY_CHECK_MS
  -- --status: 0 while an instance holds the session lock.
  noctalia.runAsync({ OVERLAY, "--status" }, function(res)
    if res.exitCode ~= 0 and surface() ~= "desktop_widget" then startOverlay() end
  end, 3000)
end

-- ── IPC ──────────────────────────────────────────────────────────────────────

local function setPaused(paused)
  if view.paused == paused then return end
  view.paused = paused
  if paused and view.pending then
    daemon({ "decline", view.pending.id })
    view.pending = nil
  end
  if not paused then play("peek") end
  recompute()
end

-- ── Sessions, shortcuts, wardrobe ─────────────────────────────────────────────

-- The session a shortcut or a card acts on: the waiting card's, the finished
-- card's, else the one Mochi shows.
local function targetSession(id)
  if type(id) == "string" and sessions[id] then return sessions[id] end
  if view.pending and sessions[view.pending.session] then return sessions[view.pending.session] end
  if view.finished and sessions[view.finished.session] then return sessions[view.finished.session] end
  if focus and sessions[focus] then return sessions[focus] end
  return nil
end

-- "Open terminal": the daemon walks to the session's window (argv, no shell).
local function openTerminal(id)
  local s = targetSession(id)
  if not s then return end
  local sid = if s.id:sub(1, 1) ~= "-" then s.id else "none"
  local argv = { DAEMON, "open-terminal", sid }
  if type(s.cwd) == "string" and s.cwd:sub(1, 1) == "/" then
    table.insert(argv, "--cwd")
    table.insert(argv, s.cwd)
  end
  noctalia.runAsync(argv, function(res)
    if res.exitCode ~= 0 then noctalia.log("mochi: no terminal found for session " .. s.id) end
  end, 5000)
end

-- Next / previous pill: Mochi shows that session (most recent first).
local function cycle(step)
  local list = sessionList()
  if #list == 0 then return end
  local at = 0
  for i, s in list do
    if s.id == focus then at = i end
  end
  local nextAt = ((at - 1 + step) % #list) + 1
  if at == 0 then nextAt = if step > 0 then 1 else #list end
  focus = list[nextAt].id
  play("blip")
  recompute()
end

local function setPreview(outfit)
  local known = false
  for _, o in wardrobe.OUTFITS do
    if o == outfit and o ~= "auto" then known = true end
  end
  local value = if known then outfit else nil
  if value == view.preview then return end
  view.preview = value
  publish()
end

-- A click in the wardrobe keeps the outfit: pop, and he is proud of it (the
-- widgets and the overlay see the new choice and play it).
local function keepOutfit(selection)
  local known = false
  for _, o in wardrobe.OUTFITS do
    if o == selection then known = true end
  end
  if not known or selection == outfitChoice() then return end
  prefs.outfit = { choice = selection, setting = outfitSetting() }
  savePrefs()
  publish()
end

local function togglePanel(id)
  noctalia.runAsync({ "noctalia", "msg", "panel-toggle", id }, nil)
end

function onIpc(event, payload)
  if event == "toggle-pause" then
    setPaused(not view.paused)
  elseif event == "pause" then
    setPaused(true)
  elseif event == "resume" then
    setPaused(false)
  elseif event == "open" then
    openPanel(view.pending and view.pending.id or nil)
  elseif event == "agents" then
    openPanel(nil, AGENTS_PANEL)
  elseif event == "chat" then
    openPanel(nil, CHAT_PANEL)
  elseif event == "island" then
    togglePanel(PANEL)
  elseif event == "alert" then
    if view.pending then openPanel(view.pending.id) end
  elseif event == "terminal" then
    openTerminal(payload)
  elseif event == "focus" then
    if type(payload) == "string" and sessions[payload] then
      focus = payload
      recompute()
    end
  elseif event == "next" then
    cycle(1)
  elseif event == "prev" then
    cycle(-1)
  elseif event == "dismiss" then
    if view.finished then
      view.finished = nil
      publish()
    end
  elseif event == "mute" then
    prefs.muted = not prefs.muted
    savePrefs()
    publish()
  elseif event == "desktop" then
    prefs.away = not prefs.away
    savePrefs()
    if not prefs.away then play("peek") end
    publish()
    ensureOverlay()
  elseif event == "wardrobe" then
    togglePanel(WARDROBE_PANEL)
  elseif event == "menu" then
    openPanel(nil, MENU_PANEL)
  elseif event == "outfit-preview" then
    setPreview(payload)
  elseif event == "outfit-keep" then
    keepOutfit(payload)
  elseif event == "attach" then
    -- A file dropped on the overlay: a chat about it, and Mochi swallows it.
    if type(payload) == "string" and payload:sub(1, 1) == "/" and not payload:find("\n", 1, true) then
      chatAttach({ path = payload, name = basename(payload) })
      openPanel(nil, CHAT_PANEL)
    end
  end
end

-- UI entries can also ask through shared state: "event:seq", or
-- { event, payload, seq } (a new seq each time, so a repeat still arrives).
noctalia.state.watch("mochi.cmd", function(cmd)
  if type(cmd) == "string" and cmd ~= "" then
    onIpc(cmd:match("^[^:]+"), nil)
  elseif type(cmd) == "table" and type(cmd.event) == "string" then
    onIpc(cmd.event, if type(cmd.payload) == "string" then cmd.payload else nil)
  end
end)

-- ── Music: MPRIS through playerctl, when installed ──────────────────────────

local function setMusic(line)
  local on = type(line) == "string" and line:match("^%s*(.-)%s*$") == "Playing"
  if on == view.music then return end
  view.music = on
  publish()
end

local function startMusic()
  -- No playerctl: no dance, nothing else changes.
  if not (noctalia.commandExists and noctalia.commandExists("playerctl")) then return end
  noctalia.runStream("playerctl -F -f '{{status}}' status 2>/dev/null", setMusic)
end

-- ── Ticks: finished -> idle, stream watchdog ────────────────────────────────

function update()
  local t = now()
  local changed = false
  for id, s in sessions do
    if s.state == "finished" and s.finishedAt and t - s.finishedAt > FINISHED_HOLD_MS then
      s.state = "idle"
      s.finishedAt = nil
      s.badge = nil
      changed = true
    end
    if t - s.updated > SESSION_FORGET_MS and s.state ~= "approval" and s.state ~= "question" then
      sessions[id] = nil
      if focus == id then focus = nil end
      changed = true
    end
  end
  -- The finished card stays as long as the original island's auto-close (30 s max).
  if view.finished and t - view.finished.at > FINISHED_CARD_MS then
    view.finished = nil
    changed = true
  end
  -- The outfit setting changed (a kept wardrobe outfit gives way to it).
  local choice = outfitChoice()
  if choice ~= shownChoice then
    shownChoice = choice
    changed = true
  end
  if changed then recompute() end
  publishOverlay()
  ensureOverlay()
  if chat.busy and t - chat.lastLine > CHAT_STALE_MS then
    onChatLine('{"type":"error","message":"No answer from coucou-daemon."}')
  end
  if t - lastLine > STREAM_STALE_MS then
    noctalia.log("mochi: daemon stream silent, restarting it")
    startStream()
  end
end

noctalia.setUpdateInterval(1000)
publish()
publishChat()
startStream()
startMusic()
identity.read(noctalia, function(name)
  firstName = name
  publishOverlay()
end)
