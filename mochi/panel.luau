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

local AGENTS = {
  [""] = "Claude Code", ["claude-desktop"] = "Claude", gemini = "Gemini CLI", antigravity = "Antigravity",
  codex = "Codex", cursor = "Cursor", copilot = "Copilot CLI", opencode = "OpenCode", muse = "Muse Code",
  amp = "Amp", hermes = "Hermes",
}

local snap = {}
local shownId = nil -- request id the panel is showing
local answers = {} -- question text -> label (single) or { label = true } (multi)
local sending = false

local function tr(key, args) return noctalia.tr(key, args) end

local function agentName(id)
  return AGENTS[id or ""] or tostring(id)
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
      ui.button({ text = tr("panel.submit"), variant = "primary", flexGrow = 1, onClick = "onSubmit",
        enabled = not sending and allAnswered(p.questions) }),
    }),
  })
end

local function emptyView()
  local status = if snap.paused then tr("state.paused")
    elseif not snap.daemon then tr("panel.offline")
    else tr("panel.empty")
  local rows = {}
  for _, s in (snap.sessions or {}) do
    table.insert(rows, ui.row({ gap = 8, align = "center" }, {
      ui.label({ text = if s.project ~= "" then s.project else agentName(s.agent), fontWeight = "semibold", maxWidth = 200, maxLines = 1 }),
      ui.spacer({ flexGrow = 1 }),
      ui.label({ text = tr("state." .. (s.state or "idle")), fontSize = 12, color = "on_surface_variant" }),
    }))
  end
  if #rows == 0 then
    table.insert(rows, ui.label({ text = tr("panel.no_sessions"), fontSize = 12, color = "on_surface_variant" }))
  end
  return ui.column({ padding = 16, gap = 12, flexGrow = 1 }, {
    header("Mochi", status),
    ui.label({ text = tr("panel.recent"), fontSize = 12, fontWeight = "semibold", color = "primary" }),
    ui.scroll({ flexGrow = 1, gap = 8 }, rows),
    ui.button({ text = tr("panel.agents"), glyph = "plug-connected", variant = "outline", onClick = "onAgents" }),
  })
end

render = function()
  local p = snap.pending
  if p and safeId(p.id) then
    if p.id ~= shownId then
      shownId = p.id
      answers = {}
    end
    panel.render(if p.questions then questionView(p) else approvalView(p))
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
