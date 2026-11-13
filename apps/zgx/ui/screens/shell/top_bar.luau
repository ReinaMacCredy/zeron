-- crates/ui shell.rs `titlebar_nav_shift`.

local ds = require("@ds")
local motion = require("@ds/motion")
local route = require("@app/screens/shell/route")
local view = require("@app/screens/shell/view")
local fit = require("@app/screens/shell/fit")
local art = require("@app/screens/shell/art")

local M = {}

local CONTROL, GAP, PAST_NAV, PAST_EDGE = 24, 2, 12, 8

-- (Read by what's beside it, which a `motion.tween` binding can't be.)
local function sliding(key: string, to: () -> number): State<number>
	local value = ui.state(key, to())
	local aim = value:peek()
	ui.effect(function()
		local goal = to()
		if aim == goal then
			return
		end
		aim = goal
		if motion.reduced_now() then
			value:set(goal)
		else
			anime.animate(value, { value = goal, duration = motion.sidebar.duration, ease = motion.sidebar.ease })
		end
	end)
	return value
end

local reveal = sliding("shell_top_bar_reveal", function(): number
	return if fit.current:get().sidebar and not view.collapsed:get() then 1 else 0
end)
function M.reveal(): number
	return reveal:get()
end

local room = sliding("shell_top_bar_new_tab", function(): number
	return if route.chat:get().name == "home" then 0 else 1
end)
function M.new_tab(): number
	return (1 - reveal:get()) * room:get()
end

local function page_edge(): number
	return art.page_left:get()
end

function M.nav_left(closed_up: number): number
	return math.max(closed_up, page_edge() + PAST_EDGE)
end

function M.options_x(): number
	return math.max(page_edge() - PAST_EDGE - CONTROL, ds.TitleBar.nav_base())
end

function M.tabs_start(): number
	local nav = M.nav_left(ds.TitleBar.nav_base())
	return nav + CONTROL + GAP + CONTROL + (GAP + CONTROL) * M.new_tab() + PAST_NAV
end

return M
