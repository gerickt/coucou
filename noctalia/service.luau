--!nonstrict
-- Service: tracks coucou daemon state and relays events to the Noctalia shell.

local COMMAND = "coucou-daemon --status-json"

local function refresh()
    noctalia.runAsync(COMMAND, function(result)
        if result ~= nil and result.exitCode == 0 and result.stdout ~= nil then
            local decoded = noctalia.json.decode(result.stdout)
            if type(decoded) == "table" then
                noctalia.state.set("mochi_state", decoded)
                return
            end
        end
        -- Fallback default state
        noctalia.state.set("mochi_state", {
            state = "idle",
            agent = "Claude Code",
            tool = nil,
            active = false
        })
    end, 2000)
end

noctalia.state.watch("command", function(value)
    if type(value) == "table" and value.action == "toggle-pause" then
        local current = noctalia.state.get("mochi_paused") or false
        noctalia.state.set("mochi_paused", not current)
    end
end)

function update()
    refresh()
end

noctalia.setUpdateInterval(1000)
refresh()
