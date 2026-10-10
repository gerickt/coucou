--!nonstrict
-- The [[shortcut]] tile "mute" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.mute", icon = "volume-off", event="mute", active = function(s) return s.muted == true end })
function onClick() click() end
