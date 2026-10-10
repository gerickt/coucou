-- Tile spec: mute asks the service and shows its state.
assert(tile.label == "shortcut.mute" and tile.icon == "volume-off", "tile labelled")
onClick()
assert(state["mochi.cmd"].event == "mute", "click asks the service to mute")
local first = state["mochi.cmd"].seq
onClick()
assert(state["mochi.cmd"].seq == first + 1, "a repeat is a new command")
noctalia.state.set("mochi", { muted = true })
assert(tile.active == true, "tile active while muted")
noctalia.state.set("mochi", { muted = false })
assert(tile.active == false, "and off again")
print("ok  mute tile")
print("TILE_MUTE PASSED")
