--[[
  Add a log entry to a job's log list.

  KEYS[1] = job logs key (jobKey:logs)
  KEYS[2] = meta hash

  ARGV[1] = log line (string)
  ARGV[2] = maxLogCount (optional, 0 or absent = unlimited)

  Returns: current log count (LLEN)

  Ported from BullMQ.
]]
local rcall = redis.call

local logsKey = KEYS[1]
-- local metaKey = KEYS[2]

local logLine = ARGV[1]
local maxLogCount = tonumber(ARGV[2]) or 0

rcall("RPUSH", logsKey, logLine)

if maxLogCount > 0 then
  -- Keep only the last maxLogCount entries
  rcall("LTRIM", logsKey, -maxLogCount, -1)
end

return rcall("LLEN", logsKey)
