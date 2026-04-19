--[[
  Promote delayed jobs that are ready to be processed.

  Moves delayed jobs whose score <= now from the delayed sorted set
  into the wait list (or prioritized set for priority jobs).

  Events emitted: 'waiting'

  Ported from BullMQ.
  Depends on addBaseMarkerIfNeeded and getPriorityScore being inlined
  before this include.
]]

-- Try to get as much as 1000 jobs at once
local function promoteDelayedJobs(delayedKey, markerKey, targetKey, prioritizedKey,
                                  eventStreamKey, prefix, timestamp, priorityCounterKey,
                                  isPaused)
  local jobs = rcall("ZRANGEBYSCORE", delayedKey, 0,
    (timestamp + 1) * 0x1000 - 1, "LIMIT", 0, 1000)

  if (#jobs > 0) then
    rcall("ZREM", delayedKey, unpack(jobs))

    for _, jobId in ipairs(jobs) do
      local jobKey = prefix .. jobId
      local priority =
        tonumber(rcall("HGET", jobKey, "priority")) or 0

      if priority == 0 then
        -- LIFO or FIFO
        rcall("LPUSH", targetKey, jobId)
      else
        local score = getPriorityScore(priority, priorityCounterKey)
        rcall("ZADD", prioritizedKey, score, jobId)
      end

      -- Emit waiting event
      rcall("XADD", eventStreamKey, "*", "event", "waiting", "jobId",
            jobId, "prev", "delayed")
      rcall("HSET", jobKey, "delay", 0)
    end

    addBaseMarkerIfNeeded(markerKey, isPaused)
  end
end
