--[[
  Move completed, failed or delayed jobs to wait.

  Note: Does not support jobs with priorities.

  KEYS[1] = base key (queue prefix ending with ':')
  KEYS[2] = events stream
  KEYS[3] = state key (failed, completed, delayed)
  KEYS[4] = wait list
  KEYS[5] = paused list
  KEYS[6] = meta hash
  KEYS[7] = active list
  KEYS[8] = marker sorted set

  ARGV[1] = count
  ARGV[2] = timestamp
  ARGV[3] = prev state

  Returns:
    1 - the operation is not completed
    0 - the operation is completed

  Ported from BullMQ.
]]
local maxCount = tonumber(ARGV[1])
local timestamp = tonumber(ARGV[2])

local rcall = redis.call;

--@include "addBaseMarkerIfNeeded"
--@include "batches"
--@include "getOrSetMaxEvents"
--@include "getTargetQueueList"

local metaKey = KEYS[6]
local target, isPausedOrMaxed = getTargetQueueList(metaKey, KEYS[7], KEYS[4], KEYS[5])

local jobs = rcall('ZRANGEBYSCORE', KEYS[3], 0, timestamp, 'LIMIT', 0, maxCount)
if (#jobs > 0) then

    if ARGV[3] == "failed" then
        for i, key in ipairs(jobs) do
            local jobKey = KEYS[1] .. key
            rcall("HDEL", jobKey, "finishedOn", "processedOn", "failedReason")
        end
    elseif ARGV[3] == "completed" then
        for i, key in ipairs(jobs) do
            local jobKey = KEYS[1] .. key
            rcall("HDEL", jobKey, "finishedOn", "processedOn", "returnvalue")
        end
    end

    local maxEvents = getOrSetMaxEvents(metaKey)

    for i, key in ipairs(jobs) do
        rcall("XADD", KEYS[2], "MAXLEN", "~", maxEvents, "*", "event",
              "waiting", "jobId", key, "prev", ARGV[3]);
    end

    for from, to in batches(#jobs, 7000) do
        rcall("ZREM", KEYS[3], unpack(jobs, from, to))
        rcall("LPUSH", target, unpack(jobs, from, to))
    end

    addBaseMarkerIfNeeded(KEYS[8], isPausedOrMaxed)
end

maxCount = maxCount - #jobs

if (maxCount <= 0) then return 1 end

return 0
