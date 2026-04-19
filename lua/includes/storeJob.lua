--[[
  Function to store a job's hash fields in Redis.

  Ported from BullMQ (stripped: parent/dependency, dedup, debounce, repeat).
]]
local function storeJob(eventsKey, jobIdKey, jobId, name, data, opts, timestamp, parentKey, parentData)
  local jsonOpts = cjson.encode(opts)
  local delay = opts['delay'] or 0
  local priority = opts['priority'] or 0

  rcall("HMSET", jobIdKey, "name", name, "data", data, "opts", jsonOpts,
        "timestamp", timestamp, "delay", delay, "priority", priority,
        "atm", 0, "ats", 0)

  if parentKey ~= nil and parentKey ~= "" then
    rcall("HSET", jobIdKey, "parentKey", parentKey)
  end

  if parentData ~= nil and parentData ~= "" then
    rcall("HSET", jobIdKey, "parent", parentData)
  end

  rcall("XADD", eventsKey, "*", "event", "added", "jobId", jobId, "name", name)

  return delay, priority
end
