--[[
  Add a standard (non-prioritized, non-delayed) job.

  KEYS[1] = wait list
  KEYS[2] = meta hash
  KEYS[3] = id counter key
  KEYS[4] = events stream
  KEYS[5] = marker sorted set
  KEYS[6] = stalled set
  KEYS[7] = job hash key (prefix:queueName:jobId)
  KEYS[8] = active list
  KEYS[9] = completed sorted set

  ARGV[1] = name
  ARGV[2] = data (JSON string)
  ARGV[3] = timestamp
  ARGV[4] = jobId
  ARGV[5] = opts (JSON string)
  ARGV[6] = maxEvents

  Returns: jobId

  Ported from BullMQ (stripped: parent/flow, dedup, repeat, debounce).
]]
local rcall = redis.call

--@include "storeJob"
--@include "addBaseMarkerIfNeeded"

local waitKey = KEYS[1]
local metaKey = KEYS[2]
-- local idKey = KEYS[3]
local eventsKey = KEYS[4]
local markerKey = KEYS[5]
-- local stalledKey = KEYS[6]
local jobIdKey = KEYS[7]

local name = ARGV[1]
local data = ARGV[2]
local timestamp = ARGV[3]
local jobId = ARGV[4]
local opts = cjson.decode(ARGV[5])
local maxEvents = tonumber(ARGV[6]) or 10000

-- Idempotent: if job hash already exists, return the jobId
if rcall("EXISTS", jobIdKey) == 1 then
  return jobId
end

local delay, priority = storeJob(eventsKey, jobIdKey, jobId, name, data, opts, timestamp)

-- Check if paused
local paused = rcall("HEXISTS", metaKey, "paused") == 1

-- Push to wait list or paused list depending on queue state
local targetKey = waitKey
if paused then
  -- Derive paused key: replace ":wait" suffix with ":paused"
  targetKey = string.gsub(waitKey, ":wait$", ":paused")
end
rcall("LPUSH", targetKey, jobId)

-- Emit waiting event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
      "event", "waiting", "jobId", jobId, "prev", "")

-- Signal workers if not paused
if not paused then
  addBaseMarkerIfNeeded(markerKey)
end

return jobId
