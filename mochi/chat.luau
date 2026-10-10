--!nonstrict
-- The chat panel, the [[panel]] "chat" entry: the original's chat view
-- (views/chat.ts) and its Settings sections (Claude API, Chat providers,
-- Local models).
--
-- The conversation itself runs in the service (service.luau): this panel shows
-- noctalia.state("mochi.chat") and asks through noctalia.state("mochi.chat.cmd").
-- The model name above the text field opens the picker: provider, then that
-- provider's models (`coucou-daemon chat-models`), asked for only once picked.
--
-- Keys: the panel only learns whether one is saved (`coucou-daemon secret has`).
-- Saving hands the typed key to `coucou-daemon secret set --from-file` through a
-- file in the private runtime directory, which the daemon removes as it reads
-- it; the key is never on a command line and never comes back.
--
-- Files: Noctalia's drop zones only take drags from inside the shell, so a file
-- comes from the clipboard instead (copy it in the file manager, then the clip
-- button): text inline up to 24 000 characters, images and PDF by name for the
-- model servers, as the original did.

local DAEMON = "coucou-daemon"
local STATE = "mochi.chat"
local CMD = "mochi.chat.cmd"

-- providers.ts PROVIDERS: who the chat can talk to.
local PROVIDERS = {
  { id = "anthropic", name = "Anthropic", key = true, default = "claude-opus-5", prefer = "opus" },
  { id = "google", name = "Google", key = true, default = "gemini-2.0-flash", prefer = "flash" },
  { id = "openai", name = "OpenAI", key = true, default = "gpt-4o", prefer = "mini" },
  { id = "openrouter", name = "OpenRouter", key = true, default = "openrouter/auto" },
  { id = "ollama", name = "Ollama", server = "http://127.0.0.1:11434", default = "" },
  { id = "lmstudio", name = "LM Studio", server = "http://127.0.0.1:1234", default = "" },
  { id = "custom", name = "Custom server", key = true, server = "", default = "" },
}

local snap = { messages = {}, live = "", busy = false, error = "" }
local prefs = { provider = "anthropic", models = {}, servers = {} }
local mode = "chat" -- "chat" | "settings"
local picking = false
local keys = {} -- provider id -> true / false (nil: not asked yet)
local models = {} -- provider id -> { { id, label } }
local modelStatus = {} -- provider id -> message while loading or on error
local modelTicket = 0
local draft = "" -- the question being typed
local inputRev = 0 -- a new key clears the field
local keyDrafts = {}
local serverDrafts = {}
local formRev = 0
local notice = nil -- { ok, text }
local firstName = nil
local cmdSeq = 0

local function tr(key, args) return noctalia.tr(key, args) end

local function providerDef(id)
  for _, p in PROVIDERS do
    if p.id == id then return p end
  end
  return PROVIDERS[1]
end

local function activeModel()
  local p = providerDef(prefs.provider)
  local m = prefs.models[p.id]
  if type(m) == "string" and m ~= "" then return m end
  return p.default
end

local function decode(text)
  if type(text) ~= "string" or text == "" then return nil end
  local ok, value = pcall(noctalia.json.decode, text)
  if ok then return value end
  return nil
end

-- ── Preferences (provider, models, server addresses; never a key) ────────────

local function prefsPath()
  local dir = noctalia.pluginDataDir and noctalia.pluginDataDir()
  if type(dir) ~= "string" then return nil end
  return dir .. "/chat.json"
end

local function loadPrefs()
  local path = prefsPath()
  local saved = path and decode((noctalia.readFile(path)))
  if type(saved) ~= "table" then return end
  if type(saved.provider) == "string" then prefs.provider = providerDef(saved.provider).id end
  if type(saved.models) == "table" then prefs.models = saved.models end
  if type(saved.servers) == "table" then prefs.servers = saved.servers end
end

local function savePrefs()
  local path = prefsPath()
  local text = noctalia.json.encode(prefs)
  if path and text then noctalia.writeFile(path, text) end
end

local function serverUrl(id)
  local url = prefs.servers[id]
  if type(url) == "string" then return url end
  return ""
end

-- ── Talking to the service and the daemon ────────────────────────────────────

local function command(t)
  cmdSeq += 1
  t.seq = tostring(noctalia.nowMs()) .. ":" .. cmdSeq
  noctalia.state.set(CMD, t)
end

local render

