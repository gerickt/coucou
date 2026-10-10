--!nonstrict
-- Session pills as the original island draws them (windows/src/island/agents.ts,
-- views.ts buildPill): each agent's name and colour, the badge colours, and
-- toOneLine (core/diff.ts), which turns an agent's last answer into the line
-- the finished card shows. Shared by service.luau, panel.luau and bar.luau.

local M = {}

-- Ids, names and colours of the original's PillCatalog (pill `agent_<id>`);
-- Claude Code's own pill is integration_claude (#F5F6F8).
M.AGENTS = {
  [""] = { name = "Claude Code", color = "#F5F6F8" },
  ["claude-desktop"] = { name = "Claude Desktop", color = "#D97757" },
  cursor = { name = "Cursor", color = "#C0C4CC" },
  antigravity = { name = "Antigravity", color = "#E879F9" },
  codex = { name = "Codex", color = "#2DD4BF" },
  gemini = { name = "Gemini CLI", color = "#8AB4F8" },
  copilot = { name = "Copilot CLI", color = "#818CF8" },
  muse = { name = "Muse Code", color = "#38BDF8" },
  opencode = { name = "OpenCode", color = "#4ADE80" },
  amp = { name = "Amp", color = "#F59E0B" },
  hermes = { name = "Hermes", color = "#C084FC" },
}

local FALLBACK_COLORS = { "#22C55E", "#EAB308", "#60A5FA", "#E879F9" }

-- The pill badge (views.ts buildPill): approval, finished, error.
M.BADGES = { approval = "#F5A524", finished = "#22C55E", error = "#F4505E" }
M.BADGE_GLYPHS = { approval = "exclamation-mark", finished = "check", error = "x" }

function M.agentName(id)
  local known = M.AGENTS[id or ""]
  if known then return known.name end
  return tostring(id)
end

--- agents.ts agentColor: the catalog's colour, else one of four by a hash of the id.
function M.agentColor(id)
  id = id or ""
  local known = M.AGENTS[id]
  if known then return known.color end
  local h = 0
  for i = 1, #id do
    h = (31 * h + string.byte(id, i)) % 4294967296
    if h >= 2147483648 then h -= 4294967296 end
  end
  return FALLBACK_COLORS[(math.abs(h) % #FALLBACK_COLORS) + 1]
end

--- A colour with an alpha byte, as the pill's tints ("#RRGGBB" .. "24").
function M.tint(color, alphaHex)
  if type(color) ~= "string" or not color:match("^#%x%x%x%x%x%x$") then return color end
  return color .. alphaHex
end

local function trim(s) return (s:gsub("^[ \t]+", ""):gsub("[ \t]+$", "")) end

--- core/diff.ts toOneLine: the first paragraph with words, markdown marks and
--- list bullets taken off, whitespace collapsed, at most `maxChars` characters.
function M.toOneLine(text, maxChars)
  if type(text) ~= "string" then return "" end
  maxChars = maxChars or 200
  local paragraphs = {}
  local current = {}
  for line in (text:gsub("\r\n", "\n") .. "\n"):gmatch("([^\n]*)\n") do
    local t = trim(line)
    local isHR = #t >= 3 and (t:match("^%-+$") or t:match("^%*+$") or t:match("^_+$"))
    if t == "" or isHR or t:sub(1, 1) == "|" then
      if #current > 0 then table.insert(paragraphs, current) end
      current = {}
    else
      table.insert(current, line)
    end
  end
  if #current > 0 then table.insert(paragraphs, current) end
  for _, para in paragraphs do
    local s = table.concat(para, "\n"):gsub("%*%*", ""):gsub("__", ""):gsub("`", "")
    local processed = {}
    for line in (s .. "\n"):gmatch("([^\n]*)\n") do
      local l = trim((line:gsub("^#+", "")))
      if l:sub(1, 2) == "- " or l:sub(1, 2) == "* " then
        l = l:sub(3)
      elseif l:sub(1, 4) == "• " then
        l = l:sub(5)
      else
        l = l:gsub("^%d+%.%s+", "")
      end
      l = trim(l)
      if l ~= "" then table.insert(processed, l) end
    end
    local words = {}
    for w in table.concat(processed, " "):gmatch("%S+") do table.insert(words, w) end
    local collapsed = table.concat(words, " ")
    if collapsed ~= "" then
      -- Cut on characters, not bytes.
      local n = utf8.len(collapsed)
      if n and n > maxChars then collapsed = collapsed:sub(1, utf8.offset(collapsed, maxChars + 1) - 1) end
      return collapsed
    end
  end
  return ""
end

return M
