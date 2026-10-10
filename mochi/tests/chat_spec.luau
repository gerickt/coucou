-- Chat panel spec: asks the service through state, keys only by presence and
-- by a hand-off file, models from the daemon, files from the clipboard.
local replies = {}
local ran = {}
local files = {}
local copied = nil
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
local writes = {}
noctalia.writeFile = function(p, t) files[p] = t table.insert(writes, { p, t }) return true end
noctalia.readFile = function(p) return files[p] end
noctalia.fileExists = function(p) return files[p] ~= nil end
noctalia.removeFile = function(p) files[p] = nil return true end
noctalia.fileInfo = function(p) if p == "/home/tester/My Notes.txt" then return { size = 3, mtime = 0, isDir = false } end return nil, "missing" end
noctalia.copyToClipboard = function(t) copied = t return true end
noctalia.clipboardText = function() return nil end
noctalia.string = { urlDecode = function(s) return (s:gsub("%%(%x%x)", function(h) return string.char(tonumber(h, 16)) end)) end }
env.XDG_RUNTIME_DIR = "/run/user/1000"

local function check(cond, msg) if not cond then error("FAIL: " .. msg, 2) end print("ok  " .. msg) end
local function node(pred) return find(rendered, function(t) return pred(t) end) end
local function button(pred) return node(function(t) return t.type == "button" and pred(t.props) end) end
local function label(text) return node(function(t) return t.type == "label" and t.props.text == text end) end
local function cmd() return state["mochi.chat.cmd"] end
local function ranWith(s) for _, c in ran do if c:find(s, 1, true) then return c end end return nil end
local function click(b)
  local h = b.props.onClick
  local named = { onStop = onStop, onPicker = onPicker, onDetach = onDetach, onSettings = onSettings,
    onSend = onSend, onAttach = onAttach, onNewChat = onNewChat }
  if type(h) == "string" then named[h]() else h() end
end

-- Opening: the first name greets, the field takes the keyboard.
replies["secret has anthropic"] = { exitCode = 0, stderr = "", stdout = '{"key":"anthropic-api-key","present":true}' }
onOpen()
validate(rendered)
check(label("chat.greeting"), "greets by first name")
local input = node(function(t) return t.type == "input" end)
check(input and input.props.focus and input.props.submitOnEnter, "question field focused, Enter sends")

-- Sending asks the service, with the provider and model, never a key.
onSend("  Hello  ")
check(cmd().op == "send" and cmd().question == "Hello" and cmd().provider == "anthropic" and cmd().model == "claude-opus-5", "send command")
local firstSeq = cmd().seq
onSend("")
check(cmd().seq == firstSeq, "empty question not sent")

-- The conversation from the service: markdown answers, copy, code copy.
noctalia.state.set("mochi.chat", { messages = {
  { role = "user", content = "Hello" },
  { role = "assistant", content = "Here:\n```sh\nls -la\n```\nDone." },
}, live = "", busy = false, error = "", rev = 2 })
validate(rendered)
check(node(function(t) return t.type == "markdown" and t.props.text:find("Done.", 1, true) end), "answer as markdown")
click(button(function(p) return p.text == "chat.copy" end))
check(copied and copied:find("ls %-la"), "copy answer")
click(button(function(p) return p.text == "chat.copy_code" end))
check(copied == "ls -la\n", "copy code block")
check(node(function(t) return t.type == "scroll" and t.props.stickToBottom end), "list sticks to the bottom")

-- While a turn runs: the live text and Stop.
noctalia.state.set("mochi.chat", { messages = { { role = "user", content = "Q" } }, live = "Part", busy = true, error = "", rev = 3 })
check(node(function(t) return t.type == "markdown" and t.props.text == "Part" end), "live answer shown")
click(button(function(p) return p.onClick == "onStop" end))
check(cmd().op == "stop", "stop command")
noctalia.state.set("mochi.chat", { messages = {}, live = "", busy = false, error = "", rev = 4 })

