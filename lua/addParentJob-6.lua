--[[
  Add a parent job directly into the waiting-children state.

  KEYS[1] = waiting-children sorted set
  KEYS[2] = meta hash
  KEYS[3] = id counter key
  KEYS[4] = events stream
  KEYS[5] = job hash key (prefix:queueName:jobId)
  KEYS[6] = parent dependencies key (optional, stores fully-qualified child job keys)

  ARGV[1] = name
  ARGV[2] = data (JSON string)
  ARGV[3] = timestamp
  ARGV[4] = jobId
  ARGV[5] = opts (JSON string)
  ARGV[6] = maxEvents
  ARGV[7] = parentKey (optional)
  ARGV[8] = parent (optional)

  Returns: jobId
]]
local rcall = redis.call

--@include "storeJob"

local waitChildrenKey = KEYS[1]
local eventsKey = KEYS[4]
local jobIdKey = KEYS[5]
local childJobKey = jobIdKey
local parentDependenciesKey = KEYS[6]

local name = ARGV[1]
local data = ARGV[2]
local timestamp = ARGV[3]
local jobId = ARGV[4]
local opts = cjson.decode(ARGV[5])
local maxEvents = tonumber(ARGV[6]) or 10000
local parentKey = ARGV[7]
local parentData = ARGV[8]

if parentDependenciesKey == "" then
  parentDependenciesKey = nil
end
if parentKey == "" then
  parentKey = nil
end
if parentData == "" then
  parentData = nil
end

if rcall("EXISTS", jobIdKey) == 1 then
  return jobId
end

storeJob(eventsKey, jobIdKey, jobId, name, data, opts, timestamp, parentKey, parentData)
rcall("HSET", jobIdKey, "state", "waiting-children")

rcall("ZADD", waitChildrenKey, timestamp, jobId)
rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event",
      "waiting-children", "jobId", jobId)

if parentDependenciesKey ~= nil then
  rcall("SADD", parentDependenciesKey, childJobKey)
end

return jobId