local function refreshKey(id)
  noctalia.runAsync({ DAEMON, "secret", "has", id }, function(res)
    local v = decode(res.stdout)
    keys[id] = res.exitCode == 0 and type(v) == "table" and v.present == true
    render()
  end, 5000)
end

local function refreshKeys()
  for _, p in PROVIDERS do
    if p.key then refreshKey(p.id) end
  end
end

-- pickModel(): the saved model if offered, else the preferred one, the default, the first.
function pickModel(p, offered, current)
  if #offered == 0 then return nil end
  for _, id in offered do
    if id == current then return current end
  end
  if p.prefer then
    for _, id in offered do
      if id:find(p.prefer, 1, true) then return id end
    end
  end
  for _, id in offered do
    if id == p.default then return id end
  end
  return offered[1]
end

local function loadModels()
  local p = providerDef(prefs.provider)
  modelTicket += 1
  local ticket = modelTicket
  -- Nothing is asked of a provider without a key.
  if p.key and p.id ~= "custom" and keys[p.id] == false then
    modelStatus[p.id] = tr("chat.no_key")
    render()
    return
  end
  -- A model server's list is asked again each time; a cloud one once.
  if models[p.id] and not p.server then return end
  modelStatus[p.id] = tr("chat.loading_models")
  local argv = { DAEMON, "chat-models", p.id }
  if p.server and serverUrl(p.id) ~= "" then
    table.insert(argv, "--base-url")
    table.insert(argv, serverUrl(p.id))
  end
  render()
  noctalia.runAsync(argv, function(res)
    if ticket ~= modelTicket then return end
    local v = decode(res.stdout)
    if type(v) == "table" and type(v.models) == "table" then
      models[p.id] = v.models
      modelStatus[p.id] = nil
      local ids = {}
      for _, m in v.models do table.insert(ids, m.id) end
      local keep = pickModel(p, ids, activeModel())
      if keep and keep ~= activeModel() then
        prefs.models[p.id] = keep
        savePrefs()
      end
    else
      modelStatus[p.id] = if type(v) == "table" and type(v.error) == "string" then v.error else tr("chat.models_failed")
    end
    render()
  end, 30000)
end

-- ── Handlers ─────────────────────────────────────────────────────────────────

function onSend(value)
  local text = if type(value) == "string" then value else draft
  text = text:match("^%s*(.-)%s*$")
  if text == "" or snap.busy then return end
  local p = providerDef(prefs.provider)
  command({
    op = "send",
    question = text,
    provider = p.id,
    model = activeModel(),
    base_url = if p.server then serverUrl(p.id) else "",
  })
  draft = ""
  inputRev += 1
  picking = false
  render()
end

function onDraft(value)
  draft = value or ""
end

function onStop() command({ op = "stop" }) end

function onNewChat()
  command({ op = "reset" })
  notice = nil
  render()
end

function onPicker()
  if snap.busy then return end
  picking = not picking
  if picking then loadModels() end
  render()
end

function onProvider(index)
  local p = PROVIDERS[tonumber(index) + 1]
  if not p or p.id == prefs.provider then return end
  prefs.provider = p.id
  savePrefs()
  if p.key then refreshKey(p.id) end
  loadModels()
  render()
end

function onModel(index)
  local list = models[prefs.provider]
  local m = list and list[tonumber(index) + 1]
  if not m then return end
  prefs.models[prefs.provider] = m.id
  savePrefs()
  picking = false
  render()
end

function onSettings()
  mode = if mode == "settings" then "chat" else "settings"
  notice = nil
  if mode == "settings" then refreshKeys() end
  render()
end

local function copy(text)
  if noctalia.copyToClipboard(text, "text/plain") then
    notice = { ok = true, text = tr("chat.copied") }
  end
  render()
end

-- The path in a copied file's URI list ("file:///home/me/a%20b.txt"), or a
-- pasted absolute path.
function clipboardPath(text)
  if type(text) ~= "string" then return nil end
  for line in text:gmatch("[^\r\n]+") do
    local uri = line:match("^file://[^/]*(/.*)$")
    if uri then return noctalia.string.urlDecode(uri) end
    if line:sub(1, 1) == "/" then return line end
  end
  return nil
end

local function attach(path)
  local info = path and noctalia.fileInfo(path)
  if not info or info.isDir then
    notice = { ok = false, text = tr("chat.attach_none") }
    render()
    return
  end
  local name = path:match("([^/]+)$") or path
  command({ op = "attach", name = name, path = path })
  notice = nil
  render()
