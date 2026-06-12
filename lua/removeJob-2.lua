--[[
    Remove a job from all the statuses it may be in as well as all its data.
    In order to be able to remove a job, it cannot be active.

    Input:
      KEYS[1] jobKey
      KEYS[2] repeat key

      ARGV[1] jobId
      ARGV[2] remove children
      ARGV[3] queue prefix

    Output:
      1  - removed
      0  - the job (or one of its children) is locked
      -8 - the job belongs to a job scheduler

    Events:
      'removed'

  Ported from BullMQ.
]]

local rcall = redis.call

--@include "destructureJobKey"
--@include "addBaseMarkerIfNeeded"
--@include "getTargetQueueList"
--@include "addJobInTargetList"
--@include "getOrSetMaxEvents"
--@include "isJobSchedulerJob"
--@include "removeDeduplicationKeyIfNeededOnRemoval"
--@include "removeJobKeys"
--@include "removeParentDependencyKey"
--@include "isLocked"
--@include "removeJobFromAnyState"
--@include "removeJobWithChildren"

local jobId = ARGV[1]
local shouldRemoveChildren = ARGV[2]
local prefix = ARGV[3]
local jobKey = KEYS[1]
local repeatKey = KEYS[2]

if isJobSchedulerJob(jobId, jobKey, repeatKey) then
    return -8
end

if not isLocked(prefix, jobId, shouldRemoveChildren) then
    local options = {
        removeChildren = shouldRemoveChildren == "1",
        ignoreProcessed = false,
        ignoreLocked = false
    }

    removeJobWithChildren(prefix, jobId, nil, options)
    return 1
end
return 0
