--[[
  Move the parent only if it still exists and is still blocked in waiting-children.
]]
local function moveParentToWaitIfNeeded(parentQueueKey, parentKey, parentId, timestamp)
  if rcall("EXISTS", parentKey) == 1 and
    rcall("ZSCORE", parentQueueKey .. ":waiting-children", parentId) then
    moveParentToWait(parentQueueKey, parentKey, parentId, timestamp)
  end
end
