--[[
  Function to store a job's hash fields in Redis.

  Ported from BullMQ (stripped: parent/dependency, dedup, debounce, repeat).
]]
local function storeJob(eventsKey, jobIdKey, jobId, name, data, opts, timestamp)
  local jsonOpts = cjson.encode(opts)
  local delay = opts['delay'] or 0
  local priority = opts['priority'] or 0

  rcall("HMSET", jobIdKey, "name", name, "data", data, "opts", jsonOpts,
        "timestamp", timestamp, "delay", delay, "priority", priority,
        "atm", 0, "ats", 0)

  rcall("XADD", eventsKey, "*", "event", "added", "jobId", jobId, "name", name)

  return delay, priority
end
