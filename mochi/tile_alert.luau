--!nonstrict
-- The [[shortcut]] tile "alert" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.alert", icon = "alert-circle", event="alert" })
function onClick() click() end
