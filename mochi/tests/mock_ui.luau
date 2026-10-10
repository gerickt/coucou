-- Mocked ui.* builders and tree checks for the specs.
local function node(kind) return function(props, children) return { type = kind, props = props or {}, children = children or {} } end end
ui = {}
for _, k in { "column", "row", "scroll", "box", "label", "glyph", "image", "spacer", "button", "separator", "progress", "markdown", "toggle", "input", "select" } do ui[k] = node(k) end
rendered = nil
frameTick = false
desktopWidget = { render = function(t) rendered = t end, setNeedsFrameTick = function(on) frameTick = on end }
barWidget = { render = function(t) rendered = t end, setTooltip = function(t) tooltip = t end }
tile = {}
shortcut = {
  setLabel = function(t) tile.label = t end,
  setIcon = function(on) tile.icon = on end,
  setActive = function(a) tile.active = a end,
  setEnabled = function(e) tile.enabled = e end,
}
panelClosed = false
panel = { render = function(t) rendered = t end, close = function() panelClosed = true end }
noctalia.tr = function(k, a) return k end
noctalia.togglePanel = function(id) table.insert(calls, "toggle " .. id) end
noctalia.notifyError = function() end
local function enc(v)
  if type(v) == "string" then return '"' .. v .. '"' end
  if type(v) == "table" then
    if #v > 0 then local o = {} for _, x in v do table.insert(o, enc(x)) end return "[" .. table.concat(o, ",") .. "]" end
    local o = {} for k, x in v do table.insert(o, '"' .. k .. '":' .. enc(x)) end return "{" .. table.concat(o, ",") .. "}"
  end
  return tostring(v)
end
noctalia.json.encode = enc
-- Walks a tree and checks every numeric prop is a finite, non-negative number.
function validate(t, path)
  path = path or t.type
  for k, v in t.props do
    if type(v) == "number" and (v ~= v or v == math.huge or (v < 0 and k ~= "x")) then error("bad " .. k .. "=" .. v .. " at " .. path) end
  end
  for i, c in t.children do validate(c, path .. "/" .. c.type .. i) end
end
function find(t, pred) if pred(t) then return t end for _, c in t.children do local r = find(c, pred) if r then return r end end end
