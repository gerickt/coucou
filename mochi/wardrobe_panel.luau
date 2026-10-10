--!nonstrict
-- The wardrobe, a [[panel]] entry (views/wardrobe.ts): a grid of the outfits,
-- each a frame of Mochi wearing it. Resting the pointer on one tries it on him
-- (the service publishes the preview; the widgets and the overlay wear it);
-- a click keeps it. Closing the panel drops the preview.

local okSprite, sprite = pcall(require, "./sprite.luau")
if not okSprite then sprite = require("./sprite") end
sprite.setup(noctalia)

local COLUMNS = 4
local ICON = 72

local snap = {}
local hovered = nil
local cmdSeq = 0

local function ask(event, payload)
  cmdSeq += 1
  noctalia.state.set("mochi.cmd", { event = event, payload = payload or "", seq = cmdSeq })
end

--- The first idle frame of an outfit (the "auto" tile shows today's outfit).
local function thumbnail(selection)
  local outfit = sprite.resolveOutfit(selection)
  local seq = sprite.manifest.desktop.sequences[outfit .. ".idle"]
  local f = seq and seq.frames and seq.frames[1]
  if not f then return nil end
  return sprite.path(f[1])
end

local function name(selection)
  return noctalia.tr("settings.outfit.options." .. selection)
end

local render

local function hover(selection, on)
  if on then
    hovered = selection
    ask("outfit-preview", sprite.resolveOutfit(selection))
  elseif hovered == selection then
    hovered = nil
    ask("outfit-preview", "")
  end
  render()
end

local function keep(selection)
  ask("outfit-keep", selection)
end

-- The header's note: what is tried on, else what he wears (wardrobeHeader).
local function note()
  if hovered then return name(hovered) end
  return noctalia.tr("wardrobe.wearing", { outfit = name(snap.outfit_choice or "auto") })
end

render = function()
  local current = snap.outfit_choice or "auto"
  local rows = {}
  local row = {}
  for _, sel in sprite.OUTFITS do
    local path = thumbnail(sel)
    local selected = sel == current
    local tile = ui.column({
      key = "o" .. sel, gap = 2, align = "center", padding = 6, radius = 10,
      fill = if selected then "primary/0.18" elseif hovered == sel then "surface_variant" else nil,
      border = if selected then "primary" else nil, borderWidth = if selected then 1 else 0,
      onClick = function() keep(sel) end,
      onHover = function(state) hover(sel, state == true or state == "true") end,
    }, {
      if path then ui.image({ path = path, width = ICON, height = ICON, fit = "contain" }) else ui.box({ width = ICON, height = ICON }),
      ui.label({ text = name(sel), fontSize = 10, color = "on_surface_variant", maxWidth = ICON + 8, maxLines = 1, textAlign = "center" }),
    })
    table.insert(row, tile)
    if #row == COLUMNS then
      table.insert(rows, ui.row({ gap = 6, justify = "center" }, row))
      row = {}
    end
  end
  if #row > 0 then table.insert(rows, ui.row({ gap = 6, justify = "center" }, row)) end
  panel.render(ui.column({ padding = 16, gap = 10, flexGrow = 1 }, {
    ui.row({ gap = 8, align = "center" }, {
      ui.label({ text = noctalia.tr("wardrobe.title"), fontSize = 16, fontWeight = "bold", color = "primary" }),
      ui.spacer({ flexGrow = 1 }),
      ui.label({ text = note(), fontSize = 12, color = "on_surface_variant", maxLines = 1, maxWidth = 220 }),
    }),
    ui.scroll({ flexGrow = 1, gap = 6 }, rows),
  }))
end

noctalia.state.watch("mochi", function(v)
  if type(v) == "table" then snap = v end
  render()
end)

function onOpen(_context)
  hovered = nil
  snap = noctalia.state.get("mochi") or {}
  render()
end

-- Trying one on ends with the panel.
function onClose()
  hovered = nil
  ask("outfit-preview", "")
end
