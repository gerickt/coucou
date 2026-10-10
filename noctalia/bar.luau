--!nonstrict
-- Bar capsule for Mochi.
-- Seamlessly integrates Mochi into Noctalia's bar alongside other capsules.

local showLabel = noctalia.getConfig("show_label") == true
local showTool = noctalia.getConfig("show_tool") ~= false

local mochiState = {
    state = "idle",
    agent = "Claude Code",
    tool = nil,
    active = false
}

local function getEmoteGlyph(state, isPaused)
    if isPaused then return "󰒲" end -- sleeping / paused
    if state == "permission" then return "󰚌" end -- alert / approval needed
    if state == "thinking" then return "󰏤" end -- thinking
    if state == "running" then return "󱚠" end -- working/tool
    if state == "done" then return "󰸞" end -- happy/done
    return "󰚩" -- default friendly bot face
end

local function getEmoteColor(state, isPaused)
    if isPaused then return "outline" end
    if state == "permission" then return "error" end
    if state == "thinking" then return "tertiary" end
    if state == "running" then return "primary" end
    if state == "done" then return "secondary" end
    return "primary"
end

local function render()
    local isPaused = noctalia.state.get("mochi_paused") == true
    local state = tostring(mochiState.state or "idle")
    local color = getEmoteColor(state, isPaused)
    local glyph = getEmoteGlyph(state, isPaused)

    local children = {}

    -- Mochi icon / avatar
    children[#children + 1] = ui.label({
        key = "mochi-face",
        text = glyph,
        fill = color,
    })

    -- Optional tool or label
    if showTool and mochiState.tool and mochiState.tool ~= "" then
        children[#children + 1] = ui.label({
            key = "mochi-tool",
            text = tostring(mochiState.tool),
            fill = "on_secondary",
        })
    elseif showLabel then
        children[#children + 1] = ui.label({
            key = "mochi-name",
            text = "Mochi",
            fill = "on_secondary",
        })
    end

    barWidget.render(ui.row({
        gap = 6,
        align = "center",
    }, children))
end

noctalia.state.watch("mochi_state", function(value)
    if type(value) == "table" then
        mochiState = value
        render()
    end
end)

noctalia.state.watch("mochi_paused", function()
    render()
end)

function tooltip()
    local isPaused = noctalia.state.get("mochi_paused") == true
    local statusText = isPaused and noctalia.tr("ui.paused") or noctalia.tr("ui." .. tostring(mochiState.state or "idle"))
    local rows = {
        { key = "Agent", value = tostring(mochiState.agent or "Claude Code") },
        { key = "Status", value = statusText },
    }
    if mochiState.tool then
        rows[#rows + 1] = { key = "Tool", value = tostring(mochiState.tool) }
    end
    return rows
end

render()
