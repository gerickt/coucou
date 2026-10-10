-- Menu spec: Wardrobe / Agents / Chat / Settings.
onOpen(nil) validate(rendered)
local handlers = { onWardrobe = onWardrobe, onAgents = onAgents, onChat = onChat, onSettings = onSettings }
for handler, expect in { onWardrobe = "noctalia msg panel-open coucou/mochi:wardrobe", onAgents = "noctalia msg panel-open coucou/mochi:agents",
  onChat = "noctalia msg panel-open coucou/mochi:chat", onSettings = "noctalia msg settings-open-plugin coucou/mochi" } do
  assert(find(rendered, function(t) return t.type == "button" and t.props.onClick == handler end), handler .. " has a button")
  panelClosed = false
  handlers[handler]()
  assert(panelClosed and lastCall() == expect, handler .. " closes and opens " .. expect)
end
print("MENU PASSED")
