-- Minimal JSON decoder for the harness.
local function decode(s)
  local i = 1
  local function ws() while s:sub(i,i):match("%s") do i += 1 end end
  local value
  local function str()
    i += 1; local out = ""
    while true do
      local c = s:sub(i,i)
      if c == '"' then i += 1; return out end
      if c == "\\" then local n = s:sub(i+1,i+1); out ..= (n == "n" and "\n" or n); i += 2 else out ..= c; i += 1 end
    end
  end
  function value()
    ws(); local c = s:sub(i,i)
    if c == "{" then i += 1; local t = {}; ws(); if s:sub(i,i) == "}" then i += 1; return t end
      while true do ws(); local k = str(); ws(); i += 1; t[k] = value(); ws(); local d = s:sub(i,i); i += 1; if d == "}" then return t end end
    elseif c == "[" then i += 1; local t = {}; ws(); if s:sub(i,i) == "]" then i += 1; return t end
      while true do table.insert(t, value()); ws(); local d = s:sub(i,i); i += 1; if d == "]" then return t end end
    elseif c == '"' then return str()
    elseif s:sub(i,i+3) == "true" then i += 4; return true
    elseif s:sub(i,i+4) == "false" then i += 5; return false
    elseif s:sub(i,i+3) == "null" then i += 4; return nil
    else local n = s:match("^-?[%d%.eE+-]+", i); i += #n; return tonumber(n) end
  end
  return value()
end
noctalia.json.decode = decode
-- Minimal JSON encoder (the UI mocks keep their own).
local function encode(v)
  local t = type(v)
  if t == "string" then
    return '"' .. v:gsub('[\\"]', "\\%0"):gsub("\n", "\\n") .. '"'
  elseif t == "table" then
    local out = {}
    if #v > 0 then
      for _, x in v do table.insert(out, encode(x)) end
      return "[" .. table.concat(out, ",") .. "]"
    end
    local keys = {}
    for k in v do table.insert(keys, k) end
    table.sort(keys)
    for _, k in keys do table.insert(out, encode(k) .. ":" .. encode(v[k])) end
    return "{" .. table.concat(out, ",") .. "}"
  end
  return tostring(v)
end
noctalia.json.encode = encode
