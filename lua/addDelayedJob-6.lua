--[[
  Add a delayed job.

  KEYS[1] = delayed sorted set
  KEYS[2] = meta hash
  KEYS[3] = id counter key
  KEYS[4] = events stream
  KEYS[5] = marker sorted set
  KEYS[6] = job hash key (prefix:queueName:jobId)

  ARGV[1] = name
  ARGV[2] = data (JSON string)
  ARGV[3] = timestamp
  ARGV[4] = jobId
  ARGV[5] = opts (JSON string)
  ARGV[6] = maxEvents
  ARGV[7] = delayedTimestamp (the target timestamp when the job should run)
  ARGV[8] = parentKey (optional)
  ARGV[9] = parent (optional)

  Returns: jobId

  Ported from BullMQ (stripped: parent/flow, dedup, repeat, debounce).
]]
local rcall = redis.call

--@include "storeJob"
--@include "getDelayedScore"
--@include "addDelayMarkerIfNeeded"

local delayedKey = KEYS[1]
local metaKey = KEYS[2]
-- local idKey = KEYS[3]
local eventsKey = KEYS[4]
local markerKey = KEYS[5]
local jobIdKey = KEYS[6]

local name = ARGV[1]
local data = ARGV[2]
local timestamp = ARGV[3]
local jobId = ARGV[4]
local opts = cjson.decode(ARGV[5])
local maxEvents = tonumber(ARGV[6]) or 10000
local delayedTimestamp = tonumber(ARGV[7])
local parentKey = ARGV[8]
local parentData = ARGV[9]

if parentKey == "" then
  parentKey = nil
end
if parentData == "" then
  parentData = nil
end

-- Idempotent: if job hash already exists, return the jobId
if rcall("EXISTS", jobIdKey) == 1 then
  return jobId
end

local delay = opts['delay'] or 0
storeJob(eventsKey, jobIdKey, jobId, name, data, opts, timestamp, parentKey, parentData)

local score = getDelayedScore(delayedKey, timestamp, delay)
rcall("ZADD", delayedKey, score, jobId)

-- Emit delayed event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
      "event", "delayed", "jobId", jobId, "delay", delayedTimestamp)

addDelayMarkerIfNeeded(markerKey, delayedKey)

return jobId
