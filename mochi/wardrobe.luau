--!nonstrict
-- The wardrobe's "auto" outfit (port of windows/src/mochi/wardrobe.ts), shared
-- by sprite.luau (the widgets) and service.luau (it tells the coucou-mochi
-- overlay what Mochi wears today). No frames needed here.

local M = {}

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

return M
