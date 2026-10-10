--!nonstrict
-- The [[shortcut]] tile "prev" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.prev", icon = "chevron-left", event="prev" })
function onClick() click() end
