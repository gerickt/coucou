-- Mocked Noctalia host for the specs (see run.sh).
local clock = 0
local streamCb = nil
calls = {}
state = {}
local watchers = {}
played = {}
pendingSounds = {}
streams = {}
commands = { playerctl = true }
env = { USER = "tester" }
asyncReplies = {
  ["getent passwd tester"] = { exitCode = 0, stderr = "", stdout = "tester:x:1000:1000:Théodore Riant,,,:/home/tester:/bin/bash\n" },
}
config = { sound_enabled = true }
noctalia = {
  nowMs = function() return clock end,
  log = function(m) end,
  getConfig = function(k) return config[k] end,
  setUpdateInterval = function(ms) updateInterval = ms end,
  pluginDir = function() return "/plugin" end,
  pluginDataDir = function() return "/data" end,
  readFile = function(_p) return nil, "missing" end,
  runStream = function(cmd, cb)
    streams[cmd] = cb
    if cmd:find("--follow", 1, true) then streamCb = cb end
    table.insert(calls, cmd)
    return true
  end,
  commandExists = function(name) return commands[name] == true end,
  runAsync = function(argv, cb)
    local cmd = if type(argv) == "table" then table.concat(argv, " ") else argv
    table.insert(calls, cmd)
    if cb then cb(asyncReplies[cmd] or { exitCode = 0, stderr = "", stdout = "" }) end
    return true
  end,
  getenv = function(k) return env[k] end,
  -- Like the host: at most eight loads pending; flushSounds() finishes them.
  sound = {
    load = function(n, p, cb)
      if #pendingSounds >= 8 then return false end
      table.insert(pendingSounds, cb)
      return true
    end,
    play = function(n) table.insert(played, n) end,
  },
  state = {
    set = function(k, v) state[k] = v; if watchers[k] then watchers[k](v) end end,
    get = function(k) return state[k] end,
    watch = function(k, f) watchers[k] = f end,
  },
  json = {},
}
function advance(ms) clock += ms end
function flushSounds()
  while #pendingSounds > 0 do table.remove(pendingSounds, 1)(true) end
end
function feed(line) streamCb(line) end
function lastCall() return calls[#calls] end
