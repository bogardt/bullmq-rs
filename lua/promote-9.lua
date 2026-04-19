--[[
  Promote a delayed job: move it from delayed to wait/prioritized.

  Input:
    KEYS[1] delayed
    KEYS[2] wait
    KEYS[3] paused
    KEYS[4] meta
    KEYS[5] prioritized
    KEYS[6] active
    KEYS[7] pc (priority counter)
    KEYS[8] events
    KEYS[9] marker

    ARGV[1] jobId
    ARGV[2] jobKey
    ARGV[3] timestamp
    ARGV[4] maxEvents

  Output:
    0  - OK
    -1 - Job not in delayed set

  Ported from BullMQ (stripped: flows, groups).
]]
local rcall = redis.call

--@include "addBaseMarkerIfNeeded"
--@include "getPriorityScore"
--@include "moveJobToWait"

local delayedKey = KEYS[1]
local waitKey = KEYS[2]
local pausedKey = KEYS[3]
local metaKey = KEYS[4]
local prioritizedKey = KEYS[5]
local activeKey = KEYS[6]
local pcKey = KEYS[7]
local eventsKey = KEYS[8]
local markerKey = KEYS[9]

local jobId = ARGV[1]
local jobKey = ARGV[2]
local timestamp = ARGV[3]
local maxEvents = tonumber(ARGV[4])

-- Remove from delayed set
if rcall("ZREM", delayedKey, jobId) == 0 then
  return -1
end

-- Reset the delay field
rcall("HSET", jobKey, "delay", 0)

-- Read priority to determine target
local priority = tonumber(rcall("HGET", jobKey, "priority")) or 0

if priority > 0 then
  local score = getPriorityScore(priority, pcKey)
  rcall("ZADD", prioritizedKey, score, jobId)
  addBaseMarkerIfNeeded(markerKey, false)
else
  local target, isPausedOrMaxed = getTargetQueueList(metaKey, activeKey, waitKey, pausedKey)
  addJobInTargetList(target, markerKey, "RPUSH", isPausedOrMaxed, jobId)
end

-- Emit waiting event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
  "event", "waiting", "jobId", jobId, "prev", "delayed")

return 0
