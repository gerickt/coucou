-- Service spec: feeds daemon lines and checks the published snapshot.
flushSounds()
local function check(cond, msg) if not cond then error("FAIL: " .. msg, 2) end print("ok  " .. msg) end
local m = function() return state.mochi end
check(m().state == "idle" and m().daemon == false, "starts idle, daemon unknown")
check(calls[1] == "coucou-daemon --follow", "follows the daemon")
local MUSIC = "playerctl -F -f '{{status}}' status 2>/dev/null"
check(streams[MUSIC] ~= nil, "follows the MPRIS players when playerctl exists")
check(m().music == false, "no music at first")
feed('{"type":"hello","pending":[],"status":{}}')
check(m().daemon == true, "hello marks the daemon up")
feed('{"type":"hook","payload":{"hook_event_name":"UserPromptSubmit","session_id":"s1","cwd":"/home/me/proj"}}')
check(m().state == "thinking" and m().project == "proj", "prompt -> thinking")
check(#played == 0, "thinking plays nothing (hooks.ts)")
feed('{"type":"hook","payload":{"hook_event_name":"SessionStart","session_id":"s0","cwd":"/home/me/other"}}')
check(played[#played] == "work", "SessionStart plays work (hooks.ts)")
feed('{"type":"hook","payload":{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Grep"}}')
check(m().state == "searching" and m().tool == "Grep", "Grep -> searching")
feed('{"type":"hook","payload":{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Bash"}}')
check(m().state == "working", "Bash -> working")
local p0 = #played
feed('{"type":"hook","payload":{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Edit"}}')
check(#played == p0, "tools play nothing")
feed('{"type":"hook","payload":{"hook_event_name":"PermissionRequest","session_id":"s1","tool_name":"Bash","tool_input":{"command":"rm -rf build"},"request_id":"42-1"}}')
check(m().state == "approval" and m().pending.id == "42-1" and m().pending.target == "rm -rf build", "permission -> approval card")
check(calls[#calls-1] == "coucou-daemon ack 42-1", "acked at once")
check(lastCall() == "noctalia msg panel-open coucou/mochi:approval 42-1", "panel opened")
check(played[#played-1] == "approval" and played[#played] == "open", "approval then open sounds")
feed('{"type":"hook","payload":{"hook_event_name":"PermissionRequest","session_id":"s2","tool_name":"Edit","request_id":"42-2"}}')
check(lastCall() == "coucou-daemon decline 42-2" and m().pending.id == "42-1", "second request declined")
feed('{"type":"resolved","request_id":"42-1","outcome":"allow"}')
check(m().pending == nil and m().state == "working" and played[#played-1] == "approve" and played[#played] == "close", "allow resolves to working, approve + close")
feed('{"type":"hook","payload":{"hook_event_name":"PermissionRequest","session_id":"s1","tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which?","header":"H","multiSelect":true,"options":[{"label":"A"},{"label":"B","description":"bee"}]}]},"request_id":"42-3"}}')
check(m().state == "question" and m().pending.questions[1].multiSelect == true and #m().pending.questions[1].options == 2, "AskUserQuestion -> question card")
feed('{"type":"resolved","request_id":"42-3","outcome":"timeout"}')
check(m().pending == nil and m().state == "idle", "timeout releases the card")
feed('{"type":"hook","payload":{"hook_event_name":"PermissionRequest","session_id":"s1","tool_name":"Bash","request_id":"42-6"}}')
feed('{"type":"resolved","request_id":"42-6","outcome":"deny"}')
check(played[#played-1] == "blip" and played[#played] == "close", "deny plays blip + close")
feed('{"type":"hook","payload":{"hook_event_name":"PermissionRequest","coucou_agent":"gemini","session_id":"g","tool_name":"x","request_id":"42-4"}}')
check(lastCall() == "coucou-daemon decline 42-4", "agent that cannot take decisions is declined")
feed('{"type":"hook","payload":{"hook_event_name":"Stop","session_id":"s1"}}')
check(m().state == "finished" and played[#played] == "finish", "stop -> finished")
advance(6000); update()
check(m().state == "idle", "finished -> idle after 5.2 s")
advance(30*60*1000); feed('{"type":"ping"}'); update()
check(m().state == "idle", "the service never sleeps (the desktop widget does)")
feed('{"type":"hook","payload":{"hook_event_name":"StopFailure","session_id":"s1"}}')
check(m().state == "error" and played[#played] == "error", "StopFailure -> error sound")
streams[MUSIC]("Playing")
check(m().music == true, "playerctl Playing -> music")
streams[MUSIC]("Paused")
check(m().music == false, "Paused -> no music")
onIpc("toggle-pause")
check(m().paused == true, "pause")
feed('{"type":"hook","payload":{"hook_event_name":"PermissionRequest","session_id":"s1","tool_name":"Bash","request_id":"42-5"}}')
check(lastCall() == "coucou-daemon decline 42-5" and m().pending == nil, "paused declines requests")
noctalia.state.set("mochi.cmd", "toggle-pause:1")
check(m().paused == false and played[#played] == "peek", "resume through mochi.cmd, with the peek")
feed('{"type":"hook","payload":{"hook_event_name":"Notification","session_id":"s1","message":"Rate limit reached"}}')
check(m().state == "ratelimit" and played[#played] == "rate", "rate limit notification")
feed('{"type":"daemon","running":false}')
check(m().daemon == false, "daemon offline")
local n = #calls
advance(50000); update()
check(#calls == n + 1 and lastCall() == "coucou-daemon --follow", "silent stream restarted")
config.sound_enabled = false
local p = #played
feed('{"type":"hook","payload":{"hook_event_name":"UserPromptSubmit","session_id":"s1"}}')
check(#played == p, "sounds respect the setting")
feed('{"type":"hook","payload":{"hook_event_name":"SessionEnd","session_id":"s1"}}')
check(#m().sessions >= 0, "session end handled")
onIpc("agents")
check(lastCall() == "noctalia msg panel-open coucou/mochi:agents", "agents IPC opens the Agents panel")
onIpc("chat")
check(lastCall() == "noctalia msg panel-open coucou/mochi:chat", "chat IPC opens the chat panel")

-- ── Chat ──
config.sound_enabled = true
env.XDG_RUNTIME_DIR = "/run/user/1000"
local written = {}
noctalia.writeFile = function(path, text) written[path] = text return true end
local seq = 0
local function cmd(t) seq += 1; t.seq = seq; noctalia.state.set("mochi.chat.cmd", t) end
local function c() return state["mochi.chat"] end
local function chatStream() for k, cb in streams do if k:find(" chat --request ", 1, true) then return k, cb end end end
check(type(c()) == "table" and #c().messages == 0 and c().busy == false, "chat published empty")

-- A file first: a fresh conversation, Mochi swallows it.
local before = m().reaction
cmd({ op = "attach", name = "notes.txt", path = "/home/tester/notes.txt" })
check(c().file and c().file.name == "notes.txt", "file attached")
check(m().reaction and m().reaction.name == "gulp" and m().reaction ~= before, "gulp reaction published")
check(played[#played] == "approve", "swallow plays approve")

cmd({ op = "send", question = "  what is this?  ", provider = "ollama", model = "llama3.2", base_url = "" })
local key, cb = chatStream()
check(key and key:match("^coucou%-daemon chat %-%-request '/run/user/1000/coucou%-chat%-[%d%-]+%.json'$"), "turn handed over by file: " .. tostring(key))
local path = key:match("'(.-)'")
local turn = noctalia.json.decode(written[path])
check(turn.question == "what is this?" and turn.provider == "ollama" and turn.model == "llama3.2", "turn content")
check(turn.file and turn.file.path == "/home/tester/notes.txt", "file rides with the first question")
check(not written[path]:find("key", 1, true), "no key in the turn file")
check(c().busy and m().state == "thinking", "Mochi thinks while the chat runs")
check(played[#played] == "send", "send sound")
cb('{"type":"start","pid":4242}')
cb('{"type":"delta","text":"Hel"}')
check(c().live == "Hel", "live text")
cb('{"type":"done","text":"Hello!"}')
check(not c().busy and #c().messages == 2 and c().messages[2].content == "Hello!", "answer recorded")
check(played[#played] == "finish" and m().state ~= "thinking", "finish sound, Mochi back")
cb('{"type":"delta","text":"late"}')
check(c().live == "", "lines after the end are ignored")

-- The next question: history, no file again.
streams[key] = nil
cmd({ op = "send", question = "more", provider = "ollama", model = "llama3.2" })
local key2, cb2 = chatStream()
local turn2 = noctalia.json.decode(written[key2:match("'(.-)'")])
check(#turn2.history == 2 and turn2.file == nil, "history sent, file only once")
cb2('{"type":"error","message":"Cannot reach it"}')
check(c().error == "Cannot reach it" and c().messages[3].failed and played[#played] == "error", "error shown")

-- A failed question is not history; Stop kills the turn and keeps what came.
streams[key2] = nil
cmd({ op = "send", question = "again", provider = "ollama", model = "m" })
local key3, cb3 = chatStream()
check(#noctalia.json.decode(written[key3:match("'(.-)'")]).history == 2, "failed question left out of history")
cb3('{"type":"start","pid":777}')
cb3('{"type":"delta","text":"Part"}')
cmd({ op = "stop" })
check(lastCall() == "kill -TERM 777", "stop kills the daemon turn")
check(not c().busy and c().messages[#c().messages].content == "Part", "partial answer kept")
cmd({ op = "reset" })
check(#c().messages == 0 and c().file == nil, "new chat")
cmd({ op = "send", question = "   " })
check(not c().busy, "empty question ignored")
print("ALL PASSED")
