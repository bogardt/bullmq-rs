--[[
  Add a prioritized job.

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
  ARGV[7] = prioritizedKey
  ARGV[8] = priorityCounterKey
  ARGV[9] = parentKey (optional)
  ARGV[10] = parent (optional)
  ARGV[11] = deduplication key (optional, prefix:queueName:de:<dedupId>)

  Returns: jobId (the existing job's id when the add is deduplicated)

  Ported from BullMQ (stripped: parent/flow, repeat).
]]
local rcall = redis.call

--@include "storeJob"
--@include "addBaseMarkerIfNeeded"
--@include "getPriorityScore"
--@include "deduplicateJob"

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
local prioritizedKey = ARGV[7]
local pcKey = ARGV[8]
local parentKey = ARGV[9]
local parentData = ARGV[10]
local deduplicationKey = ARGV[11]

if parentKey == "" then
  parentKey = nil
end
if parentData == "" then
  parentData = nil
end
if deduplicationKey == "" then
  deduplicationKey = nil
end

-- Idempotent: if job hash already exists, return the jobId
if rcall("EXISTS", jobIdKey) == 1 then
  return jobId
end

local deduplicationJobId = deduplicateJob(opts['de'], jobId, deduplicationKey,
                                          eventsKey, maxEvents)
if deduplicationJobId then
  return deduplicationJobId
end

local delay, priority = storeJob(eventsKey, jobIdKey, jobId, name, data, opts, timestamp,
                                 parentKey, parentData)

-- Add to prioritized sorted set
local score = getPriorityScore(priority, pcKey)
rcall("ZADD", prioritizedKey, score, jobId)

-- Emit waiting event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
      "event", "waiting", "jobId", jobId, "prev", "")

-- Signal workers if not paused
local paused = rcall("HEXISTS", metaKey, "paused") == 1
if not paused then
  addBaseMarkerIfNeeded(markerKey)
end

return jobId
