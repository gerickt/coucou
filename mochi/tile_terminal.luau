--!nonstrict
-- The [[shortcut]] tile "terminal" (see tile.luau).
local okTile, tileLib = pcall(require, "./tile.luau")
if not okTile then tileLib = require("./tile") end
local click = tileLib.setup(noctalia, shortcut, { labelKey = "shortcut.terminal", icon = "terminal-2", event="terminal" })
function onClick() click() end