end

function onAttach()
  noctalia.runAsync({ "wl-paste", "--no-newline", "--type", "text/uri-list" }, function(res)
    local path = if res.exitCode == 0 then clipboardPath(res.stdout) else nil
    attach(path or clipboardPath(noctalia.clipboardText()))
  end, 3000)
end

function onDetach() command({ op = "detach" }) end

local function runtimeFile(prefix)
  local dir = noctalia.getenv("XDG_RUNTIME_DIR")
  if type(dir) ~= "string" or dir:sub(1, 1) ~= "/" then return nil end
  return dir .. "/" .. prefix .. math.floor(noctalia.nowMs()) .. "-" .. math.random(1, 1e9)
end

local function saveKey(id)
  local value = (keyDrafts[id] or ""):match("^%s*(.-)%s*$")
  if value == "" then return end
  local argv = { DAEMON, "secret", "set", id }
  if id == "custom" then
    if serverUrl("custom") == "" then
      notice = { ok = false, text = tr("chat.custom_url_first") }
      render()
      return
    end
    table.insert(argv, "--bound-url")
    table.insert(argv, serverUrl("custom"))
  end
  local path = runtimeFile("coucou-key-")
  if not path or not noctalia.writeFile(path, value) then
    notice = { ok = false, text = tr("chat.key_failed") }
    render()
    return
  end
  table.insert(argv, "--from-file")
  table.insert(argv, path)
  keyDrafts[id] = nil
  formRev += 1
  noctalia.runAsync(argv, function(res)
    -- The daemon removes it as it reads it; this only covers a daemon that never ran.
    if noctalia.fileExists(path) then noctalia.removeFile(path) end
    notice = if res.exitCode == 0 then { ok = true, text = tr("chat.key_saved_notice") } else { ok = false, text = tr("chat.key_failed") }
    models[id] = nil
    refreshKey(id)
    render()
  end, 10000)
  render()
end

local function deleteKey(id)
  noctalia.runAsync({ DAEMON, "secret", "delete", id }, function()
    models[id] = nil
    refreshKey(id)
  end, 10000)
end

local function saveServer(id)
  local value = serverDrafts[id]
  if value == nil then return end
  prefs.servers[id] = value:match("^%s*(.-)%s*$")
  serverDrafts[id] = nil
  models[id] = nil
  savePrefs()
  notice = { ok = true, text = tr("chat.server_saved") }
  render()
end

-- ── First name (identity.rs, as the desktop widget reads it) ─────────────────

local function readFirstName()
  local user = noctalia.getenv("USER")
  if type(user) ~= "string" or not user:match("^[%w%._%-]+$") then return end
  noctalia.runAsync({ "getent", "passwd", user }, function(res)
    local gecos = res.exitCode == 0 and type(res.stdout) == "string"
      and res.stdout:match("^[^:\n]*:[^:\n]*:[^:\n]*:[^:\n]*:([^:\n]*)")
    if not gecos then return end
    local full = (gecos:match("^([^,]*)") or ""):match("^%s*(.-)%s*$")
    local handle = not full:find(" ", 1, true) and (full:lower() == full or full:find("[%d%._%-@]") ~= nil)
    local first = full:match("^([^ ]+)")
    if full ~= "" and not handle and (utf8.len(full) or 99) <= 32 and first and not first:find("[^%a\128-\255'%-]") then
      firstName = first
      render()
    end
  end, 2000)
end

-- ── Drawing ──────────────────────────────────────────────────────────────────

local function noticeRow()
  if not notice then return nil end
  return ui.label({ text = notice.text, fontSize = 12, color = if notice.ok then "primary" else "error" })
end

local function codeBlocks(text)
  local blocks = {}
  for body in text:gmatch("```[^\n]*\n(.-)```") do table.insert(blocks, body) end
  return blocks
end

local function bubble(m, i)
  if m.role == "user" then
    return ui.row({ key = "m" .. i, justify = "end" }, {
      ui.column({ fill = if m.failed then "error/0.12" else "primary/0.16", radius = 12, paddingH = 10, paddingV = 6 }, {
        ui.label({ text = m.content, fontSize = 13, maxWidth = 330, color = "on_surface" }),
      }),
    })
  end
  local actions = {
    ui.button({ text = tr("chat.copy"), glyph = "copy", variant = "ghost", controlSize = "sm", onClick = function() copy(m.content) end }),
  }
  for n, body in codeBlocks(m.content) do
    table.insert(actions, ui.button({
      text = tr("chat.copy_code", { n = n }), glyph = "code", variant = "ghost", controlSize = "sm",
      onClick = function() copy(body) end,
    }))
  end
  return ui.column({ key = "m" .. i, gap = 2 }, {
    ui.markdown({ text = m.content }),
    ui.row({ gap = 4 }, actions),
  })
