-- Shortcut tile spec: a labelled tile that opens the chat.
assert(tile.label == "shortcut.chat" and tile.icon == "message-circle", "tile labelled")
onClick()
assert(lastCall() == "toggle coucou/mochi:chat", "tile toggles the chat panel")
print("ok  tile opens the chat")
print("SHORTCUT PASSED")
