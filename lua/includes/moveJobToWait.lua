--[[
  Move a job to the wait list or prioritized set.

  If priority > 0, ZADD to prioritized set (using getPriorityScore).
  Otherwise, LPUSH (FIFO default) or RPUSH (retried jobs) to wait list
  (or paused list if queue is paused).
  Then signal workers via addBaseMarkerIfNeeded.

  Ported from BullMQ (stripped: flow/parent logic).
  Inlines getTargetQueueList and addJobInTargetList (not registered as
  separate includes). Depends on addBaseMarkerIfNeeded and getPriorityScore
  being inlined before this include.
]]

--- getTargetQueueList: determine target list and whether the queue is
--- paused or at max concurrency.
local function getTargetQueueList(queueMetaKey, activeKey, waitKey, pausedKey)
  local queueAttributes = rcall("HMGET", queueMetaKey, "paused", "concurrency")

  if queueAttributes[1] then
    -- Queue is paused
    return pausedKey, true
  else
    if queueAttributes[2] then
      local activeCount = rcall("LLEN", activeKey)
      if activeCount >= tonumber(queueAttributes[2]) then
        return waitKey, true
      else
        return waitKey, false
      end
    end
  end
  return waitKey, false
end

--- addJobInTargetList: push a job into the target list and add marker.
local function addJobInTargetList(targetKey, markerKey, pushCmd, isPausedOrMaxed, jobId)
  rcall(pushCmd, targetKey, jobId)
  addBaseMarkerIfNeeded(markerKey, isPausedOrMaxed)
end

--- addJobWithPriority: add a job to the prioritized sorted set.
local function addJobWithPriority(markerKey, prioritizedKey, priority, jobId,
  priorityCounterKey, isPausedOrMaxed)
  local score = getPriorityScore(priority, priorityCounterKey)
  rcall("ZADD", prioritizedKey, score, jobId)
  addBaseMarkerIfNeeded(markerKey, isPausedOrMaxed)
end

local function moveJobToWait(prefixKey, metaKey, activeKey, waitKey, pausedKey,
  prioritizedKey, markerKey, eventStreamKey, jobId, pushCmd, priorityCounterKey)
  local jobKey = prefixKey .. jobId
  local priority = tonumber(rcall("HGET", jobKey, "priority")) or 0

  if priority > 0 then
    addJobWithPriority(markerKey, prioritizedKey, priority, jobId,
      priorityCounterKey, false)
  else
    local target, isPausedOrMaxed = getTargetQueueList(metaKey, activeKey, waitKey, pausedKey)
    addJobInTargetList(target, markerKey, pushCmd, isPausedOrMaxed, jobId)
  end

  rcall("XADD", eventStreamKey, "*", "event", "waiting", "jobId", jobId, "prev", "active")
end
