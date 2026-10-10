-- Wardrobe spec: a grid of thumbnails, hover previews, click keeps.
onOpen(nil) validate(rendered)
noctalia.state.set("mochi", { outfit_choice = "bow", daemon = true, rev = 1 })
validate(rendered)
local function tileOf(sel) return find(rendered, function(t) return t.props.key == "o" .. sel end) end
for _, sel in sprite.OUTFITS do
  local t = tileOf(sel)
  assert(t, "tile for " .. sel)
  assert(find(t, function(n) return n.type == "image" and n.props.path:find("/plugin/frames/", 1, true) end), "thumbnail for " .. sel)
end
assert(tileOf("bow").props.border == "primary" and tileOf("crown").props.border == nil, "the kept outfit is marked")
tileOf("crown").props.onHover("true")
assert(state["mochi.cmd"].event == "outfit-preview" and state["mochi.cmd"].payload == "crown", "hover tries it on")
tileOf("crown").props.onHover("false")
assert(state["mochi.cmd"].event == "outfit-preview" and state["mochi.cmd"].payload == "", "leaving drops the preview")
tileOf("auto").props.onHover("true")
assert(state["mochi.cmd"].payload == sprite.resolveOutfit("auto"), "auto previews today's outfit")
tileOf("scarf").props.onClick()
assert(state["mochi.cmd"].event == "outfit-keep" and state["mochi.cmd"].payload == "scarf", "click keeps it")
onClose()
assert(state["mochi.cmd"].event == "outfit-preview" and state["mochi.cmd"].payload == "", "closing drops the preview")
print("WARDROBE PASSED")
