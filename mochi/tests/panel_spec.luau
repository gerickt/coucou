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
print("PANEL PASSED")