end

local function header()
  local p = providerDef(prefs.provider)
  local model = activeModel()
  return ui.row({ gap = 4, align = "center" }, {
    ui.button({
      text = p.name .. " · " .. (if model ~= "" then model else tr("chat.no_model")),
      glyph = if picking then "chevron-up" else "chevron-down",
      variant = "ghost", contentAlign = "start", flexGrow = 1, enabled = not snap.busy,
      tooltip = tr("chat.switch_model"), onClick = "onPicker",
    }),
    ui.button({ glyph = "paperclip", variant = "ghost", tooltip = tr("chat.attach"), onClick = "onAttach" }),
    ui.button({ glyph = "plus", variant = "ghost", tooltip = tr("chat.new"), onClick = "onNewChat" }),
    ui.button({ glyph = "key", variant = "ghost", selected = mode == "settings", tooltip = tr("chat.settings"), onClick = "onSettings" }),
  })
end

local function picker()
  local p = providerDef(prefs.provider)
  local names, current = {}, 0
  for i, def in PROVIDERS do
    table.insert(names, def.name)
    if def.id == p.id then current = i - 1 end
  end
  local rows = { ui.select({ options = names, selectedIndex = current, onChange = "onProvider" }) }
  local list = models[p.id]
  if modelStatus[p.id] then
    table.insert(rows, ui.label({ text = modelStatus[p.id], fontSize = 12, color = "on_surface_variant" }))
  elseif list and #list > 0 then
    local labels, selected = {}, nil
    for i, m in list do
      table.insert(labels, m.label)
      if m.id == activeModel() then selected = i - 1 end
    end
    table.insert(rows, ui.select({ options = labels, selectedIndex = selected, placeholder = tr("chat.no_model"), onChange = "onModel" }))
  end
  return ui.column({ gap = 6, padding = 8, radius = 10, fill = "surface_variant/0.5" }, rows)
end

