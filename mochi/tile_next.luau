--!nonstrict
-- The [[shortcut]] tile "next" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.next", icon = "chevron-right", event="next" })
function onClick() click() end
