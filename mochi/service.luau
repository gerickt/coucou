--!nonstrict
-- Mochi's brain, the headless [[service]] entry.
--
-- Follows `coucou-daemon --follow` (one JSON line per hook event), runs the
-- state machine of the original app (windows/src/island/hooks.ts), plays the
-- state sounds, and publishes one snapshot to noctalia.state("mochi") that the
-- desktop widget, the bar widget and the panel all render.
--
-- Permission requests: the card is acknowledged at once (the daemon only waits
-- for a human once acknowledged, within 800 ms) and the approval panel opens.
-- Nothing is ever approved here: decisions come only from a click in the panel.
--
-- IPC: noctalia msg plugin coucou/mochi:daemon all toggle-pause|pause|resume|open

local KEY = "mochi"
local PANEL = "coucou/mochi:approval"
local DAEMON = "coucou-daemon"

-- Agents whose permission requests can be answered (coucou-hook reply.rs).
local APPROVAL_AGENTS = { [""] = true, ["claude-desktop"] = true, codex = true, copilot = true, muse = true }
-- Only Claude Code asks AskUserQuestion through a permission request.
local QUESTION_AGENTS = { [""] = true, ["claude-desktop"] = true }
local SEARCH_TOOLS = {
  Grep = true, Glob = true, LS = true, WebSearch = true, WebFetch = true, ToolSearch = true,
  grep = true, glob = true, search = true, google_web_search = true, web_fetch = true,
}
-- STATE_SOUND from the original engine.
local STATE_SOUND = {
  working = "work", thinking = "think", searching = "search", approval = "approval",
  question = "question", error = "error", finished = "finish", ratelimit = "rate",
  sleeping = "sleep",
}
local SOUNDS = { "work", "think", "search", "approval", "question", "error", "finish", "rate", "sleep", "approve", "close" }

local FINISHED_HOLD_MS = 5200
local SLEEP_AFTER_MS = 5 * 60 * 1000
local SESSION_FORGET_MS = 60 * 60 * 1000
local STREAM_STALE_MS = 45 * 1000
local MAX_SESSIONS = 8

local function now() return noctalia.nowMs() end

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
}
local sessions = {} -- session id -> { id, agent, project, state, tool, updated, finishedAt }
local focus = nil -- the session Mochi is showing

-- ── Sounds ───────────────────────────────────────────────────────────────────

local loaded = {}
for _, name in SOUNDS do
  noctalia.sound.load(name, "sounds/" .. name .. ".wav", function(ok, err)
    loaded[name] = ok
    if not ok then noctalia.log("mochi: sound " .. name .. " failed: " .. tostring(err)) end
  end)
end

local function play(name)
  if name and loaded[name] and noctalia.getConfig("sound_enabled") ~= false then
    noctalia.sound.play(name)
  end
end

-- ── Publishing ───────────────────────────────────────────────────────────────

local function sessionList()
  local list = {}
  for _, s in sessions do
    table.insert(list, { id = s.id, agent = s.agent, project = s.project, state = s.state, tool = s.tool, updated = s.updated })
  end
  table.sort(list, function(a, b) return a.updated > b.updated end)
  while #list > MAX_SESSIONS do table.remove(list) end
  return list
end

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
    pending = view.pending,
    sessions = view.sessions,
    rev = view.rev,
  })
end

-- Mochi shows the pending card first, else the session that spoke last.
local function recompute(quiet)
  local before = view.state
  if view.pending then
    view.state = view.pending.kind
    view.tool = view.pending.tool or ""
    view.agent = view.pending.agent or ""
    view.project = view.pending.project or ""
  elseif focus and sessions[focus] then
    local s = sessions[focus]
    view.state = s.state
    view.tool = s.tool
    view.agent = s.agent
    view.project = s.project
  else
    view.state = if before == "sleeping" then "sleeping" else "idle"
    view.tool = ""
  end
  if view.state ~= before then
    view.stateSince = now()
    if not quiet then play(STATE_SOUND[view.state]) end
  end
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

local function openPanel(context)
  local argv = { "noctalia", "msg", "panel-open", PANEL }
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
    s = { id = id, agent = "", project = "", state = "idle", tool = "", updated = now() }
    sessions[id] = s
  end
  s.agent = p.coucou_agent or ""
  local project = basename(p.cwd)
  if project ~= "" then s.project = project end
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
  openPanel(id)
  return true
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
  elseif name == "UserPromptSubmit" then
    s.state = "thinking"
    s.tool = ""
  elseif name == "PreToolUse" then
    s.tool = p.tool_name or ""
    s.state = if SEARCH_TOOLS[s.tool] then "searching" else "working"
  elseif name == "PostToolUse" or name == "PostToolUseFailure" then
    s.state = "working"
  elseif name == "Notification" then
    local message = tostring(p.message or "")
    local lower = message:lower()
    if lower:find("rate limit", 1, true) then
      s.state = "ratelimit"
    elseif message:sub(-1) == "?" then
      s.state = "question"
    end
  elseif name == "Stop" then
    s.state = "finished"
    s.finishedAt = now()
  elseif name == "Interrupt" then
    s.state = "idle"
  elseif name == "StopFailure" then
    s.state = "error"
  elseif name == "SessionEnd" then
    sessions[s.id] = nil
    if focus == s.id then focus = nil end
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
  elseif outcome == "deny" then
    play("close")
  end
  recompute(true)
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
    if was ~= view.daemon then recompute(true) end
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

-- ── IPC ──────────────────────────────────────────────────────────────────────

local function setPaused(paused)
  if view.paused == paused then return end
  view.paused = paused
  if paused and view.pending then
    daemon({ "decline", view.pending.id })
    view.pending = nil
  end
  recompute(true)
end

function onIpc(event, _payload)
  if event == "toggle-pause" then
    setPaused(not view.paused)
  elseif event == "pause" then
    setPaused(true)
  elseif event == "resume" then
    setPaused(false)
  elseif event == "open" then
    openPanel(view.pending and view.pending.id or nil)
  end
end

-- UI entries can also ask through shared state.
noctalia.state.watch("mochi.cmd", function(cmd)
  if type(cmd) == "string" and cmd ~= "" then
    onIpc(cmd:match("^[^:]+"), nil)
  end
end)

-- ── Ticks: finished -> idle, idle -> sleeping, stream watchdog ──────────────

function update()
  local t = now()
  local changed = false
  for id, s in sessions do
    if s.state == "finished" and s.finishedAt and t - s.finishedAt > FINISHED_HOLD_MS then
      s.state = "idle"
      s.finishedAt = nil
      changed = true
    end
    if t - s.updated > SESSION_FORGET_MS and s.state ~= "approval" and s.state ~= "question" then
      sessions[id] = nil
      if focus == id then focus = nil end
      changed = true
    end
  end
  if changed then recompute(true) end
  if view.state == "idle" and not view.paused and t - view.stateSince > SLEEP_AFTER_MS then
    view.state = "sleeping"
    view.stateSince = t
    play(STATE_SOUND.sleeping)
    publish()
  end
  if t - lastLine > STREAM_STALE_MS then
    noctalia.log("mochi: daemon stream silent, restarting it")
    startStream()
  end
end

noctalia.setUpdateInterval(1000)
publish()
startStream()
