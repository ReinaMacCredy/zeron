-- crates/ui swipe.rs, shell/swipe_nav.rs.

local ds = require("@ds")
local motion = require("@ds/motion")
local Glass = require("@ds/glass")
local route = require("@app/screens/shell/route")
local tracking = require("@app/screens/shell/swipe_tracker")

local M = {}

local HIDDEN, DOT, ARMED_GROWTH, POP_SCALE, WATCH_MS = tracking.HIDDEN, tracking.DOT, tracking.ARMED_GROWTH, tracking.POP_SCALE, tracking.WATCH_MS

-- (Kept outside what's drawn, so a move touches this one node.)
local way = ui.state("shell_swipe_way", 0)
local center = ui.state("shell_swipe_center", HIDDEN)
local lit = ui.state("shell_swipe_lit", 0)
local scale = ui.state("shell_swipe_scale", 1)
local tracker = tracking.Tracker()
local armed = false
local watching = false
local moving: any = nil

local function target_of(sign: number): boolean
	return if sign >= 0 then route.can_back() else route.can_forward()
end
local function tick()
	pcall(ui.invoke, "haptic.step")
end
local function settle()
	if moving then
		moving:cancel()
		moving = nil
	end
end
local function arm(on: boolean)
	if armed == on then
		return
	end
	armed = on
	if motion.reduced_now() then
		lit:set(if on then 1 else 0)
	else
		anime.animate(lit, { value = if on then 1 else 0, duration = 200, ease = "outBack(2.2)" })
	end
end

local function spring_back(travel: number)
	arm(false)
	settle()
	if motion.reduced_now() then
		way:set(0)
		return
	end
	center:set(tracking.center(travel, target_of(math.sign(travel))))
	moving = anime.animate(center, {
		value = HIDDEN,
		duration = 180,
		ease = "out(3)",
		onComplete = function()
			moving = nil
			way:set(0)
		end,
	})
end

local shown_travel = 0
local function watch()
	if watching then
		return
	end
	watching = true
	local function look()
		if tracker.expire(ui.now()) then
			spring_back(shown_travel)
		end
		if tracker.swiping() then
			ui.after(WATCH_MS, look)
		else
			watching = false
		end
	end
	ui.after(WATCH_MS, look)
end

function M.on_swipe(e: { phase: string, dx: number, dy: number })
	local step, value = tracker.feed(e.phase, e.dx, e.dy, ui.now())
	if step == "track" then
		local travel = value :: number
		if moving or way:peek() == 0 then
			settle()
			scale:set(1)
		end
		shown_travel = travel
		local sign = if travel >= 0 then 1 else -1
		local has = target_of(sign)
		if way:peek() ~= sign then
			way:set(sign)
		end
		center:set(tracking.center(travel, has))
		local on = math.abs(travel) >= tracking.ARM and has
		if on and not armed then
			tick()
		end
		arm(on)
		watch()
	elseif step == "commit" then
		local sign = value :: number
		if not target_of(sign) then
			spring_back(shown_travel)
			return
		end
		if not armed then
			tick()
		end
		arm(false)
		settle()
		if sign > 0 then
			route.back()
		else
			route.forward()
		end
		if motion.reduced_now() then
			way:set(0)
			return
		end
		moving = anime.animate(scale, {
			value = POP_SCALE,
			duration = 200,
			ease = "out(4)",
			onComplete = function()
				moving = nil
				way:set(0)
				scale:set(1)
			end,
		})
	elseif step == "cancel" then
		spring_back(shown_travel)
	end
end

function M.showing(): boolean
	return way:peek() ~= 0
end

function M.on_navigate(e: { direction: string })
	if e.direction == "back" then
		route.back()
	else
		route.forward()
	end
end

function M.Pill(): Node
	local function size(): number
		return DOT * (1 + ARMED_GROWTH * lit:get()) * scale:get()
	end
	local function edge(): number
		return center:get() - size() / 2
	end
	local body: { [any]: any } = {
		id = "swipe-pill",
		absolute = true,
		top = "50%",
		left = function(): any
			return if way:get() > 0 then ui.raw(edge(), "where the fingers have pulled it") else nil
		end,
		right = function(): any
			return if way:get() < 0 then ui.raw(edge(), "where the fingers have pulled it") else nil
		end,
		mt = function(): any
			return ui.raw(-size() / 2, "centred down the page")
		end,
		w = size,
		h = size,
		rounded = "$radius.full",
		items = "center",
		justify = "center",
		pointer_events = "none",
		opacity = function(): number
			return 1 - math.clamp((1 - scale:get()) / (1 - POP_SCALE) - 0.55, 0, 0.45) / 0.45
		end,
		color = function(): string
			return if lit:get() >= 0.5 then "$text.primary" else "$text.secondary"
		end,
		ui.presence(function(): string
			return if way:get() >= 0 then "chevron-left" else "chevron-right"
		end, function(name: string): Node
			return ui.div {
				scale = function(): number
					return (1 + 0.15 * lit:get()) * scale:get()
				end,
				ds.Icon { name = name :: any, size = 20 },
			}
		end),
	}
	Glass.style(body, "menu")
	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		overflow = "hidden",
		pointer_events = "none",
		ui.show(function(): boolean
			return way:get() ~= 0
		end, ui.div(body)),
	}
end

return M
