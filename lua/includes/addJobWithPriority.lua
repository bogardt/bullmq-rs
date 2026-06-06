--[[
  Add a job to the prioritized sorted set and emit a worker marker when applicable.
]]
local function addJobWithPriority(markerKey, prioritizedKey, priority, jobId,
  priorityCounterKey, isPausedOrMaxed)
  local score = getPriorityScore(priority, priorityCounterKey)
  rcall("ZADD", prioritizedKey, score, jobId)
  addBaseMarkerIfNeeded(markerKey, isPausedOrMaxed)
end
