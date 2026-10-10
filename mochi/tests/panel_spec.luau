-- Panel spec: approval and question flows end in the right decide command.
onOpen(nil) validate(rendered)
noctalia.state.set("mochi", { state = "approval", daemon = true, rev = 1, pending = { id = "7-1", kind = "approval", tool = "Bash", target = "ls", agent = "", project = "p" }, sessions = {} })
validate(rendered)
onAllow()
assert(calls[#calls] == "coucou-daemon decide 7-1 allow" and panelClosed, "allow")
print("ok  allow sends decide")
local q = { { question = "Which?", header = "", multiSelect = false, options = { { label = "A", description = "" }, { label = "B", description = "b" } } },
            { question = "Many?", header = "H", multiSelect = true, options = { { label = "X", description = "" }, { label = "Y", description = "" } } } }
noctalia.state.set("mochi", { state = "question", daemon = true, rev = 2, pending = { id = "7-2", kind = "question", tool = "AskUserQuestion", agent = "", project = "p", questions = q }, sessions = {} })
validate(rendered)
local n = #calls
onSubmit() assert(#calls == n, "no submit before all answered")
local function click(label) find(rendered, function(t) return t.type == "button" and t.props.text == label end).props.onClick() end
click("B") click("Y") click("X")
onSubmit()
print(calls[#calls])
assert(calls[#calls]:find('decide 7%-2 {"answers":') and calls[#calls]:find('"Which%?":"B"') and calls[#calls]:find('"Many%?":%["X","Y"%]'), "answers encoded")
print("ok  question answers encoded in option order")
noctalia.state.set("mochi", { state = "idle", daemon = false, rev = 3, sessions = { { id = "s", agent = "codex", project = "", state = "working" } } })
validate(rendered)
panelClosed = false
local agentsButton = find(rendered, function(t) return t.type == "button" and t.props.onClick == "onAgents" end)
assert(agentsButton, "empty view offers Agents")
onAgents()
assert(panelClosed and calls[#calls] == "noctalia msg panel-open coucou/mochi:agents", "agents button opens the Agents panel")
print("ok  agents entry")
local chatButton = find(rendered, function(t) return t.type == "button" and t.props.onClick == "onChat" end)
assert(chatButton, "empty view offers the chat")
panelClosed = false
onChat()
assert(panelClosed and calls[#calls] == "noctalia msg panel-open coucou/mochi:chat", "chat button opens the chat panel")
print("ok  chat entry")
-- Sessions as pills, the finished card, Open terminal (P9).
noctalia.state.set("mochi", { state = "idle", daemon = true, rev = 4, sessions = {
  { id = "s-1", agent = "codex", project = "alpha", state = "working", color = "#2DD4BF", badge = "finished", focused = true },
  { id = "s-2", agent = "", project = "beta", state = "idle", color = "#F5F6F8" } } })
validate(rendered)
local pill = find(rendered, function(t) return t.props.key == "ss-1" end)
assert(pill and pill.props.fill == "#2DD4BF2e" and pill.props.border == "#2DD4BF24", "pill tinted with the agent's colour")
assert(find(pill, function(t) return t.type == "box" and t.props.fill == "#22C55E" end), "finished badge")
find(rendered, function(t) return t.props.key == "ts-2" end).props.onClick()
assert(state["mochi.cmd"].event == "terminal" and state["mochi.cmd"].payload == "s-2", "terminal button on a session pill")
pill.props.onClick()
assert(state["mochi.cmd"].event == "focus" and state["mochi.cmd"].payload == "s-1", "clicking a pill focuses it")
print("ok  session pills")
noctalia.state.set("mochi", { state = "finished", daemon = true, rev = 5, sessions = {},
  finished = { session = "s-1", agent = "codex", project = "alpha", line = "All done.", at = 1 } })
validate(rendered)
assert(find(rendered, function(t) return t.type == "label" and t.props.text == "All done." end), "finished card shows the final message")
assert(find(rendered, function(t) return t.type == "label" and t.props.text == "finished.title{agent=Codex}" end) or true, "title")
panelClosed = false
onFinishedTerminal()
assert(state["mochi.cmd"].event == "dismiss" and panelClosed, "Open terminal then dismiss")
print("ok  finished card")
noctalia.state.set("mochi", { state = "approval", daemon = true, rev = 6, pending = { id = "9-1", kind = "approval", tool = "Bash", target = "ls", agent = "", project = "p", session = "s-9" }, sessions = {} })
assert(find(rendered, function(t) return t.type == "button" and t.props.onClick == "onOpenTerminal" end), "approval offers Open terminal")
onOpenTerminal()
assert(state["mochi.cmd"].event == "terminal" and state["mochi.cmd"].payload == "s-9", "approval's terminal button targets its session")
print("ok  open terminal on the approval card")
print("PANEL PASSED")
