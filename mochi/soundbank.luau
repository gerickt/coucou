--!nonstrict
-- Loads a set of the plugin's sounds and plays them by name.
--
-- The host keeps at most eight loads pending and refuses the rest, so the
-- files load one after the other. A sound asked for before its file is
-- decoded (the launch greeting) plays as soon as it is, if that is soon.

local LATE_MS = 1500

local M = {}

--- `host` is the entry's `noctalia` (a required module does not see the entry's
--- globals); `enabled` reads the entry's sound setting.
function M.new(host, names, enabled)
  local loaded = {}
  local wanted = {} -- name -> when it was asked for
  local queue = table.clone(names)

  local function nextLoad()
    local name = table.remove(queue, 1)
    if not name then return end
    local ok = host.sound.load(name, "sounds/" .. name .. ".wav", function(done, err)
      loaded[name] = done
      if not done then host.log("mochi: sound " .. name .. " failed: " .. tostring(err)) end
      local at = wanted[name]
      wanted[name] = nil
      if done and at and host.nowMs() - at <= LATE_MS and enabled() then host.sound.play(name) end
      nextLoad()
    end)
    if not ok then
      host.log("mochi: sound " .. name .. " could not be queued")
      nextLoad()
    end
  end
  nextLoad()

  return function(name)
    if not name or not enabled() then return end
    if loaded[name] then
      host.sound.play(name)
    elseif loaded[name] == nil then
      wanted[name] = host.nowMs()
    end
  end
end

return M
