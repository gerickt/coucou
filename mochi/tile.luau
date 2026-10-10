--!nonstrict
-- Quick-settings tiles for the original's global shortcuts. Each [[shortcut]]
-- entry is a few lines on top of this: a label, an icon and what a click does.
-- A click is an event for the service ("mochi.cmd"), or a panel toggle. For a
-- key, bind it in Hyprland (README): noctalia msg plugin coucou/mochi:daemon all <event>
-- A required module does not see the entry's globals: the host and the tile
-- object are passed in.

local M = {}

local seq = 0

local function send(host, event)
  seq += 1
  host.state.set("mochi.cmd", { event = event, payload = "", seq = seq })
end

--- opts: labelKey, icon, event | panel, active (function(snap) -> bool)?
--- Returns the click handler for the entry's `onClick`.
function M.setup(host, tile, opts)
  tile.setLabel(host.tr(opts.labelKey))
  tile.setIcon(opts.icon)
  if opts.active then
    local function show(v)
      if type(v) == "table" then tile.setActive(opts.active(v) == true) end
    end
    host.state.watch("mochi", show)
    show(host.state.get("mochi"))
  end
  return function()
    if opts.panel then
      host.togglePanel(opts.panel)
    else
      send(host, opts.event)
    end
  end
end

return M
