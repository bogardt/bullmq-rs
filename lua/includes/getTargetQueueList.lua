--[[
  Resolve whether a job should go to the wait or paused list, and whether
  workers should be signaled immediately.
]]
local function getTargetQueueList(queueMetaKey, activeKey, waitKey, pausedKey)
  local queueAttributes = rcall("HMGET", queueMetaKey, "paused", "concurrency")
  local isPaused = queueAttributes[1]
  local concurrency = tonumber(queueAttributes[2])

  if isPaused then
    return pausedKey, true
  end

  if concurrency then
    local activeCount = rcall("LLEN", activeKey)
    if activeCount >= concurrency then
      return waitKey, true
    end
  end

  return waitKey, false
end
