--!nonstrict
-- The user's first name for the greeting (port of identity.rs first_name_from),
-- from the passwd GECOS field. Shared by desktop.luau and service.luau (which
-- hands the greeting to the coucou-mochi overlay).

local M = {}

local function isHandle(name)
  if name:find(" ", 1, true) then return false end
  return name:lower() == name or name:find("[%d%._%-@]") ~= nil
end

--- "Théodore Riant" -> "Théodore"; a login handle or an odd name -> nil.
function M.firstNameFrom(full)
  if type(full) ~= "string" then return nil end
  full = full:match("^%s*(.-)%s*$")
  local count = utf8.len(full) or #full
  if full == "" or count > 32 or isHandle(full) then return nil end
  local first = full:match("^([^ ]+)")
  -- Letters (any non-ASCII byte counts as part of a letter), ' and -.
  if not first or first:find("[^%a\128-\255'%-]") then return nil end
  return first
end

--- Asks `getent passwd $USER` (argv, no shell); `done(firstName or nil)`.
function M.read(host, done)
  local user = host.getenv and host.getenv("USER")
  if type(user) ~= "string" or not user:match("^[%w%._%-]+$") then return end
  host.runAsync({ "getent", "passwd", user }, function(res)
    if res.exitCode ~= 0 or type(res.stdout) ~= "string" then return end
    -- name:password:uid:gid:GECOS:home:shell; the full name is GECOS up to a comma.
    local gecos = res.stdout:match("^[^:\n]*:[^:\n]*:[^:\n]*:[^:\n]*:([^:\n]*)")
    if not gecos then return end
    done(M.firstNameFrom(gecos:match("^([^,]*)")))
  end, 2000)
end

return M
