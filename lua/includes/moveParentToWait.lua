--[[
  Release a parent from waiting-children into the correct target queue.
]]
local function moveParentToWait(parentQueueKey, parentKey, parentId, timestamp)
  local waitKey = parentQueueKey .. ":wait"
  local activeKey = parentQueueKey .. ":active"
  local pausedKey = parentQueueKey .. ":paused"
  local prioritizedKey = parentQueueKey .. ":prioritized"
  local delayedKey = parentQueueKey .. ":delayed"
  local waitingChildrenKey = parentQueueKey .. ":waiting-children"
  local eventsKey = parentQueueKey .. ":events"
  local metaKey = parentQueueKey .. ":meta"
  local markerKey = parentQueueKey .. ":marker"
  local pcKey = parentQueueKey .. ":pc"

  local target, isPausedOrMaxed = getTargetQueueList(metaKey, activeKey, waitKey, pausedKey)
  local jobAttributes = rcall("HMGET", parentKey, "priority", "delay")
  local priority = tonumber(jobAttributes[1]) or 0
  local delay = tonumber(jobAttributes[2]) or 0

  rcall("ZREM", waitingChildrenKey, parentId)

  if delay > 0 then
    local score, delayedTimestamp = getDelayedScore(delayedKey, timestamp, delay)
    rcall("ZADD", delayedKey, score, parentId)
    rcall("XADD", eventsKey, "*", "event", "delayed", "jobId", parentId, "delay", delayedTimestamp)
    addDelayMarkerIfNeeded(markerKey, delayedKey)
  elseif priority > 0 then
    local score = getPriorityScore(priority, pcKey)
    rcall("ZADD", prioritizedKey, score, parentId)
    addBaseMarkerIfNeeded(markerKey, isPausedOrMaxed)
  else
    addJobInTargetList(target, markerKey, "RPUSH", isPausedOrMaxed, parentId)
  end

  if delay == 0 then
    rcall("XADD", eventsKey, "*", "event", "waiting", "jobId", parentId, "prev", "waiting-children")
  end
end
