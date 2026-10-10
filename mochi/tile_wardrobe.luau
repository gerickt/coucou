--!nonstrict
-- The [[shortcut]] tile "wardrobe" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.wardrobe", icon = "hanger", panel="coucou/mochi:wardrobe" })
function onClick() click() end
