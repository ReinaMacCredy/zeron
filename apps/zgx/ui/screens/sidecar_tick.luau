--!strict
local M = {}

function M.next_change(secs: number): number
	local s = math.max(math.floor(secs), 0)
	local unit = if s < 60 then 1 elseif s < 3600 then 60 elseif s < 86400 then 3600 else 86400
	return unit - s % unit
end

function M.soonest(started: { number }, now: number): number?
	local soonest: number? = nil
	for _, at in started do
		local wait = M.next_change(now - at)
		soonest = if soonest then math.min(soonest, wait) else wait
	end
	return soonest
end

return M
