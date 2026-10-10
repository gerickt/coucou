--!nonstrict
-- The Agents panel, the [[panel]] "agents" entry: Settings → Agents of the
-- original app.
--
-- Lists every agent `coucou-daemon install-hooks --status --json` knows, with
-- whether it is on this machine (detected) and whether its hooks reach Coucou
-- (connected). Connect / Disconnect first shows the exact change
-- (`--dry-run --json`: the diff and where the backup goes); only a click on
-- the confirm button of that preview writes, with
-- `coucou-daemon install-hooks --agent <id> --yes [--remove]` (argv, no shell).
-- The daemon takes the dated backup and writes atomically.

local DAEMON = "coucou-daemon"
local TIMEOUT_MS = 10000

local agents = nil -- the status list, nil until loaded
local relayOk = true
local loadError = nil
local preview = nil -- { id, name, remove, diff, backup, changed, error }
local notice = nil -- { ok, text } after a write
local busy = false

local function tr(key, args) return noctalia.tr(key, args) end

local function safeId(id)
  return type(id) == "string" and id:match("^[%w%-]+$") ~= nil
end

local function findAgent(id)
  for _, a in agents or {} do
    if a.id == id then return a end
  end
  return nil
end

local function decode(text)
  if type(text) ~= "string" or text == "" then return nil end
  local ok, value = pcall(noctalia.json.decode, text)
  if ok and type(value) == "table" then return value end
  return nil
end

local function firstLine(text)
  if type(text) ~= "string" then return "" end
  return (text:match("[^\n]+") or ""):gsub("^coucou%-daemon: ", "")
end

local render

-- ── Daemon calls ─────────────────────────────────────────────────────────────

local function hooks(args, onDone)
  local argv = { DAEMON, "install-hooks" }
  for _, a in args do table.insert(argv, a) end
  local started = noctalia.runAsync(argv, onDone, TIMEOUT_MS)
  if not started then onDone({ exitCode = -1, stdout = "", stderr = tr("agents.daemon_missing") }) end
end

local function load()
  busy = true
  hooks({ "--status", "--json" }, function(res)
    busy = false
    local v = if res.exitCode == 0 then decode(res.stdout) else nil
    if v and type(v.agents) == "table" then
      agents = v.agents
      relayOk = v.relay_ok ~= false
      loadError = nil
    else
      agents = agents or {}
      loadError = firstLine(res.stderr) ~= "" and firstLine(res.stderr) or tr("agents.load_failed")
    end
    render()
  end)
end

-- ── Handlers ─────────────────────────────────────────────────────────────────

local function showPreview(id, remove)
  local a = findAgent(id)
  if busy or not a or not safeId(id) then return end
  busy = true
  notice = nil
  local args = { "--dry-run", "--json", "--agent", id }
  if remove then table.insert(args, "--remove") end
  hooks(args, function(res)
    busy = false
    local v = if res.exitCode == 0 then decode(res.stdout) else nil
    local entry = v and type(v.agents) == "table" and v.agents[1] or nil
    preview = { id = id, name = a.name or id, remove = remove, note = a.note }
    if not entry then
      preview.error = firstLine(res.stderr) ~= "" and firstLine(res.stderr) or tr("agents.preview_failed")
    elseif type(entry.error) == "string" then
      preview.error = entry.error
    else
      preview.changed = entry.changed == true
      preview.diff = if type(entry.diff) == "string" then entry.diff else ""
      preview.backup = if type(entry.backup_path) == "string" then entry.backup_path else nil
    end
    render()
  end)
end

-- Only ever reached from the confirm button of a preview that has a change.
function onConfirm()
  local p = preview
  if busy or not p or p.error or not p.changed or not safeId(p.id) then return end
  busy = true
  render()
  local args = { "--agent", p.id, "--yes" }
  if p.remove then table.insert(args, "--remove") end
  hooks(args, function(res)
    busy = false
    preview = nil
    if res.exitCode == 0 then
      local text = tr(if p.remove then "agents.removed" else "agents.connected_done", { name = p.name })
      if not p.remove and type(p.note) == "string" and p.note ~= "" then text = text .. " " .. p.note end
      notice = { ok = true, text = text }
    else
      notice = { ok = false, text = tr("agents.write_failed", { error = firstLine(res.stderr) }) }
    end
    load()
  end)
end

function onCancel()
  preview = nil
  render()
end

function onRefresh()
  if busy then return end
  notice = nil
  load()
end

-- ── Views ────────────────────────────────────────────────────────────────────

local function chip(text, on)
  return ui.row({ paddingH = 8, paddingV = 2, radius = 8, fill = if on then "primary" else "surface_variant" }, {
    ui.label({ text = text, fontSize = 11, fontWeight = "semibold", color = if on then "on_primary" else "on_surface_variant" }),
  })
end

local function header(title, subtitle)
  return ui.column({ gap = 2 }, {
    ui.label({ text = title, fontSize = 16, fontWeight = "bold", color = "primary", maxLines = 1, maxWidth = 460 }),
    ui.label({ text = subtitle, fontSize = 12, color = "on_surface_variant", maxLines = 3, maxWidth = 460 }),
  })
