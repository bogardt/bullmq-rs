--[[
  Change the priority of a job.

  Moves job between wait list and prioritized sorted set as needed.
  Updates the priority field on the job hash.

  Input:
    KEYS[1] meta
    KEYS[2] prioritized
    KEYS[3] wait
    KEYS[4] paused
    KEYS[5] active
    KEYS[6] pc (priority counter)
    KEYS[7] marker

    ARGV[1] priority (new value)
    ARGV[2] jobId
    ARGV[3] jobKey
    ARGV[4] events prefix key (e.g. "bull:queueName:events")
    ARGV[5] timestamp
    ARGV[6] maxEvents

  Output:
    0  - OK
    -1 - Job not found

  Ported from BullMQ (stripped: flows, groups).
]]
local rcall = redis.call

--@include "addBaseMarkerIfNeeded"
--@include "getPriorityScore"

local metaKey = KEYS[1]
local prioritizedKey = KEYS[2]
local waitKey = KEYS[3]
local pausedKey = KEYS[4]
local activeKey = KEYS[5]
local pcKey = KEYS[6]
local markerKey = KEYS[7]

local priority = tonumber(ARGV[1])
local jobId = ARGV[2]
local jobKey = ARGV[3]
local eventsKey = ARGV[4]
local timestamp = ARGV[5]
local maxEvents = tonumber(ARGV[6])

if rcall("EXISTS", jobKey) ~= 1 then
  return -1
end

local currentPriority = tonumber(rcall("HGET", jobKey, "priority")) or 0

-- Determine if the queue is paused
local queueAttributes = rcall("HMGET", metaKey, "paused", "concurrency")
local isPaused = queueAttributes[1]

if priority > 0 then
  -- Move to prioritized set
  if currentPriority == 0 then
    -- Was in wait list, move to prioritized
    local removed = rcall("LREM", waitKey, 0, jobId)
    if removed == 0 and isPaused then
      rcall("LREM", pausedKey, 0, jobId)
    end
  else
    -- Already in prioritized, remove old entry
    rcall("ZREM", prioritizedKey, jobId)
  end

  local score = getPriorityScore(priority, pcKey)
  rcall("ZADD", prioritizedKey, score, jobId)
elseif currentPriority > 0 then
  -- Was prioritized, move to wait/paused list
  rcall("ZREM", prioritizedKey, jobId)
  if isPaused then
    rcall("RPUSH", pausedKey, jobId)
  else
    rcall("RPUSH", waitKey, jobId)
  end
end

-- Update the priority field on the job hash
rcall("HSET", jobKey, "priority", priority)

-- Emit priority event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
  "event", "priority", "jobId", jobId, "priority", priority)

-- Signal workers
addBaseMarkerIfNeeded(markerKey, isPaused)

return 0
