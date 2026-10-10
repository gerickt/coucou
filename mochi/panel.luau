--!nonstrict
-- The approval panel, the [[panel]] entry.
--
-- Shows the pending request from noctalia.state("mochi"): a permission card
-- with Allow / Deny, or Claude Code's AskUserQuestion with its options. The
-- decision goes to the daemon with `coucou-daemon decide <id> ...` (argv, no
-- shell) and the panel closes. With nothing pending: recent sessions.
--
-- This is the only place a decision is made, and only on a click.

local DAEMON = "coucou-daemon"
local AGENTS_PANEL = "coucou/mochi:agents"
local CHAT_PANEL = "coucou/mochi:chat"

local okPills, pills = pcall(require, "./pills.luau")
if not okPills then pills = require("./pills") end
local WARDROBE_PANEL = "coucou/mochi:wardrobe"

local snap = {}
local shownId = nil -- request id the panel is showing
local answers = {} -- question text -> label (single) or { label = true } (multi)
local sending = false

local function tr(key, args) return noctalia.tr(key, args) end

local function agentName(id)
  return pills.agentName(id)
end

-- Asks the service (it knows the sessions' folders): "terminal", "focus", "dismiss".
local cmdSeq = 0
local function ask(event, payload)
  cmdSeq += 1
  noctalia.state.set("mochi.cmd", { event = event, payload = payload, seq = cmdSeq })
end

local function safeId(id)
  return type(id) == "string" and id:match("^[%w%-]+$") ~= nil
end

local function send(args, closeAfter)
  if sending then return end
  sending = true
  local argv = { DAEMON }
  for _, a in args do table.insert(argv, a) end
  noctalia.runAsync(argv, function(res)
    sending = false
    if res.exitCode ~= 0 then
      noctalia.notifyError("Mochi", tr("panel.send_failed"))
    end
    if closeAfter then panel.close() end
  end, 5000)
end

-- ── Handlers ─────────────────────────────────────────────────────────────────

local render

local function decide(word)
  local p = snap.pending
  if not p or not safeId(p.id) then return end
  send({ "decide", p.id, word }, true)
end

function onAllow() decide("allow") end
function onDeny() decide("deny") end

-- Settings → Agents lives in its own panel.
function onAgents()
  panel.close()
  noctalia.runAsync({ "noctalia", "msg", "panel-open", AGENTS_PANEL }, nil)
end

-- The chat lives in its own panel too.
function onChat()
  panel.close()
  noctalia.runAsync({ "noctalia", "msg", "panel-open", CHAT_PANEL }, nil)
end

function onOpenTerminal()
  local p = snap.pending
  local f = snap.finished
  local id = if p then p.session elseif f then f.session else nil
  ask("terminal", id)
end

function onFinishedOk()
  ask("dismiss")
  panel.close()
end

function onFinishedTerminal()
  local f = snap.finished
  ask("terminal", f and f.session or nil)
  ask("dismiss")
  panel.close()
end

function onTerminal()
  local p = snap.pending
  if not p or not safeId(p.id) then return end
  send({ "decline", p.id }, true)
end

local function allAnswered(questions)
  for _, q in questions do
    local a = answers[q.question]
    if a == nil then return false end
    if q.multiSelect then
      local any = false
      for _ in a do any = true end
      if not any then return false end
    end
  end
  return true
end

function onSubmit()
  local p = snap.pending
  if not p or not safeId(p.id) or not p.questions or not allAnswered(p.questions) then return end
  local out = {}
  for _, q in p.questions do
    local a = answers[q.question]
    if q.multiSelect then
      -- In option order, so the list is stable.
      local list = {}
      for _, o in q.options do
        if a[o.label] then table.insert(list, o.label) end
      end
      out[q.question] = list
    else
      out[q.question] = a
    end
  end
  local encoded = noctalia.json.encode({ answers = out })
  if type(encoded) ~= "string" then return end
  send({ "decide", p.id, encoded }, true)
end

local function pick(q, label)
  if q.multiSelect then
    local set = answers[q.question] or {}
    set[label] = if set[label] then nil else true
    answers[q.question] = set
  else
    answers[q.question] = label
  end
  render()
end

-- ── Views ────────────────────────────────────────────────────────────────────

local function header(title, subtitle)
  return ui.column({ gap = 2 }, {
    ui.label({ text = title, fontSize = 16, fontWeight = "bold", color = "primary", maxLines = 2, maxWidth = 400 }),
    ui.label({ text = subtitle, fontSize = 12, color = "on_surface_variant", maxLines = 1, maxWidth = 400 }),
  })
end

local function where(p)
  local parts = { agentName(p.agent) }
  if type(p.project) == "string" and p.project ~= "" then table.insert(parts, p.project) end
  return table.concat(parts, " · ")
end

local function approvalView(p)
  local target = if type(p.target) == "string" and p.target ~= "" then p.target else p.tool
  return ui.column({ padding = 16, gap = 12, flexGrow = 1 }, {
    header(tr("panel.wants", { tool = p.tool or "Tool" }), where(p)),
    ui.scroll({ flexGrow = 1, fill = "surface_variant", radius = 10, padding = 10 }, {
      ui.label({ text = target, fontSize = 13, color = "on_surface", maxWidth = 380, maxLines = 40 }),
    }),
    ui.row({ gap = 8 }, {
      ui.button({ text = tr("panel.deny"), variant = "destructive", flexGrow = 1, onClick = "onDeny", enabled = not sending }),
      ui.button({ text = tr("panel.allow"), variant = "primary", flexGrow = 1, onClick = "onAllow", enabled = not sending }),
    }),
    ui.button({ text = tr("finished.open_terminal"), glyph = "terminal-2", variant = "ghost", onClick = "onOpenTerminal" }),
  })
end

local function questionView(p)
  local blocks = {}
  for qi, q in p.questions do
    local rows = {}
    if q.header ~= "" then
      table.insert(rows, ui.label({ text = q.header, fontSize = 11, fontWeight = "semibold", color = "primary" }))
    end
    table.insert(rows, ui.label({ text = q.question, fontSize = 14, fontWeight = "semibold", maxWidth = 380, maxLines = 6 }))
    if q.multiSelect then
      table.insert(rows, ui.label({ text = tr("panel.multi_hint"), fontSize = 11, color = "on_surface_variant" }))
    end
    local a = answers[q.question]
    for oi, o in q.options do
      local selected = if q.multiSelect then (a ~= nil and a[o.label] == true) else a == o.label
      table.insert(rows, ui.button({
        key = "q" .. qi .. "o" .. oi,
        text = o.label,
        variant = if selected then "primary" else "outline",
        selected = selected,
        contentAlign = "start",
        onClick = function() pick(q, o.label) end,
      }))
      if o.description ~= "" then
        table.insert(rows, ui.label({ text = o.description, fontSize = 11, color = "on_surface_variant", maxWidth = 380, maxLines = 3 }))
      end
    end
    table.insert(blocks, ui.column({ gap = 6 }, rows))
  end
  return ui.column({ padding = 16, gap = 12, flexGrow = 1 }, {
    header(tr("panel.asks", { agent = agentName(p.agent) }), where(p)),
    ui.scroll({ flexGrow = 1, gap = 14 }, blocks),
    ui.row({ gap = 8 }, {
      ui.button({ text = tr("panel.terminal"), variant = "ghost", flexGrow = 1, onClick = "onTerminal", enabled = not sending }),
      ui.button({ glyph = "terminal-2", variant = "ghost", onClick = "onOpenTerminal", tooltip = tr("finished.open_terminal") }),
      ui.button({ text = tr("panel.submit"), variant = "primary", flexGrow = 1, onClick = "onSubmit",
        enabled = not sending and allAnswered(p.questions) }),
    }),
  })
end

-- One session as the original's pill: a tint of its agent's colour, a badge.
local function pillRow(sess)
  local color = sess.color or pills.agentColor(sess.agent)
  local badge = sess.badge
  local title = if type(sess.project) == "string" and sess.project ~= "" then sess.project else agentName(sess.agent)
  local kids = {
    ui.box({ width = 10, height = 10, radius = 5, fill = color }),
    ui.column({ gap = 0, flexGrow = 1 }, {
      ui.label({ text = title, fontWeight = "semibold", maxWidth = 190, maxLines = 1 }),
      ui.label({ text = agentName(sess.agent) .. " · " .. tr("state." .. (sess.state or "idle")), fontSize = 11,
        color = "on_surface_variant", maxWidth = 190, maxLines = 1 }),
    }),
  }
  if badge and pills.BADGES[badge] then
    table.insert(kids, ui.box({ width = 12, height = 12, radius = 6, fill = pills.BADGES[badge] }))
  end
  local id = sess.id
  table.insert(kids, ui.button({
    key = "t" .. tostring(id), glyph = "terminal-2", variant = "ghost", controlSize = "sm",
    tooltip = tr("finished.open_terminal"), onClick = function() ask("terminal", id) end,
  }))
  return ui.row({
    key = "s" .. tostring(id), gap = 8, align = "center", padding = 8, radius = 10,
    fill = pills.tint(color, if sess.focused then "2e" else "14"), border = pills.tint(color, "24"), borderWidth = 1,
    onClick = function() ask("focus", id) end,
  }, kids)
end

-- The finished card (buildFinished): the agent, its final message, Open terminal / OK.
local function finishedView(f)
  local line = if type(f.line) == "string" and f.line ~= "" then f.line else tr("finished.none")
  return ui.column({ padding = 16, gap = 12, flexGrow = 1 }, {
    header(tr("finished.title", { agent = agentName(f.agent) }), where(f)),
    ui.scroll({ flexGrow = 1, fill = "surface_variant", radius = 10, padding = 10 }, {
      ui.label({ text = line, fontSize = 14, color = "on_surface", maxWidth = 380, maxLines = 40 }),
    }),
    ui.row({ gap = 8 }, {
      ui.button({ text = tr("finished.open_terminal"), variant = "primary", flexGrow = 1, onClick = "onFinishedTerminal" }),
      ui.button({ text = tr("finished.ok"), variant = "outline", flexGrow = 1, onClick = "onFinishedOk" }),
    }),
  })
end

local function emptyView()
  local status = if snap.paused then tr("state.paused")
    elseif not snap.daemon then tr("panel.offline")
    else tr("panel.empty")
  local rows = {}
  for _, s in (snap.sessions or {}) do
    table.insert(rows, pillRow(s))
  end
  if #rows == 0 then
    table.insert(rows, ui.label({ text = tr("panel.no_sessions"), fontSize = 12, color = "on_surface_variant" }))
  end
  return ui.column({ padding = 16, gap = 12, flexGrow = 1 }, {
    header("Mochi", status),
    ui.label({ text = tr("panel.recent"), fontSize = 12, fontWeight = "semibold", color = "primary" }),
    ui.scroll({ flexGrow = 1, gap = 6 }, rows),
    ui.row({ gap = 8 }, {
      ui.button({ text = tr("panel.chat"), glyph = "message-circle", variant = "outline", onClick = "onChat", flexGrow = 1 }),
      ui.button({ text = tr("panel.agents"), glyph = "plug-connected", variant = "outline", onClick = "onAgents", flexGrow = 1 }),
      ui.button({ glyph = "hanger", variant = "outline", onClick = "onWardrobe", tooltip = tr("panel.wardrobe") }),
    }),
  })
end

-- The wardrobe lives in its own panel too.
function onWardrobe()
  panel.close()
  noctalia.runAsync({ "noctalia", "msg", "panel-open", WARDROBE_PANEL }, nil)
end

render = function()
  local p = snap.pending
  if p and safeId(p.id) then
    if p.id ~= shownId then
      shownId = p.id
      answers = {}
    end
    panel.render(if p.questions then questionView(p) else approvalView(p))
  elseif snap.finished then
    shownId = nil
    panel.render(finishedView(snap.finished))
  else
    shownId = nil
    panel.render(emptyView())
  end
end

local function refresh(v)
  if type(v) == "table" then snap = v end
  render()
end

noctalia.state.watch("mochi", refresh)

function onOpen(_context)
  sending = false
  refresh(noctalia.state.get("mochi"))
end
