--[[
  Pause or resume a queue.

  KEYS[1] = wait list
  KEYS[2] = paused list
  KEYS[3] = meta hash
  KEYS[4] = events stream
  KEYS[5] = delayed sorted set
  KEYS[6] = prioritized sorted set
  KEYS[7] = marker sorted set

  ARGV[1] = action ("pause" or "resume")
  ARGV[2] = maxEvents

  Returns: nil

  Ported from BullMQ.
]]
local rcall = redis.call

--@include "addBaseMarkerIfNeeded"

local waitKey = KEYS[1]
local pausedKey = KEYS[2]
local metaKey = KEYS[3]
local eventsKey = KEYS[4]
-- local delayedKey = KEYS[5]
-- local prioritizedKey = KEYS[6]
local markerKey = KEYS[7]

local action = ARGV[1]
local maxEvents = tonumber(ARGV[2]) or 10000

if action == "pause" then
  -- Set paused flag
  rcall("HSET", metaKey, "paused", 1)

  -- Move all jobs from wait to paused
  while true do
    local jobId = rcall("RPOPLPUSH", waitKey, pausedKey)
    if not jobId then
      break
    end
  end

  -- Emit paused event
  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
        "event", "paused")
else
  -- Resume
  -- Remove paused flag
  rcall("HDEL", metaKey, "paused")

  -- Move all jobs from paused to wait
  while true do
    local jobId = rcall("RPOPLPUSH", pausedKey, waitKey)
    if not jobId then
      break
    end
  end

  -- Emit resumed event
  rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*",
        "event", "resumed")

  -- Signal workers
  addBaseMarkerIfNeeded(markerKey)
end
