--!nonstrict
-- Shared by the desktop and bar entries: plays Mochi's frame sequences.
--
-- The frames in frames/ are the ORIGINAL Mochi: tools/render-mochi drives the
-- original engine (windows/src/mochi/engine.ts + outfits.ts at 081a2ad,
-- unmodified) in headless Chrome and saves every frame as a PNG. Noctalia has
-- no canvas, so the widgets only swap ui.image paths at each sequence's fps.
--
-- Also re-exports the wardrobe's "auto" outfit (wardrobe.luau).

local M = {}

local function loadManifest()
  local ok, m = pcall(require, "./frames/manifest.luau")
  if not ok or type(m) ~= "table" then ok, m = pcall(require, "./frames/manifest") end
  if ok and type(m) == "table" then return m, true end
  return { desktop = { px = 180, sequences = {} }, bar = { px = 64, sequences = {} } }, false
end

local manifestOk
M.manifest, manifestOk = loadManifest()

local dir = ""

--- Called once by the entry with its `noctalia` host (a required module does
--- not rely on the entry's globals): absolute frame paths and a log line.
function M.setup(host)
  local d = host.pluginDir
  if type(d) == "function" then d = d() end
  if type(d) == "string" and d ~= "" then dir = d .. "/" end
  if not manifestOk then host.log("mochi: frames/manifest.luau missing, run tools/render-mochi/render.sh") end
end

function M.path(file)
  return dir .. file
end

-- ── Wardrobe (wardrobe.luau) ─────────────────────────────────────────────────

local okWardrobe, wardrobe = pcall(require, "./wardrobe.luau")
if not okWardrobe then wardrobe = require("./wardrobe") end
M.OUTFITS = wardrobe.OUTFITS
M.easter = wardrobe.easter
M.seasonalOutfit = wardrobe.seasonalOutfit
M.resolveOutfit = wardrobe.resolveOutfit

-- ── Player ───────────────────────────────────────────────────────────────────

local Player = {}
Player.__index = Player

--- A player for one mode ("desktop" or "bar") of the manifest.
function M.player(mode)
  local set = M.manifest[mode] or { sequences = {} }
  return setmetatable({
    seqs = set.sequences,
    outfit = "none",
    key = nil, -- current sequence key "<outfit>.<name>"
    seq = nil,
    run = 1, -- index in seq.frames
    left = 0, -- seconds left on the current run
    loopName = "idle",
    onceName = nil,
    onDone = nil,
  }, Player)
end

function Player:has(name)
  return self.seqs[self.outfit .. "." .. name] ~= nil
end

function Player:lookup(name)
  return self.seqs[self.outfit .. "." .. name] or self.seqs["none." .. name]
end

function Player:start(name)
  local seq = self:lookup(name)
  if not seq then return false end
  self.key = self.outfit .. "." .. name
  self.seq = seq
  self.run = 1
  self.left = seq.frames[1][2] / seq.fps
  return true
end

--- The loop to show whenever no one-shot is playing.
function Player:setLoop(name)
  if name == self.loopName and (self.onceName or self.key == self.outfit .. "." .. name) then return end
  self.loopName = name
  if not self.onceName then self:start(name) end
end

--- A one-shot; `onDone` runs when it ends, then the loop resumes.
--- Returns false when the outfit has no such sequence.
function Player:once(name, onDone)
  if not self:has(name) then return false end
  self.onceName = name
  self.onDone = onDone
  self:start(name)
  return true
end

--- Drops the one-shot in flight (its body no longer matches), back to the loop.
function Player:cancel()
  if not self.onceName then return end
  self.onceName = nil
  self.onDone = nil
  self:start(self.loopName)
end

function Player:setOutfit(outfit)
  if outfit == self.outfit then return end
  self.outfit = outfit
  self.onceName = nil
  self.onDone = nil
  self:start(self.loopName)
end

function Player:frame()
  if not self.seq then self:start(self.loopName) end
  if not self.seq then return nil end
  return M.path(self.seq.frames[self.run][1])
end

--- Seconds until the picture changes (math.huge for a still).
function Player:due()
  if not self.seq then return math.huge end
  if #self.seq.frames == 1 and self.seq.loop then return math.huge end
  return math.max(0, self.left)
end

--- Advances by `secs`; returns true when the frame changed.
function Player:step(secs)
  if not self.seq then return self:start(self.loopName) end
  local before = self.seq.frames[self.run][1]
  local beforeKey = self.key
  self.left -= secs
  local guard = 0
  while self.left <= 0 and guard < 500 do
    guard += 1
    if self.run < #self.seq.frames then
      self.run += 1
      self.left += self.seq.frames[self.run][2] / self.seq.fps
    elseif self.seq.loop then
      self.run = 1
      self.left += self.seq.frames[1][2] / self.seq.fps
      if #self.seq.frames == 1 then self.left = math.max(self.left, 0.001) break end
    else
      -- One-shot finished: back to the loop, then tell the caller.
      local done = self.onDone
      self.onceName = nil
      self.onDone = nil
      self:start(self.loopName)
      self.left = math.max(self.left, 0.001)
      if done then done() end
      break
    end
  end
  return self.key ~= beforeKey or (self.seq and self.seq.frames[self.run][1] ~= before)
end

return M
