--[[
  Remove jobs from the specific set.

  KEYS[1] = set key (wait, active, paused, delayed, prioritized, completed or failed)
  KEYS[2] = events stream
  KEYS[3] = repeat (job schedulers) sorted set

  ARGV[1] = jobKey prefix ("bull:queue:")
  ARGV[2] = timestamp (max timestamp to be considered for removal)
  ARGV[3] = limit the number of jobs to be removed. 0 is unlimited
  ARGV[4] = set name, can be any of 'wait', 'active', 'paused', 'delayed', 'prioritized', 'completed', or 'failed'

  Returns: deleted job ids

  Ported from BullMQ.
]]
local rcall = redis.call
local repeatKey = KEYS[3]
local rangeStart = 0
local rangeEnd = -1

local limit = tonumber(ARGV[3])

-- If we're only deleting _n_ items, avoid retrieving all items
-- for faster performance
--
-- Start from the tail of the list, since that's where oldest elements
-- are generally added for FIFO lists
if limit > 0 then
  rangeStart = -1 - limit + 1
  rangeEnd = -1
end

--@include "batches"
--@include "getTimestamp"
--@include "getJobsInZset"
--@include "isJobSchedulerJob"
--@include "removeJobKeys"
--@include "removeDeduplicationKeyIfNeededOnRemoval"
--@include "destructureJobKey"
--@include "addBaseMarkerIfNeeded"
--@include "getTargetQueueList"
--@include "addJobInTargetList"
--@include "removeParentDependencyKey"
--@include "removeJob"
--@include "cleanList"
--@include "cleanSet"

local result
if ARGV[4] == "active" then
  result = cleanList(KEYS[1], ARGV[1], rangeStart, rangeEnd, ARGV[2], false --[[ hasFinished ]],
                      repeatKey)
elseif ARGV[4] == "delayed" then
  rangeEnd = "+inf"
  result = cleanSet(KEYS[1], ARGV[1], rangeEnd, ARGV[2], limit,
                    {"processedOn", "timestamp"}, false  --[[ hasFinished ]], repeatKey)
elseif ARGV[4] == "prioritized" then
  rangeEnd = "+inf"
  result = cleanSet(KEYS[1], ARGV[1], rangeEnd, ARGV[2], limit,
                    {"timestamp"}, false  --[[ hasFinished ]], repeatKey)
elseif ARGV[4] == "wait" or ARGV[4] == "paused" then
  result = cleanList(KEYS[1], ARGV[1], rangeStart, rangeEnd, ARGV[2], true --[[ hasFinished ]],
                      repeatKey)
else
  rangeEnd = ARGV[2]
  -- No need to pass repeat key as in that moment job won't be related to a job scheduler
  result = cleanSet(KEYS[1], ARGV[1], rangeEnd, ARGV[2], limit,
                    {"finishedOn"}, true  --[[ hasFinished ]])
end

rcall("XADD", KEYS[2], "*", "event", "cleaned", "count", result[2])

return result[1]
