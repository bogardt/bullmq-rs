--[[
  Record the completed child return value and release the parent if it has
  no pending dependencies left.
]]
local function updateParentDepsIfNeeded(parentKey, parentQueueKey, parentDependenciesKey,
  parentId, jobIdKey, returnvalue, timestamp)
  redis.call("HSET", parentKey .. ":processed", jobIdKey, returnvalue)
  moveParentToWaitIfNoPendingDependencies(
    parentQueueKey,
    parentDependenciesKey,
    parentKey,
    parentId,
    timestamp
  )
end
