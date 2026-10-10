-- Bar spec: the mini Mochi plays the bar loops of the original engine's frames.
local manifest = sprite.manifest.bar.sequences
local function image() return find(rendered, function(n) return n.type == "image" end) end
local function shows(key)
  local img = image()
  if not img then return false end
  for _, f in manifest[key].frames do if "/plugin/" .. f[1] == img.props.path then return true end end
  return false
end
for _, outfit in sprite.OUTFITS do
  if outfit ~= "auto" then
    for _, st in { "idle", "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping", "dance" } do
      assert(manifest[outfit .. "." .. st], "missing bar sequence " .. outfit .. "." .. st)
    end
  end
end
config.outfit = "none"
for _, st in { "idle", "working", "approval", "finished", "sleeping" } do
  noctalia.state.set("mochi", { state = st, tool = "Edit", daemon = true, rev = math.random(1, 1e6), pending = st == "approval" and { id = "1-1" } or nil, project = "p" })
  for i = 1, 10 do advance(1100) update() validate(rendered) end
  assert(shows("none." .. st), "bar shows " .. st)
  if st == "approval" then assert(find(rendered, function(n) return n.type == "box" end), "pending dot") end
  assert(updateInterval >= 50 and updateInterval <= 400, "update interval follows the frames")
end
config.outfit = "santaHat"
for i = 1, 3 do advance(1100) update() end
noctalia.state.set("mochi", { state = "working", tool = "Edit", daemon = true, rev = 7 })
for i = 1, 3 do advance(100) update() end
assert(shows("santaHat.working"), "bar wears the outfit")
noctalia.state.set("mochi", { state = "idle", daemon = true, music = true, rev = 8 })
for i = 1, 3 do advance(100) update() end
assert(shows("santaHat.dance"), "the bar dances while music plays")
noctalia.state.set("mochi", { state = "approval", daemon = true, music = true, pending = { id = "1-1" }, rev = 9 })
for i = 1, 3 do advance(100) update() end
assert(shows("santaHat.approval"), "an alert wins over the music")
noctalia.state.set("mochi", { state = "working", daemon = true, rev = 20, sessions = {
  { id = "a", agent = "codex", color = "#2DD4BF", badge = "finished" }, { id = "b", agent = "", color = "#F5F6F8" } } })
for i = 1, 2 do advance(100) update() end
local chips = {}
find(rendered, function(n) if n.type == "box" and n.props.width == 8 then table.insert(chips, n) end return false end)
assert(#chips == 2 and chips[1].props.fill == "#2DD4BF" and chips[1].props.border == "#22C55E" and chips[2].props.fill == "#F5F6F8", "one chip per session in its colour, ringed by the badge")
print("ok  session chips")
onClick()
assert(calls[#calls] == "toggle coucou/mochi:approval")
print("BAR PASSED")
