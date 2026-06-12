--[[
  Function to deduplicate a job by deduplication id.

  Returns the currently deduplicated job id when the add is deduplicated,
  nil when the job should be added normally.

  Ported from BullMQ (stripped: replace/extend/keepLastIfActive modes).
]]
local function deduplicateJob(deduplicationOpts, jobId, deduplicationKey, eventsKey, maxEvents)
  local deduplicationId = deduplicationOpts and deduplicationOpts['id']
  if deduplicationId then
    local ttl = deduplicationOpts['ttl']
    local deduplicationKeyExists
    if ttl and ttl > 0 then
      deduplicationKeyExists = not rcall('SET', deduplicationKey, jobId, 'PX', ttl, 'NX')
    else
      deduplicationKeyExists = not rcall('SET', deduplicationKey, jobId, 'NX')
    end

    if deduplicationKeyExists then
      local currentDebounceJobId = rcall('GET', deduplicationKey)

      -- TODO remove debounced event in next breaking change
      rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event", "debounced",
            "jobId", currentDebounceJobId, "debounceId", deduplicationId)
      rcall("XADD", eventsKey, "MAXLEN", "~", maxEvents, "*", "event", "deduplicated",
            "jobId", currentDebounceJobId, "deduplicationId", deduplicationId,
            "deduplicatedJobId", jobId)
      return currentDebounceJobId
    end
  end
end