local function chatView()
  local items = {}
  if #snap.messages == 0 and not snap.busy then
    table.insert(items, ui.label({
      text = if firstName then tr("chat.greeting", { name = firstName }) else tr("chat.greeting_anon"),
      fontSize = 16, fontWeight = "semibold", color = "on_surface",
    }))
    table.insert(items, ui.label({ text = tr("chat.intro"), fontSize = 12, color = "on_surface_variant" }))
  end
  for i, m in snap.messages do table.insert(items, bubble(m, i)) end
  if snap.busy then
    if snap.live ~= "" then
      table.insert(items, ui.markdown({ key = "live", text = snap.live }))
    else
      table.insert(items, ui.label({ key = "live", text = "• • •", color = "on_surface_variant" }))
    end
  end
  if snap.error ~= "" then
    table.insert(items, ui.label({ key = "err", text = snap.error, fontSize = 12, color = "error" }))
  end
  local children = { header() }
  if picking then table.insert(children, picker()) end
  if type(snap.file) == "table" then
    table.insert(children, ui.row({ gap = 6, align = "center", paddingH = 8, paddingV = 4, radius = 10, fill = "primary/0.12" }, {
      ui.glyph({ name = "file", size = 14, color = "primary" }),
      ui.label({ text = snap.file.name, fontSize = 12, flexGrow = 1, maxLines = 1 }),
      ui.button({ glyph = "x", variant = "ghost", controlSize = "sm", tooltip = tr("chat.detach"), onClick = "onDetach" }),
    }))
  end
  table.insert(children, ui.scroll({ flexGrow = 1, gap = 10, stickToBottom = true }, items))
  local n = noticeRow()
  if n then table.insert(children, n) end
  table.insert(children, ui.row({ gap = 6, align = "end" }, {
    ui.input({
      key = "q" .. inputRev, value = draft, flexGrow = 1, multiline = true, submitOnEnter = true, focus = true,
      placeholder = tr(if #snap.messages == 0 then "chat.placeholder_first" else "chat.placeholder_next"),
      onChange = "onDraft", onSubmit = "onSend",
    }),
    if snap.busy
      then ui.button({ glyph = "player-stop", variant = "destructive", tooltip = tr("chat.stop"), onClick = "onStop" })
      else ui.button({ glyph = "send", variant = "primary", tooltip = tr("chat.send"), onClick = "onSend" }),
  }))
  return ui.column({ padding = 12, gap = 8, flexGrow = 1 }, children)
end

local function keyRow(p)
  local saved = keys[p.id]
  local status = if saved == nil then "…" elseif saved then tr("chat.key_saved") else tr("chat.key_missing")
  return ui.column({ key = "k" .. p.id, gap = 4 }, {
    ui.row({ gap = 6, align = "center" }, {
      ui.label({ text = if p.id == "custom" then tr("chat.custom_key") else p.name, fontWeight = "semibold", flexGrow = 1 }),
      ui.label({ text = status, fontSize = 12, color = if saved then "primary" else "on_surface_variant" }),
    }),
    ui.row({ gap = 6 }, {
      ui.input({
        key = "kin" .. p.id .. formRev, password = true, flexGrow = 1, controlSize = "sm",
        placeholder = tr("chat.key_placeholder"),
        onChange = function(v) keyDrafts[p.id] = v end,
        onSubmit = function(v) keyDrafts[p.id] = v; saveKey(p.id) end,
      }),
      ui.button({ text = tr("chat.save"), controlSize = "sm", variant = "primary", onClick = function() saveKey(p.id) end }),
      ui.button({ text = tr("chat.delete"), controlSize = "sm", variant = "outline", enabled = saved == true,
        onClick = function() deleteKey(p.id) end }),
    }),
  })
end

local function serverRow(p)
  return ui.column({ key = "s" .. p.id, gap = 4 }, {
    ui.label({ text = p.name, fontWeight = "semibold" }),
    ui.row({ gap = 6 }, {
      ui.input({
        key = "sin" .. p.id .. formRev, value = serverUrl(p.id), flexGrow = 1, controlSize = "sm",
        placeholder = if p.server ~= "" then p.server else tr("chat.server_placeholder"),
        onChange = function(v) serverDrafts[p.id] = v end,
        onSubmit = function(v) serverDrafts[p.id] = v; saveServer(p.id) end,
      }),
      ui.button({ text = tr("chat.save"), controlSize = "sm", variant = "primary", onClick = function() saveServer(p.id) end }),
    }),
  })
end

local function settingsView()
  local rows = {
    ui.label({ text = tr("chat.keys_title"), fontWeight = "semibold", color = "primary" }),
    ui.label({ text = tr("chat.keys_intro"), fontSize = 12, color = "on_surface_variant" }),
  }
  for _, p in PROVIDERS do
    if p.key and not p.server then table.insert(rows, keyRow(p)) end
  end
  table.insert(rows, ui.separator({}))
  table.insert(rows, ui.label({ text = tr("chat.servers_title"), fontWeight = "semibold", color = "primary" }))
  table.insert(rows, ui.label({ text = tr("chat.servers_intro"), fontSize = 12, color = "on_surface_variant" }))
  for _, p in PROVIDERS do
    if p.server then table.insert(rows, serverRow(p)) end
  end
  table.insert(rows, keyRow(providerDef("custom")))
  local children = { header(), ui.scroll({ flexGrow = 1, gap = 10 }, rows) }
  local n = noticeRow()
  if n then table.insert(children, n) end
  table.insert(children, ui.button({ text = tr("chat.back"), glyph = "arrow-left", variant = "outline", onClick = "onSettings" }))
  return ui.column({ padding = 12, gap = 8, flexGrow = 1 }, children)
end

render = function()
  panel.render(if mode == "settings" then settingsView() else chatView())
end

local function readSnap(v)
  if type(v) == "table" then
    snap = v
    snap.messages = snap.messages or {}
    snap.live = snap.live or ""
    snap.error = snap.error or ""
  end
  render()
end

function onOpen(_context)
  mode = "chat"
  notice = nil
  inputRev += 1 -- a fresh field takes the keyboard
  local p = providerDef(prefs.provider)
  if p.key then refreshKey(p.id) end
  render()
end

function onClose()
  picking = false
end

loadPrefs()
readFirstName()
noctalia.state.watch(STATE, readSnap)
readSnap(noctalia.state.get(STATE))
