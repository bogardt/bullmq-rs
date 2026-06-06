--[[
  Move the next job to the active state.

  KEYS[1]  = wait list
  KEYS[2]  = active list
  KEYS[3]  = prioritized sorted set
  KEYS[4]  = events stream
  KEYS[5]  = stalled set
  KEYS[6]  = limiter key (unused in this simplified port)
  KEYS[7]  = delayed sorted set
  KEYS[8]  = paused list
  KEYS[9]  = meta hash
  KEYS[10] = priority counter key
  KEYS[11] = marker sorted set

  ARGV[1] = token (worker lock token)
  ARGV[2] = lockDuration (milliseconds)
  ARGV[3] = timestamp (current time ms)
  ARGV[4] = maxEvents
  ARGV[5] = jobKeyPrefix (e.g. "bull:queueName:")

  Returns:
    Array with [jobId, jobData...] on success, or nil/0 if no jobs.

  Ported from BullMQ (stripped: rate limiter, groups, parent/flow).
]]
local rcall = redis.call

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
local metaKey = KEYS[9]
local pcKey = KEYS[10]
local markerKey = KEYS[11]

local token = ARGV[1]
local lockDuration = tonumber(ARGV[2])
local timestamp = tonumber(ARGV[3])
local maxEvents = tonumber(ARGV[4]) or 10000
local jobKeyPrefix = ARGV[5]

-- 1. Promote any delayed jobs that are ready
local isPaused = rcall("HEXISTS", metaKey, "paused") == 1
promoteDelayedJobs(delayedKey, markerKey, waitKey, prioritizedKey,
                   eventsKey, jobKeyPrefix, timestamp, pcKey, isPaused)

if isPaused then
  return nil
end

-- 2. Try to get a job from the wait list first (RPOPLPUSH = FIFO order)
local jobId = rcall("RPOPLPUSH", waitKey, activeKey)

-- 3. If no job in wait, try prioritized sorted set
if not jobId then
  local result = rcall("ZPOPMIN", prioritizedKey)
  if #result > 0 then
    jobId = result[1]
    rcall("LPUSH", activeKey, jobId)
  end
end

-- 4. If still no job, check if we should emit drained
if not jobId then
  -- Check if there are delayed jobs; if so, add delay marker
  addDelayMarkerIfNeeded(markerKey, delayedKey)
  return nil
end

-- 5. We have a job — acquire lock and set active state
local jobKey = jobKeyPrefix .. jobId
local lockKey = jobKey .. ":lock"

-- Set lock with token and expiry
rcall("SET", lockKey, token, "PX", lockDuration)

-- Add to stalled set (will be removed when lock is extended)
rcall("SADD", stalledKey, jobId)

-- Increment attempts started counter
rcall("HINCRBY", jobKey, "ats", 1)

-- Emit active event
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
      "event", "active", "jobId", jobId, "prev", "waiting")

-- Remove marker since we consumed a job
rcall("ZREM", markerKey, "0")

-- Re-add marker if there are still jobs waiting
local waitLen = rcall("LLEN", waitKey)
local priLen = rcall("ZCARD", prioritizedKey)
if waitLen > 0 or priLen > 0 then
  addBaseMarkerIfNeeded(markerKey, false)
end

-- Return job ID and all hash fields
local jobData = rcall("HGETALL", jobKey)
local result = {jobId}
for i, v in ipairs(jobData) do
  result[#result + 1] = v
end
return result