-- The model name opens the picker; the list comes from the daemon.
replies["chat-models anthropic"] = { exitCode = 0, stderr = "", stdout = '{"models":[{"id":"claude-haiku-4-5","label":"Claude Haiku"},{"id":"claude-opus-5-1","label":"Claude Opus 5.1"}]}' }
click(button(function(p) return p.onClick == "onPicker" end))
local sel = node(function(t) return t.type == "select" and t.props.onChange == "onModel" end)
check(sel and #sel.props.options == 2, "model list in a select")
check(cmd().op == "stop" and button(function(p) return p.onClick == "onPicker" end).props.text:find("claude-opus-5-1", 1, true), "an offered opus is picked when the saved model is gone")
onModel("0")
check(button(function(p) return p.onClick == "onPicker" end).props.text == "Anthropic · claude-haiku-4-5", "model chosen")
check(files["/data/chat.json"] and files["/data/chat.json"]:find("claude-haiku-4-5", 1, true), "choice saved (no key in it)")
check(pickModel({ prefer = "flash", default = "x" }, { "a", "gemini-flash" }, "zzz") == "gemini-flash", "pickModel prefers")

-- Switching to a model server: no key asked, its address when set.
onPicker()
replies["chat-models ollama"] = { exitCode = 0, stderr = "", stdout = '{"error":"Cannot reach http://127.0.0.1:11434. Is the server running?"}' }
onProvider("4")
check(ranWith("chat-models ollama") and not ranWith("chat-models ollama --base-url"), "ollama models at the usual address")
check(label("Cannot reach http://127.0.0.1:11434. Is the server running?"), "unreachable server said")
onSend("hi")
check(cmd().provider == "ollama" and cmd().base_url == "", "send goes to ollama")

-- Keys: presence only; saved through a hand-off file, never on a command line.
replies["secret has openai"] = { exitCode = 0, stderr = "", stdout = '{"key":"openai-api-key","present":false}' }
onSettings()
validate(rendered)
check(label("chat.key_saved") and label("chat.key_missing"), "key presence shown")
local openaiInput = node(function(t) return t.type == "input" and t.props.key and t.props.key:find("^kinopenai") end)
check(openaiInput and openaiInput.props.password, "key field is masked")
openaiInput.props.onChange("sk-test-SECRET")
local openaiRow = node(function(t) return t.props.key == "kopenai" end)
click(find(openaiRow, function(t) return t.type == "button" and t.props.text == "chat.save" end))
local setCmd = ranWith("secret set openai")
check(setCmd and setCmd:find("--from-file /run/user/1000/coucou-key-", 1, true), "key handed over by file")
for _, c in ran do check(not c:find("SECRET", 1, true), "no key on a command line: " .. c:sub(1, 40)) end
local handed = nil
for _, w in writes do if w[1]:find("coucou-key-", 1, true) and w[2] == "sk-test-SECRET" then handed = w[1] end end
check(handed and setCmd:find(handed, 1, true), "the hand-off file holds the key for the daemon")
check(files[handed] == nil, "and is gone afterwards")
-- The custom key needs its address first.
local customInput = node(function(t) return t.type == "input" and t.props.key and t.props.key:find("^kincustom") end)
customInput.props.onChange("k2")
click(find(node(function(t) return t.props.key == "kcustom" end), function(t) return t.type == "button" and t.props.text == "chat.save" end))
check(label("chat.custom_url_first") and not ranWith("secret set custom"), "custom key waits for the address")
local srv = node(function(t) return t.type == "input" and t.props.key and t.props.key:find("^sincustom") end)
srv.props.onChange("https://llm.example.com")
click(find(node(function(t) return t.props.key == "scustom" end), function(t) return t.type == "button" end))
customInput = node(function(t) return t.type == "input" and t.props.key and t.props.key:find("^kincustom") end)
customInput.props.onChange("k2")
click(find(node(function(t) return t.props.key == "kcustom" end), function(t) return t.type == "button" and t.props.text == "chat.save" end))
check(ranWith("secret set custom --bound-url https://llm.example.com --from-file"), "custom key bound to its address")
onSettings()

-- A copied file goes to the chat.
replies["wl-paste"] = { exitCode = 0, stderr = "", stdout = "file:///home/tester/My%20Notes.txt\n" }
onAttach()
check(cmd().op == "attach" and cmd().path == "/home/tester/My Notes.txt" and cmd().name == "My Notes.txt", "clipboard file attached")
replies["wl-paste"] = { exitCode = 1, stderr = "", stdout = "" }
onAttach()
check(label("chat.attach_none"), "nothing copied: says how")
noctalia.state.set("mochi.chat", { messages = {}, live = "", busy = false, error = "", file = { name = "My Notes.txt", path = "/x" }, rev = 5 })
check(label("My Notes.txt"), "file chip")
click(button(function(p) return p.onClick == "onDetach" end))
check(cmd().op == "detach", "detach")
print("CHAT PASSED")
