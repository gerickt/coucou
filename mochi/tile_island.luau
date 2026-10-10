--!nonstrict
-- The [[shortcut]] tile "island" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.island", icon = "layout-dashboard", event="island" })
function onClick() click() end
