--[[
  Adds an already-stored job to the delayed zset and emits the
  'delayed' event.

  Ported from BullMQ.
]]
local function addDelayedJob(jobId, delayedKey, eventsKey, timestamp,
  maxEvents, markerKey, delay)

  local score, delayedTimestamp = getDelayedScore(delayedKey, timestamp, tonumber(delay))

  rcall("ZADD", delayedKey, score, jobId)
  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event", "delayed",
    "jobId", jobId, "delay", delayedTimestamp)

  -- mark that a delayed job is available
  addDelayMarkerIfNeeded(markerKey, delayedKey)
end
