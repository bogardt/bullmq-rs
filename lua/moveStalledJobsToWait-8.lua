--[[
  Move stalled jobs back to wait (or mark them as failed).

  KEYS[1] = wait list
  KEYS[2] = active list
  KEYS[3] = prioritized sorted set
  KEYS[4] = stalled set
  KEYS[5] = stalled-check key
  KEYS[6] = meta hash
  KEYS[7] = paused list
  KEYS[8] = events stream

  ARGV[1] = maxStalledCount (max times a job can stall before failing)
  ARGV[2] = stalledInterval (ms, used as PX for the stalled-check lock)
  ARGV[3] = timestamp
  ARGV[4] = maxEvents
  ARGV[5] = jobKeyPrefix (e.g. "bull:queueName:")
  ARGV[6] = markerKey (e.g. "bull:queueName:marker")
  ARGV[7] = priorityCounterKey (e.g. "bull:queueName:pc")

  Returns: {stalledCount, failedCount}

  Ported from BullMQ (stripped: parent/flow logic, groups).
]]
local rcall = redis.call

--@include "addBaseMarkerIfNeeded"
--@include "getPriorityScore"

local waitKey = KEYS[1]
local activeKey = KEYS[2]
local prioritizedKey = KEYS[3]
local stalledKey = KEYS[4]
local stalledCheckKey = KEYS[5]
local metaKey = KEYS[6]
local pausedKey = KEYS[7]
local eventsKey = KEYS[8]

local maxStalledCount = tonumber(ARGV[1]) or 1
local stalledInterval = tonumber(ARGV[2])
local timestamp = ARGV[3]
local maxEvents = tonumber(ARGV[4]) or 10000
local jobKeyPrefix = ARGV[5]
local markerKey = ARGV[6]
local pcKey = ARGV[7]

-- 1. Try to acquire the stalled-check lock (only one worker checks at a time)
local acquired = rcall("SET", stalledCheckKey, timestamp, "NX", "PX", stalledInterval)
if not acquired then
  return {0, 0}
end

-- 2. Scan active jobs and detect stalled ones (no lock = stalled)
local activeJobs = rcall("LRANGE", activeKey, 0, -1)
for _, jobId in ipairs(activeJobs) do
  local lockKey = jobKeyPrefix .. jobId .. ":lock"
  if rcall("EXISTS", lockKey) == 0 then
    rcall("SADD", stalledKey, jobId)
  end
end

-- 3. Process stalled jobs
local stalledJobs = rcall("SMEMBERS", stalledKey)
local stalledCount = 0
local failedCount = 0

if #stalledJobs > 0 then
  local isPaused = rcall("HEXISTS", metaKey, "paused") == 1

  for _, jobId in ipairs(stalledJobs) do
    local jobKey = jobKeyPrefix .. jobId

    if rcall("EXISTS", jobKey) == 1 then
      local stc = tonumber(rcall("HGET", jobKey, "stc")) or 0

      if stc >= maxStalledCount then
        -- Too many stalls: move to failed
        failedCount = failedCount + 1

        rcall("LREM", activeKey, 1, jobId)
        rcall("SREM", stalledKey, jobId)

        local failedReason = "job stalled more than allowable limit"
        rcall("HSET", jobKey,
              "failedReason", failedReason,
              "finishedOn", timestamp,
              "stc", stc + 1)

        rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
              "event", "failed", "jobId", jobId,
              "failedReason", failedReason, "prev", "active")
      else
        -- Move back to wait
        stalledCount = stalledCount + 1

        rcall("LREM", activeKey, 1, jobId)
        rcall("SREM", stalledKey, jobId)
        rcall("HINCRBY", jobKey, "stc", 1)

        local priority = tonumber(rcall("HGET", jobKey, "priority")) or 0

        if priority > 0 then
          local score = getPriorityScore(priority, pcKey)
          rcall("ZADD", prioritizedKey, score, jobId)
        else
          local target = isPaused and pausedKey or waitKey
          rcall("RPUSH", target, jobId)
        end

        rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
              "event", "stalled", "jobId", jobId)
      end
    else
      -- Job hash gone, just clean up stalled set
      rcall("SREM", stalledKey, jobId)
    end
  end

  -- Signal workers after processing
  local isPaused = rcall("HEXISTS", metaKey, "paused") == 1
  if not isPaused and stalledCount > 0 then
    addBaseMarkerIfNeeded(markerKey)
  end
end

return {stalledCount, failedCount}
