-- Agents panel spec: status list, preview first, writes only on the confirm click.
local replies = {}
local ran = {}
noctalia.runAsync = function(argv, cb)
  local cmd = table.concat(argv, " ")
  table.insert(ran, cmd)
  local reply = { exitCode = 0, stdout = "", stderr = "" }
  for pattern, r in replies do
    if cmd:find(pattern, 1, true) then reply = r end
  end
  if cb then cb(reply) end
  return true
end
local function button(pred) return find(rendered, function(t) return t.type == "button" and pred(t.props) end) end
local function byText(text) return button(function(p) return p.text == text end) end
local function click(b)
  local h = b.props.onClick
  if type(h) == "string" then ({ onConfirm = onConfirm, onCancel = onCancel, onRefresh = onRefresh })[h]() else h() end
end
local function writes()
  local n = 0
  for _, c in ran do
    if c:find("--yes", 1, true) then n += 1 end
  end
  return n
end

replies["--status --json"] = { exitCode = 0, stderr = "", stdout = [[{"relay":"/usr/bin/coucou-hook","relay_ok":true,"agents":[
  {"id":"claude","name":"Claude Code","detected":true,"installed":true,"connected":true,"approvals":true,"note":""},
  {"id":"cursor","name":"Cursor Agent","detected":true,"installed":false,"connected":false,"approvals":false,"note":"Restart Cursor to pick the hooks up."},
  {"id":"bad id; rm","name":"x","detected":false,"installed":false,"connected":false,"approvals":false}]}]] }

onOpen(nil)
validate(rendered)
assert(ran[1] == "coucou-daemon install-hooks --status --json", ran[1])
assert(byText("agents.disconnect") ~= nil, "installed agent offers disconnect")
assert(button(function(p) return p.key == "connect-cursor" end).props.text == "agents.connect", "cursor connect")
assert(button(function(p) return p.key == "connect-bad id; rm" end) == nil, "unsafe ids are not listed")
print("ok  status list with detected/connected chips")

-- Connect shows the preview and writes nothing.
replies["--dry-run --json --agent cursor"] = { exitCode = 0, stderr = "", stdout = [[{"relay":"/usr/bin/coucou-hook","remove":false,"agents":[
  {"id":"cursor","name":"Cursor Agent","changed":true,"diff":"--- a\n+++ a (Coucou)\n+ {\n","backup_path":"/h/.cursor/hooks.json.coucou-backup-20261010-074605","files":[{"path":"/h/.cursor/hooks.json","changed":true,"backup_path":null,"sha256":"ab12"}]}]}]] }
button(function(p) return p.key == "connect-cursor" end).props.onClick()
validate(rendered)
assert(ran[#ran] == "coucou-daemon install-hooks --dry-run --json --agent cursor", ran[#ran])
assert(writes() == 0, "a preview never writes")
local md = find(rendered, function(t) return t.type == "markdown" end)
assert(md and md.props.text:sub(1, 8) == "```diff\n" and md.props.text:find("+ {", 1, true), "diff in a fenced block")
assert(find(rendered, function(t) return t.type == "label" and t.props.text == "agents.backup" end), "backup path shown")
print("ok  preview shows the diff and the backup")

-- Cancel goes back without writing.
onCancel()
assert(writes() == 0 and byText("agents.confirm_connect") == nil, "cancel")

-- Confirm writes exactly the agent, with --yes, then reloads.
button(function(p) return p.key == "connect-cursor" end).props.onClick()
click(byText("agents.confirm_connect"))
assert(writes() == 1, "one write")
local w
for _, c in ran do if c:find("--yes", 1, true) then w = c end end
assert(w == "coucou-daemon install-hooks --agent cursor --yes --expect-sha256 /h/.cursor/hooks.json=ab12", w)
assert(ran[#ran] == "coucou-daemon install-hooks --status --json", "reloads after writing")
assert(find(rendered, function(t) return t.type == "label" and t.props.text and t.props.text:find("Restart Cursor", 1, true) end), "note shown")
print("ok  confirm writes with --yes and the previewed fingerprints, then reloads")

-- Disconnect previews with --remove, and confirm passes --remove.
replies["--dry-run --json --agent claude --remove"] = { exitCode = 0, stderr = "", stdout = [[{"agents":[
  {"id":"claude","name":"Claude Code","changed":true,"diff":"- x\n","backup_path":"/h/.claude/settings.json.coucou-backup-1","files":[{"path":"/h/.claude/settings.json","changed":true,"sha256":"cd34"}]}]}]] }
byText("agents.disconnect").props.onClick()
assert(ran[#ran] == "coucou-daemon install-hooks --dry-run --json --agent claude --remove", ran[#ran])
click(byText("agents.confirm_remove"))
assert(writes() == 2)
for _, c in ran do if c:find("--yes", 1, true) then w = c end end
assert(w == "coucou-daemon install-hooks --agent claude --yes --remove --expect-sha256 /h/.claude/settings.json=cd34", w)
print("ok  disconnect previews and removes")

-- A config the daemon refuses shows its error and offers no confirm.
replies["--dry-run --json --agent cursor"] = { exitCode = 0, stderr = "", stdout = [[{"agents":[
  {"id":"cursor","name":"Cursor Agent","changed":false,"diff":"","backup_path":null,"error":"hooks.json is not valid JSON"}]}]] }
button(function(p) return p.key == "connect-cursor" end).props.onClick()
assert(byText("agents.confirm_connect") == nil, "no confirm on error")
assert(find(rendered, function(t) return t.type == "label" and t.props.text == "hooks.json is not valid JSON" end), "error shown")
onConfirm()
assert(writes() == 2, "onConfirm without a valid preview does nothing")
onCancel()

-- Nothing to change: no confirm either.
replies["--dry-run --json --agent cursor"] = { exitCode = 0, stderr = "", stdout = [[{"agents":[{"id":"cursor","name":"Cursor Agent","changed":false,"diff":"","backup_path":null}]}]] }
button(function(p) return p.key == "connect-cursor" end).props.onClick()
assert(byText("agents.confirm_connect") == nil and find(rendered, function(t) return t.props.text == "agents.up_to_date" end), "up to date")
onCancel()
print("ok  errors and no-op previews never offer a write")

-- Without a relay, Connect is disabled; a dead daemon call reads as an error.
replies["--status --json"] = { exitCode = 0, stderr = "", stdout = [[{"relay":"coucou-hook","relay_ok":false,"agents":[{"id":"codex","name":"Codex","detected":true,"installed":false,"connected":false,"approvals":true}]}]] }
onRefresh()
validate(rendered)
assert(button(function(p) return p.key == "connect-codex" end).props.enabled == false, "connect disabled without relay")
replies["--status --json"] = { exitCode = 1, stderr = "coucou-daemon: unknown option: --status\nusage", stdout = "" }
onRefresh()
assert(find(rendered, function(t) return t.type == "label" and t.props.text == "unknown option: --status" end), "load error shown")
print("ok  missing relay and daemon errors")
print("AGENTS PASSED")
