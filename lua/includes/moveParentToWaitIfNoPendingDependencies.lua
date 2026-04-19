--[[
  Release the parent only after the dependency set becomes empty.
]]
local function moveParentToWaitIfNoPendingDependencies(parentQueueKey, parentDependenciesKey,
  parentKey, parentId, timestamp)
  if redis.call("SCARD", parentDependenciesKey) == 0 then
    moveParentToWaitIfNeeded(parentQueueKey, parentKey, parentId, timestamp)
  end
end
