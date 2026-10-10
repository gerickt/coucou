-- Bar spec: renders the mini Mochi in a few states.
for _, st in { "idle", "working", "approval", "finished", "sleeping" } do
  noctalia.state.set("mochi", { state = st, tool = "Edit", daemon = true, rev = math.random(1, 1e6), pending = st == "approval" and { id = "1-1" } or nil, project = "p" })
  for i = 1, 10 do update() validate(rendered) end
end
onClick()
assert(calls[#calls] == "toggle coucou/mochi:approval")
print("BAR PASSED")
