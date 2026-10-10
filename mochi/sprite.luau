--!nonstrict
-- Shared by the desktop and bar entries: plays Mochi's frame sequences.
--
-- The frames in frames/ are the ORIGINAL Mochi: tools/render-mochi drives the
-- original engine (windows/src/mochi/engine.ts + outfits.ts at 081a2ad,
-- unmodified) in headless Chrome and saves every frame as a PNG. Noctalia has
-- no canvas, so the widgets only swap ui.image paths at each sequence's fps.
--
-- Also the wardrobe's "auto" outfit (port of windows/src/mochi/wardrobe.ts).

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

-- ── Wardrobe (wardrobe.ts) ───────────────────────────────────────────────────

M.OUTFITS = {
  "auto", "none", "partyHat", "beanie", "crown", "sunglasses", "roundGlasses",
  "bow", "scarf", "witchHat", "pumpkin", "santaHat", "bunnyEars",
}

local known = {}
for _, o in M.OUTFITS do known[o] = true end

--- Easter Sunday of `year` (Meeus/Jones/Butcher) as month, day.
function M.easter(year)
  local a = year % 19
  local b = year // 100
  local c = year % 100
  local d = b // 4
  local e = b % 4
  local f = (b + 8) // 25
  local g = (b - f + 1) // 3
  local h = (19 * a + b - d - g + 15) % 30
  local i = c // 4
  local k = c % 4
  local l = (32 + 2 * e + 2 * i - h - k) % 7
  local m = (a + 11 * h + 22 * l) // 451
  local month = (h + l - 7 * m + 114) // 31
  local day = ((h + l - 7 * m + 114) % 31) + 1
  return month, day
end

local function dayNumber(y, m, d)
  -- Days since 1970-01-01 of a calendar date (no time zone involved).
  local a = (14 - m) // 12
  local yy = y + 4800 - a
  local mm = m + 12 * a - 3
  return d + (153 * mm + 2) // 5 + 365 * yy + yy // 4 - yy // 100 + yy // 400 - 32045 - 2440588
end

--- The seasonal outfit for a local calendar date.
--- Priority: party hat > Santa hat > witch hat > bunny ears > sunglasses > none.
function M.seasonalOutfit(year, month, day)
  if (month == 12 and day == 31) or (month == 1 and day <= 2) then return "partyHat" end
  if month == 12 and day <= 26 then return "santaHat" end
  if month == 10 or (month == 11 and day == 1) then return "witchHat" end
  local em, ed = M.easter(year)
  local delta = dayNumber(year, month, day) - dayNumber(year, em, ed)
  if delta >= -2 and delta <= 1 then return "bunnyEars" end
  if (month == 6 and day >= 21) or month == 7 or month == 8 then return "sunglasses" end
  return "none"
end

--- A stored selection -> what Mochi wears today. Unknown values mean "auto".
function M.resolveOutfit(selection)
  if type(selection) ~= "string" or not known[selection] then selection = "auto" end
  if selection ~= "auto" then return selection end
  local ok, t = pcall(os.date, "*t")
  if not ok or type(t) ~= "table" then return "none" end
  return M.seasonalOutfit(t.year, t.month, t.day)
end

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
