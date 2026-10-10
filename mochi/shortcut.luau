--!nonstrict
-- The [[shortcut]] tile: opens Mochi's chat (Ctrl+Alt+Space in the original;
-- for a key, bind `noctalia msg panel-toggle coucou/mochi:chat` in Hyprland).

shortcut.setLabel(noctalia.tr("shortcut.chat"))
shortcut.setIcon("message-circle")

function onClick()
  noctalia.togglePanel("coucou/mochi:chat")
end
