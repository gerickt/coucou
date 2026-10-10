-- Desktop spec: plays the original Mochi's frame sequences and the interactions.
flushSounds()

local manifest = sprite.manifest.desktop.sequences
local function filesOf(key)
  local set = {}
  for _, f in manifest[key].frames do set["/plugin/" .. f[1]] = true end
  return set
end
local function image() return find(rendered, function(n) return n.type == "image" end) end
local function shows(key) local img = image() return img ~= nil and filesOf(key)[img.props.path] == true end
local function run(secs) local n = math.ceil(secs / 0.016) for _ = 1, n do onFrameTick(16) validate(rendered) end end

-- Every sequence the widget can play exists for every outfit.
for _, outfit in sprite.OUTFITS do
  if outfit ~= "auto" then
    for _, name in { "idle", "working", "thinking", "searching", "approval", "question", "error", "finished",
      "ratelimit", "sleeping", "dizzy", "approval.enter", "error.enter", "finished.enter", "dizzy.enter",
      "dizzy.exit", "slap", "love", "greet", "proud", "approval.alert", "question.alert", "gulp", "dance" } do
      assert(manifest[outfit .. "." .. name], "missing desktop sequence " .. outfit .. "." .. name)
    end
  end
end
print("ok  every outfit has every desktop sequence")

