--[[
  Move an active job to the delayed state.

  KEYS[1] = active list
  KEYS[2] = delayed sorted set
  KEYS[3] = events stream
  KEYS[4] = job hash key
  KEYS[5] = job lock key (jobKey:lock)
  KEYS[6] = meta hash
  KEYS[7] = marker sorted set
  KEYS[8] = stalled set

  ARGV[1] = token
  ARGV[2] = timestamp (current time ms)
  ARGV[3] = jobId
  ARGV[4] = delayedTimestamp (target time ms)
  ARGV[5] = maxEvents
  ARGV[6] = attemptsMade

  Returns:
    0  on success
    -1 if job not found
    -2 if lock missing
    -6 if token mismatch

  Ported from BullMQ (stripped: parent/flow logic).
]]
local rcall = redis.call

--@include "removeLock"
--@include "getDelayedScore"
--@include "addDelayMarkerIfNeeded"

local activeKey = KEYS[1]
local delayedKey = KEYS[2]
local eventsKey = KEYS[3]
local jobKey = KEYS[4]
local lockKey = KEYS[5]
local metaKey = KEYS[6]
local markerKey = KEYS[7]
local stalledKey = KEYS[8]

local token = ARGV[1]
local timestamp = ARGV[2]
local jobId = ARGV[3]
local delayedTimestamp = tonumber(ARGV[4])
local maxEvents = tonumber(ARGV[5]) or 10000
local attemptsMade = ARGV[6] or "0"

-- 1. Check job exists
if rcall("EXISTS", jobKey) ~= 1 then
  return -1
end

-- 2. Verify and remove lock
local lockResult = removeLock(jobKey, stalledKey, token, jobId)
if lockResult < 0 then
  return lockResult
end

-- 3. Remove from active
rcall("LREM", activeKey, 1, jobId)

-- 4. Compute delayed score and add to delayed set
local score = getDelayedScore(delayedKey, delayedTimestamp, 0)
rcall("ZADD", delayedKey, score, jobId)

-- 5. Update job hash
rcall("HSET", jobKey, "delay", delayedTimestamp - tonumber(timestamp), "atm", attemptsMade)

-- 6. Emit delayed event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
      "event", "delayed", "jobId", jobId, "delay", delayedTimestamp)

-- 7. Signal workers about delayed job
addDelayMarkerIfNeeded(markerKey, delayedKey)

return 0
