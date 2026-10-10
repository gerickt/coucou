-- Desktop spec: renders every state and plays the interactions.
local states = { "idle", "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping" }
for _, st in states do
  for _, size in { 64, 120, 256 } do
    config.size = size
    noctalia.state.set("mochi", { state = st, tool = "Bash", paused = false, daemon = true, rev = 1 })
    for f = 1, 90 do onFrameTick(16) validate(rendered) end
  end
end
print("ok  every state renders for 90 frames at 3 sizes")
noctalia.state.set("mochi", { state = "idle", tool = "", daemon = true, rev = 2 })
onFrameTick(16)
onMochiClick() for f = 1, 10 do onFrameTick(16) end
assert(played[#played] == "slap", "click slaps")
onMochiClick() onMochiClick()
for f = 1, 10 do onFrameTick(16) validate(rendered) end
assert(played[#played] == "dizzy", "triple click dizzy")
print("ok  click and triple click")
for f = 1, 300 do onFrameTick(16) end
onMochiHover("true") for f = 1, 150 do onFrameTick(16) validate(rendered) end
assert(played[#played] == "love", "hover love")
print("ok  hover 2 s -> love")
noctalia.state.set("mochi", { state = "approval", tool = "Bash", daemon = true, rev = 3 })
onFrameTick(16) onMochiClick()
assert(calls[#calls] == "toggle coucou/mochi:approval", "click on approval opens panel")
print("ok  click while approval opens the panel")
noctalia.state.set("mochi", { state = "working", paused = true, daemon = true, rev = 4 })
for f = 1, 10 do onFrameTick(16) validate(rendered) end
print("DESKTOP PASSED")
