--[[
  Remove a job's lock after verifying the token.

  Returns:
    0   - success (lock removed or token was "0" meaning no lock needed)
    -6  - lock exists but token does not match
    -2  - lock is missing completely

  Ported from BullMQ.
]]
local function removeLock(jobKey, stalledKey, token, jobId)
  if token ~= "0" then
    local lockKey = jobKey .. ':lock'
    local lockToken = rcall("GET", lockKey)
    if lockToken == token then
      rcall("DEL", lockKey)
      rcall("SREM", stalledKey, jobId)
    else
      if lockToken then
        -- Lock exists but token does not match
        return -6
      else
        -- Lock is missing completely
        return -2
      end
    end
  end
  return 0
end