-- Start: the greeting plays (in today's "auto" outfit), then the idle loop.
assert(image() and shows(sprite.resolveOutfit(nil) .. ".greet"), "greets on start")
config.outfit = "none"
assert(played[#played] == "greeting", "the launch greeting's own sound (greeting.ts)")
assert(calls[1] == "getent passwd tester", "reads the account's full name")
assert(find(rendered, function(n) return n.type == "label" and n.props.text == "desktop.greeting" end), "greets by first name")
run(2.5)
noctalia.state.set("mochi", { state = "idle", tool = "", daemon = true, rev = 1 })
run(0.1)
assert(shows("none.idle"), "idle loop after the greeting")
print("ok  greet, then idle")

-- Every state loop renders; approval/error/finished play their entrance first.
for _, st in { "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping" } do
  noctalia.state.set("mochi", { state = st, tool = "Bash", paused = false, daemon = true, rev = 2 })
  run(0.05)
  local enter = ({ approval = "none.approval.alert", question = "none.question.alert", error = "none.error.enter", finished = "none.finished.enter" })[st]
  if enter then assert(shows(enter), st .. " plays its entrance") end
  run(2.0)
  assert(shows("none." .. st), st .. " loops")
  for _, size in { 64, 160, 256 } do
    config.size = size
    run(1.1)
    assert(image().props.width == size, "image follows the size setting")
  end
  config.size = nil
end
print("ok  every state loops, entrances play, sizes follow the setting")

-- Click = slap; three within 1.7 s = dizzy for 3.3 s, then happy back.
noctalia.state.set("mochi", { state = "idle", tool = "", daemon = true, rev = 3 })
run(1.0)
onMochiClick() run(0.02)
assert(shows("none.slap"), "click slaps")
assert(played[#played] == "slap", "slap sound")
run(0.1)
assert(played[#played] == "annoyed", "then annoyed, as slap() does")
onMochiClick() onMochiClick() run(0.02)
assert(played[#played] == "dizzy", "triple click dizzy")
assert(shows("none.dizzy.enter"), "dizzy roll")
run(2.0)
assert(shows("none.dizzy"), "dizzy loop")
local before = #played
onMochiClick() run(0.1)
assert(#played == before, "slaps are ignored while dizzy")
run(1.3)
assert(shows("none.dizzy.exit"), "recovers with a happy face after 3.3 s")
run(2.0)
assert(shows("none.idle"), "back to idle")
print("ok  click, triple click, recovery")

-- Hover 1.9 s = love, then not again within 6 s.
onMochiHover("true") run(0.02)
assert(played[#played] == "hover", "hover sound")
run(2.0)
assert(played[#played] == "love" and shows("none.love"), "hover 1.9 s -> love")
onMochiHover("false") run(0.5)
local loves = 0
for _, p in played do if p == "love" then loves += 1 end end
onMochiHover("true") run(2.5)
local after = 0
for _, p in played do if p == "love" then after += 1 end end
assert(after == loves, "no love again within 6 s")
onMochiHover("false")
print("ok  hover love with its cooldown")

-- A click while a request waits opens the panel.
noctalia.state.set("mochi", { state = "approval", tool = "Bash", daemon = true, rev = 4 })
run(0.05) onMochiClick()
assert(calls[#calls] == "toggle coucou/mochi:approval", "click on approval opens panel")
print("ok  click while approval opens the panel")

-- Paused: he sleeps.
noctalia.state.set("mochi", { state = "working", paused = true, daemon = true, rev = 5 })
run(1.0)
assert(shows("none.sleeping"), "paused sleeps")
print("ok  paused sleeps")

-- Outfits: a chosen one is worn at once.
config.outfit = "witchHat"
noctalia.state.set("mochi", { state = "idle", daemon = true, rev = 6 })
run(1.2)
assert(shows("witchHat.proud") and played[#played] == "pop", "a new outfit: pop, and proud (island.ts chooseOutfit)")
run(2.5)
assert(shows("witchHat.idle"), "wears the chosen outfit")
config.outfit = "garbage"
run(1.2)
assert(image() ~= nil, "unknown outfit falls back to auto")
print("ok  outfit setting")

-- Calm: no per-frame ticks while a still frame shows.
config.outfit = "none"
run(1.2)
local stillTicks = frameTick
run(0.5)
print("ok  frame tick while calm: " .. tostring(stillTicks) .. ", interval " .. tostring(updateInterval))

-- Seasons (wardrobe.ts seasonalOutfit).
assert(sprite.seasonalOutfit(2026, 10, 10) == "witchHat")
assert(sprite.seasonalOutfit(2026, 11, 1) == "witchHat")
assert(sprite.seasonalOutfit(2026, 11, 2) == "none")
assert(sprite.seasonalOutfit(2026, 12, 25) == "santaHat")
assert(sprite.seasonalOutfit(2026, 12, 31) == "partyHat")
assert(sprite.seasonalOutfit(2027, 1, 2) == "partyHat")
assert(sprite.seasonalOutfit(2026, 4, 3) == "bunnyEars") -- Easter 2026: April 5
assert(sprite.seasonalOutfit(2026, 4, 6) == "bunnyEars")
assert(sprite.seasonalOutfit(2026, 4, 7) == "none")
assert(sprite.seasonalOutfit(2027, 3, 28) == "bunnyEars") -- Easter 2027: March 28
assert(sprite.seasonalOutfit(2026, 7, 14) == "sunglasses")
assert(sprite.seasonalOutfit(2026, 6, 20) == "none")
assert(sprite.resolveOutfit("crown") == "crown")
print("ok  seasonal outfits")

-- Reactions over the current state's body; outfits without it use the idle one.
noctalia.state.set("mochi", { state = "working", tool = "Bash", daemon = true, rev = 20 })
run(1.0)
onMochiClick() run(0.02)
assert(shows("none.working.slap"), "slap drawn over the working body")
run(1.2)
config.outfit = "crown" run(3.5)
onMochiClick() run(0.02)
assert(shows("crown.slap"), "outfit falls back to the idle-body slap")
config.outfit = "none" run(3.5)

-- Dance while music plays, in the dance states (core/spotify.ts desktopDances).
noctalia.state.set("mochi", { state = "idle", daemon = true, music = true, rev = 21 })
run(0.5)
assert(shows("none.dance"), "dances to music when idle")
noctalia.state.set("mochi", { state = "error", daemon = true, music = true, rev = 22 })
run(1.5)
assert(shows("none.error"), "an error wins over the music")
noctalia.state.set("mochi", { state = "idle", daemon = true, music = false, rev = 23 })
run(0.5)
assert(shows("none.idle"), "music stops: no dance")
print("ok  per-state reactions and the dance")

-- Sleep after 120 s without agent activity, once the pointer is away
-- (desktop-logic.ts shouldSleep); hover or agent work wakes him.
run(100)
assert(shows("none.idle"), "awake before 120 s")
run(25)
assert(shows("none.sleeping"), "asleep after 120 s of no agent activity")
onMochiHover("true") run(0.1)
assert(shows("none.idle"), "hover wakes him")
onMochiHover("false")
run(122)
assert(shows("none.sleeping"), "asleep again")
noctalia.state.set("mochi", { state = "working", tool = "Bash", daemon = true, rev = 24 })
run(0.1)
assert(shows("none.working"), "agent work wakes him")
run(200)
assert(shows("none.working"), "never asleep while an agent works")
noctalia.state.set("mochi", { state = "idle", daemon = true, music = true, rev = 25 })
run(200)
assert(shows("none.dance"), "the music keeps him up")
noctalia.state.set("mochi", { state = "idle", daemon = true, rev = 26 })
run(118) onMochiClick() run(2)
assert(not shows("none.sleeping"), "a click counts as the pointer near him")
run(3)
assert(shows("none.sleeping"), "asleep once the pointer has been away 4 s")
print("ok  sleep after 120 s, wakes on hover and agent work")

-- First name (identity.rs first_name_from).
assert(firstNameFrom("Théodore Riant") == "Théodore")
assert(firstNameFrom("  Louis Raille ") == "Louis")
assert(firstNameFrom("Jean-Luc Picard") == "Jean-Luc")
assert(firstNameFrom("O'Brien Miles") == "O'Brien")
assert(firstNameFrom("Zoë") == "Zoë")
for _, bad in { "", "   ", "theodoreriant", "t.riant2", "Louis2", "louis_r", "me@example", "Admin_01", "R2-D2 Unit", "Dr. Who", "Louis " .. string.rep("a", 40) } do
  assert(firstNameFrom(bad) == nil, "no first name in " .. bad)
end
print("ok  first name rules")
onAgentsClick()
assert(lastCall() == "noctalia msg panel-open coucou/mochi:agents", "agents button opens the Agents panel")
print("ok  agents button")
-- A file sent to the chat: he swallows it once (the reaction already there is not replayed).
noctalia.state.set("mochi", { state = "idle", daemon = true, rev = 40, reaction = { name = "gulp", rev = 1 } })
run(0.2)
noctalia.state.set("mochi", { state = "idle", daemon = true, rev = 41, reaction = { name = "gulp", rev = 2 } })
run(0.2)
assert(shows(sprite.resolveOutfit(config.outfit) .. ".gulp"), "a new reaction plays the gulp")
run(2.5)
local gulpStart = {}
for i = 1, 5 do gulpStart[manifest[sprite.resolveOutfit(config.outfit) .. ".gulp"].frames[i][1]] = true end
noctalia.state.set("mochi", { state = "idle", daemon = true, rev = 42, reaction = { name = "gulp", rev = 2 } })
run(0.1)
assert(not gulpStart[image().props.path], "the same reaction is not replayed")
print("ok  gulp on a file for the chat")
onChatClick()
assert(lastCall() == "noctalia msg panel-toggle coucou/mochi:chat", "chat button toggles the chat panel")
print("ok  chat button")
print("DESKTOP PASSED")
