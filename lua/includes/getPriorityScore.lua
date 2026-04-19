--[[
  Compute priority-based score.

  Formula: priority * 0x100000000 + (INCR priorityCounterKey) % 0x100000000

  This ensures that within the same priority level, jobs are ordered
  by insertion order (FIFO) via the counter.

  Ported from BullMQ.
]]
local function getPriorityScore(priority, priorityCounterKey)
  local prioCounter = rcall("INCR", priorityCounterKey)
  return priority * 0x100000000 + prioCounter % 0x100000000
end
