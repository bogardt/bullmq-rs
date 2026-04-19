--[[
  Extend the lock on a job.

  KEYS[1] = job lock key (jobKey:lock)
  KEYS[2] = stalled set

  ARGV[1] = token
  ARGV[2] = lockDuration (milliseconds)
  ARGV[3] = jobId

  Returns:
    0   on success
    -2  if lock does not exist or token does not match

  Ported from BullMQ.
]]
local rcall = redis.call

local lockKey = KEYS[1]
local stalledKey = KEYS[2]

local token = ARGV[1]
local lockDuration = tonumber(ARGV[2])
local jobId = ARGV[3]

local lockToken = rcall("GET", lockKey)
if lockToken == token then
  rcall("SET", lockKey, token, "PX", lockDuration)
  rcall("SREM", stalledKey, jobId)
  return 0
end

return -2
