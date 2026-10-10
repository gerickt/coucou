--!nonstrict
-- The [[shortcut]] tile "desktop" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.desktop", icon = "device-desktop", event="desktop", active = function(s) return s.away == true end })
function onClick() click() end
