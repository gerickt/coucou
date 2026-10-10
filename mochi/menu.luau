--!nonstrict
-- The small menu the overlay's right click opens: Wardrobe, Agents, Chat,
-- Settings (the original opened the wardrobe straight from a right click; the
-- other doors it had are listed here too).

local PANELS = {
  wardrobe = "coucou/mochi:wardrobe",
  agents = "coucou/mochi:agents",
  chat = "coucou/mochi:chat",
}

local function open(id)
  panel.close()
  noctalia.runAsync({ "noctalia", "msg", "panel-open", id }, nil)
end

function onWardrobe() open(PANELS.wardrobe) end
function onAgents() open(PANELS.agents) end
function onChat() open(PANELS.chat) end

function onSettings()
  panel.close()
  noctalia.runAsync({ "noctalia", "msg", "settings-open-plugin", "coucou/mochi" }, nil)
end

local function item(glyph, key, handler)
  return ui.button({ text = noctalia.tr(key), glyph = glyph, variant = "ghost", contentAlign = "start", onClick = handler })
end

function onOpen(_context)
  panel.render(ui.column({ padding = 8, gap = 2, flexGrow = 1 }, {
    item("hanger", "menu.wardrobe", "onWardrobe"),
    item("plug-connected", "menu.agents", "onAgents"),
    item("message-circle", "menu.chat", "onChat"),
    item("settings", "menu.settings", "onSettings"),
  }))
end