end

local function agentRow(a)
  local id = a.id
  local buttons = {
    ui.button({
      key = "connect-" .. id,
      text = if a.installed then tr("agents.reconnect") else tr("agents.connect"),
      variant = if a.installed then "outline" else "primary",
      controlSize = "sm",
      enabled = not busy and relayOk,
      onClick = function() showPreview(id, false) end,
    }),
  }
  if a.installed then
    table.insert(buttons, ui.button({
      key = "disconnect-" .. id,
      text = tr("agents.disconnect"),
      variant = "destructive",
      controlSize = "sm",
      enabled = not busy,
      onClick = function() showPreview(id, true) end,
    }))
  end
  local state = if a.connected then tr("agents.connected")
    elseif a.installed then tr("agents.relay_missing")
    else tr("agents.not_connected")
  return ui.column({ gap = 4, padding = 10, radius = 10, fill = "surface_variant" }, {
    ui.row({ gap = 6, align = "center" }, {
      ui.label({ text = a.name or id, fontWeight = "semibold", maxWidth = 170, maxLines = 1 }),
      chip(if a.detected then tr("agents.detected") else tr("agents.not_found"), a.detected == true),
      chip(state, a.connected == true),
      ui.spacer({ flexGrow = 1 }),
      ui.row({ gap = 6 }, buttons),
    }),
    ui.label({
      text = if a.approvals then tr("agents.approvals") else tr("agents.display_only"),
      fontSize = 11, color = "on_surface_variant", maxWidth = 460, maxLines = 1,
    }),
  })
end

local function listView()
  local rows = {}
  if notice then
    table.insert(rows, ui.label({ text = notice.text, fontSize = 12, color = if notice.ok then "primary" else "error", maxWidth = 460, maxLines = 4 }))
  end
  if loadError then
    table.insert(rows, ui.label({ text = loadError, fontSize = 12, color = "error", maxWidth = 460, maxLines = 3 }))
  end
  if not relayOk then
    table.insert(rows, ui.label({ text = tr("agents.no_relay"), fontSize = 12, color = "error", maxWidth = 460, maxLines = 3 }))
  end
  if agents == nil then
    table.insert(rows, ui.label({ text = tr("agents.loading"), fontSize = 12, color = "on_surface_variant" }))
  else
    for _, a in agents do
      if type(a) == "table" and safeId(a.id) then table.insert(rows, agentRow(a)) end
    end
  end
  return ui.column({ padding = 16, gap = 12, flexGrow = 1 }, {
    ui.row({ gap = 8, align = "center" }, {
      header(tr("agents.title"), tr("agents.intro")),
      ui.spacer({ flexGrow = 1 }),
      ui.button({ glyph = "refresh", variant = "ghost", controlSize = "sm", enabled = not busy, onClick = "onRefresh", tooltip = tr("agents.refresh") }),
    }),
    ui.scroll({ flexGrow = 1, gap = 8 }, rows),
  })
end

-- A diff as a fenced block; a fence inside it would end the block early.
local function diffMarkdown(diff)
  return "```diff\n" .. diff:gsub("```", "` ` `") .. "\n```"
end

local function previewView(p)
  local title = tr(if p.remove then "agents.preview_remove" else "agents.preview_connect", { name = p.name })
  local body = {}
  local confirm = nil
  if p.error then
    table.insert(body, ui.label({ text = p.error, fontSize = 12, color = "error", maxWidth = 460, maxLines = 6 }))
  elseif not p.changed then
    table.insert(body, ui.label({ text = tr("agents.up_to_date"), fontSize = 12, color = "on_surface_variant" }))
  else
    table.insert(body, ui.scroll({ flexGrow = 1, fill = "surface_variant", radius = 10, padding = 10 }, {
      ui.markdown({ text = diffMarkdown(p.diff) }),
    }))
    table.insert(body, ui.label({
      text = if p.backup then tr("agents.backup", { path = p.backup }) else tr("agents.no_backup"),
      fontSize = 11, color = "on_surface_variant", maxWidth = 460, maxLines = 3,
    }))
    confirm = ui.button({
      text = tr(if p.remove then "agents.confirm_remove" else "agents.confirm_connect"),
      variant = if p.remove then "destructive" else "primary",
      flexGrow = 1, onClick = "onConfirm", enabled = not busy,
    })
  end
  local buttons = { ui.button({ text = tr("agents.cancel"), variant = "ghost", flexGrow = 1, onClick = "onCancel", enabled = not busy }) }
  if confirm then table.insert(buttons, confirm) end
  return ui.column({ padding = 16, gap = 12, flexGrow = 1 }, {
    header(title, tr(if p.remove then "agents.preview_remove_hint" else "agents.preview_connect_hint")),
    ui.column({ gap = 8, flexGrow = 1 }, body),
    ui.row({ gap = 8 }, buttons),
  })
end

render = function()
  panel.render(if preview then previewView(preview) else listView())
end

function onOpen(_context)
  preview = nil
  notice = nil
  busy = false
  render()
  load()
end
