-- Mocked Noctalia host for the specs (see run.sh).
local clock = 0
local streamCb = nil
calls = {}
state = {}
local watchers = {}
played = {}
config = { sound_enabled = true }
noctalia = {
  nowMs = function() return clock end,
  log = function(m) end,
  getConfig = function(k) return config[k] end,
  setUpdateInterval = function(ms) updateInterval = ms end,
  pluginDir = function() return "/plugin" end,
  runStream = function(cmd, cb) streamCb = cb; table.insert(calls, cmd); return true end,
  runAsync = function(argv, cb) table.insert(calls, table.concat(argv, " ")); if cb then cb({ exitCode = 0, stderr = "" }) end; return true end,
  sound = { load = function(n, p, cb) cb(true) end, play = function(n) table.insert(played, n) end },
  state = {
    set = function(k, v) state[k] = v; if watchers[k] then watchers[k](v) end end,
    get = function(k) return state[k] end,
    watch = function(k, f) watchers[k] = f end,
  },
  json = {},
}
function advance(ms) clock += ms end
function feed(line) streamCb(line) end
function lastCall() return calls[#calls] end
