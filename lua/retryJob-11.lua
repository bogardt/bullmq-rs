--[[
  Retry a failed/active job by moving it back to wait or prioritized.

  KEYS[1]  = active list
  KEYS[2]  = wait list
  KEYS[3]  = stalled set
  KEYS[4]  = paused list
  KEYS[5]  = meta hash
  KEYS[6]  = events stream
  KEYS[7]  = delayed sorted set
  KEYS[8]  = prioritized sorted set
  KEYS[9]  = priority counter key
  KEYS[10] = marker sorted set
  KEYS[11] = job hash key

  ARGV[1] = token
  ARGV[2] = pushCmd ("LPUSH" or "RPUSH")
  ARGV[3] = jobId
  ARGV[4] = maxEvents
  ARGV[5] = attemptsMade
  ARGV[6] = priority

  Returns:
    0  on success
    -1 if job not found
    -2 if lock missing
    -6 if token mismatch

  Ported from BullMQ (stripped: parent/flow logic).
]]
local rcall = redis.call

--@include "removeLock"
--@include "addBaseMarkerIfNeeded"
--@include "getPriorityScore"

local activeKey = KEYS[1]
local waitKey = KEYS[2]
local stalledKey = KEYS[3]
local pausedKey = KEYS[4]
local metaKey = KEYS[5]
local eventsKey = KEYS[6]
local delayedKey = KEYS[7]
local prioritizedKey = KEYS[8]
local pcKey = KEYS[9]
local markerKey = KEYS[10]
local jobKey = KEYS[11]

local token = ARGV[1]
local pushCmd = ARGV[2]
local jobId = ARGV[3]
local maxEvents = tonumber(ARGV[4]) or 10000
local attemptsMade = ARGV[5]
local priority = tonumber(ARGV[6]) or 0

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

-- 4. Update attempts
rcall("HSET", jobKey, "atm", attemptsMade)

-- 5. Check if paused
local isPaused = rcall("HEXISTS", metaKey, "paused") == 1

-- 6. Move to wait or prioritized
if priority > 0 then
  local score = getPriorityScore(priority, pcKey)
  rcall("ZADD", prioritizedKey, score, jobId)
else
  local target = isPaused and pausedKey or waitKey
  -- RPUSH puts retried jobs at the end (behind fresh jobs)
  rcall(pushCmd, target, jobId)
end

-- 7. Emit waiting event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
      "event", "waiting", "jobId", jobId, "prev", "failed")

-- 8. Signal workers
addBaseMarkerIfNeeded(markerKey, isPaused)

return 0
