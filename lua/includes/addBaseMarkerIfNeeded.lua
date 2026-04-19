--[[
  Add base marker if needed.
  Signal workers that immediate work is available by adding a "0" marker
  to the marker sorted set.

  Ported from BullMQ (stripped: rate limiter checks, max concurrency checks).
]]
local function addBaseMarkerIfNeeded(markerKey, isPausedOrMaxed)
  if not isPausedOrMaxed then
    rcall("ZADD", markerKey, 0, "0")
  end
end
