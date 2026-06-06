--[[
  Dynamically allocate a score within a timestamp bucket for delayed jobs.

  Algorithm:
    1. minScore = delayedTimestamp * 0x1000
    2. maxScore = (delayedTimestamp + 1) * 0x1000 - 1
    3. ZREVRANGEBYSCORE delayed maxScore minScore LIMIT 0 1
    4. If empty: use minScore
    5. If highest < maxScore: use highest + 1
    6. If full: use maxScore

  Ported from BullMQ.
]]
local function getDelayedScore(delayedKey, timestamp, delay)
  local delayedTimestamp = (delay > 0 and (tonumber(timestamp) + delay)) or tonumber(timestamp)
  local minScore = delayedTimestamp * 0x1000
  local maxScore = (delayedTimestamp + 1) * 0x1000 - 1

  local result = rcall("ZREVRANGEBYSCORE", delayedKey, maxScore,
    minScore, "WITHSCORES", "LIMIT", 0, 1)
  if #result then
    local currentMaxScore = tonumber(result[2])
    if currentMaxScore ~= nil then
      if currentMaxScore >= maxScore then
        return maxScore, delayedTimestamp
      else
        return currentMaxScore + 1, delayedTimestamp
      end
    end
  end
  return minScore, delayedTimestamp
end
