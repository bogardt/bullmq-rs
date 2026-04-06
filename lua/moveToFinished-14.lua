--[[
  Move a job to the completed or failed state and optionally fetch the next job.

  KEYS[1]  = wait list
  KEYS[2]  = active list
  KEYS[3]  = prioritized sorted set
  KEYS[4]  = events stream
  KEYS[5]  = stalled set
  KEYS[6]  = limiter key (unused)
  KEYS[7]  = delayed sorted set
  KEYS[8]  = paused list
  KEYS[9]  = finished set (completed or failed sorted set)
  KEYS[10] = meta hash
  KEYS[11] = priority counter key
  KEYS[12] = marker sorted set
  KEYS[13] = job hash key
  KEYS[14] = job lock key (jobKey:lock)

  ARGV[1]  = token
  ARGV[2]  = timestamp
  ARGV[3]  = returnvalue or failedReason (JSON string)
  ARGV[4]  = target state name ("completed" or "failed")
  ARGV[5]  = maxEvents
  ARGV[6]  = fetchNext (0 or 1)
  ARGV[7]  = lockDuration
  ARGV[8]  = jobId
  ARGV[9]  = attempts made
  ARGV[10] = jobKeyPrefix (e.g. "bull:queueName:")

  Returns:
    On error: negative integer (-1 = missing job, -2 = missing lock, -6 = token mismatch)
    On success without fetchNext: 0
    On success with fetchNext: array [nextJobId, nextJobData...] or 0

  Ported from BullMQ (stripped: parent/flow, rate limiter, groups, metrics).
]]
local rcall = redis.call

--@include "removeLock"
--@include "addBaseMarkerIfNeeded"
--@include "getPriorityScore"
--@include "getDelayedScore"
--@include "addDelayMarkerIfNeeded"
--@include "promoteDelayedJobs"

local waitKey = KEYS[1]
local activeKey = KEYS[2]
local prioritizedKey = KEYS[3]
local eventsKey = KEYS[4]
local stalledKey = KEYS[5]
-- local limiterKey = KEYS[6]
local delayedKey = KEYS[7]
local pausedKey = KEYS[8]
local finishedKey = KEYS[9]
local metaKey = KEYS[10]
local pcKey = KEYS[11]
local markerKey = KEYS[12]
local jobKey = KEYS[13]
local lockKey = KEYS[14]

local token = ARGV[1]
local timestamp = tonumber(ARGV[2])
local returnvalue = ARGV[3]
local targetState = ARGV[4]
local maxEvents = tonumber(ARGV[5]) or 10000
local fetchNext = tonumber(ARGV[6]) or 0
local lockDuration = tonumber(ARGV[7])
local jobId = ARGV[8]
local attemptsMade = ARGV[9]
local jobKeyPrefix = ARGV[10]

-- 1. Check the job still exists
if rcall("EXISTS", jobKey) ~= 1 then
  return -1
end

-- 2. Verify and remove the lock
local lockResult = removeLock(jobKey, stalledKey, token, jobId)
if lockResult < 0 then
  return lockResult
end

-- 3. Remove from active list
rcall("LREM", activeKey, 1, jobId)

-- 4. Add to finished set (scored by timestamp)
rcall("ZADD", finishedKey, timestamp, jobId)

-- 5. Update job hash
if targetState == "completed" then
  rcall("HSET", jobKey,
        "finishedOn", timestamp,
        "returnvalue", returnvalue,
        "atm", attemptsMade)
else
  -- failed
  rcall("HSET", jobKey,
        "finishedOn", timestamp,
        "failedReason", returnvalue,
        "atm", attemptsMade)
end

-- 6. Emit finished event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
      "event", targetState, "jobId", jobId,
      targetState == "completed" and "returnvalue" or "failedReason", returnvalue,
      "prev", "active")

-- 7. If fetchNext requested, try to get the next job (same logic as moveToActive)
if fetchNext == 1 then
  local isPaused = rcall("HEXISTS", metaKey, "paused") == 1

  -- Promote delayed jobs
  promoteDelayedJobs(delayedKey, markerKey, waitKey, prioritizedKey,
                     eventsKey, jobKeyPrefix, timestamp, pcKey, isPaused)

  if isPaused then
    return 0
  end

  -- Try wait list
  local nextJobId = rcall("RPOPLPUSH", waitKey, activeKey)

  -- Try prioritized
  if not nextJobId then
    local result = rcall("ZPOPMIN", prioritizedKey)
    if #result > 0 then
      nextJobId = result[1]
      rcall("LPUSH", activeKey, nextJobId)
    end
  end

  if not nextJobId then
    addDelayMarkerIfNeeded(markerKey, delayedKey)
    return 0
  end

  -- Acquire lock on next job
  local nextJobKey = jobKeyPrefix .. nextJobId
  local nextLockKey = nextJobKey .. ":lock"

  rcall("SET", nextLockKey, token, "PX", lockDuration)
  rcall("SADD", stalledKey, nextJobId)
  rcall("HINCRBY", nextJobKey, "ats", 1)

  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
        "event", "active", "jobId", nextJobId, "prev", "waiting")

  rcall("ZREM", markerKey, "0")

  local nextJobData = rcall("HGETALL", nextJobKey)
  local result = {nextJobId}
  for i, v in ipairs(nextJobData) do
    result[#result + 1] = v
  end
  return result
end

return 0
