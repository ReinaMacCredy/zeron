-- crates/ui swipe.rs, shell/swipe_nav.rs: numbers only, so tests read it alone.

local M = {}

M.ARM = 110
local FLICK_SPEED, FLICK_MIN = 500, 24
local VELOCITY_MS, SPAN_FLOOR_MS = 100, 50
local STALL_MS, IDLE_MS = 500, 8000
local HIDDEN, REACH, REACH_BLOCKED, OVERSHOOT = -30, 44, 6, 10
M.WATCH_MS, M.DOT, M.HIDDEN, M.ARMED_GROWTH, M.POP_SCALE = 250, 56, HIDDEN, 0.1, 0.12

export type Tracker = {
	feed: (phase: string, dx: number, dy: number, now: number) -> (string?, number?),
	expire: (now: number) -> boolean,
	swiping: () -> boolean,
}
function M.Tracker(): Tracker
	local travel: number? = nil
	local stalled = false
	local last = 0
	local samples: { { number } } = {}
	local function sample(now: number)
		table.insert(samples, { now, travel :: number })
		while samples[1] and now - samples[1][1] > VELOCITY_MS do
			table.remove(samples, 1)
		end
	end
	local function speed(now: number): number
		local first, latest = samples[1], samples[#samples]
		if not first then
			return 0
		end
		return (latest[2] - first[2]) / (math.max(now - first[1], SPAN_FLOOR_MS) / 1000)
	end
	local tracker = {}
	function tracker.feed(phase: string, dx: number, dy: number, now: number): (string?, number?)
		last = now
		if phase == "began" then
			travel, stalled, samples = 0, false, {}
			return nil
		elseif travel == nil then
			return nil
		elseif phase == "moved" then
			-- (Still: the platform's cancelled it. Silence after confirms.)
			stalled = dx == 0 and dy == 0
			travel = (travel :: number) - dx
			if not stalled then
				sample(now)
			end
			return "track", travel
		end
		local went = travel :: number
		travel = nil
		if phase ~= "ended" then
			return "cancel"
		end
		local v = speed(now)
		local flick = math.abs(went) >= FLICK_MIN and math.abs(v) >= FLICK_SPEED and math.sign(v) == math.sign(went)
		if math.abs(went) >= M.ARM or flick then
			return "commit", if went >= 0 then 1 else -1
		end
		return "cancel"
	end
	function tracker.expire(now: number): boolean
		if travel == nil or now - last < (if stalled then STALL_MS else IDLE_MS) then
			return false
		end
		travel = nil
		return true
	end
	function tracker.swiping(): boolean
		return travel ~= nil
	end
	return tracker
end

function M.center(travel: number, has_target: boolean): number
	local progress = math.min(math.abs(travel) / M.ARM, 1)
	local eased = 1 - (1 - progress) ^ 3
	if not has_target then
		return HIDDEN + (REACH_BLOCKED - HIDDEN) * eased
	end
	local past = math.abs(travel) - M.ARM
	local over = if past > 0 then OVERSHOOT * (1 - math.exp(-past / (OVERSHOOT * 3))) else 0
	return HIDDEN + (REACH - HIDDEN) * eased + over
end

return M
