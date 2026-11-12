local motion = require("@ds/motion")
local composer_surface = require("@ds/composer_surface")
local route = require("@app/screens/shell/route")
local Builtins = require("@app/screens/plugins/builtins")
local cards = require("@app/screens/chat/home_cards")
local widgets = require("@app/screens/chat/home_widgets")

local M = {}

local GRID_GAP = 44
local LIFT = { duration = 260, ease = "out(3)" }

function M.shown(): boolean
	return Builtins.on("home_widgets") and not route.needs_project:get() and route.current:get().name ~= "plugin"
end

local raised = ui.computed(function(): boolean
	return M.shown() and widgets.raised()
end)
local lifting: any = nil
local lift_was: boolean? = nil
ui.effect(function()
	local up = raised:get()
	if up == lift_was then
		return
	end
	local first = lift_was == nil
	lift_was = up
	if lifting then
		lifting:cancel()
		lifting = nil
	end
	local to = if up then 1 else 0
	if first or motion.reduced_now() then
		composer_surface.lift:set(to)
		return
	end
	lifting = anime.animate(composer_surface.lift, { value = to, duration = LIFT.duration, ease = LIFT.ease })
end)

M.Panel = ui.component(function(): Node
	local box = ui.ref()
	local top = ui.computed(function(): number
		local foot = composer_surface.foot:get()
		local _ = box.height
		local el = box:peek()
		return math.max(0, foot - (if el and el.y then el.y else 0) + GRID_GAP)
	end)
	local width = ui.computed(function(): number
		local _ = composer_surface.moved:get()
		local el = composer_surface.element
		return if el and el.width then el.width else 0
	end)
	return ui.div {
		ref = box,
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		ui.scroll {
			id = "new-work-scroll",
			absolute = true,
			left = 0,
			right = 0,
			top = top,
			bottom = 0,
			pointer_events = "auto",
			fade = { 16, 16 },
			fade_overflow = true,
			ui.column {
				w_full = true,
				items = "center",
				pb = ui.raw(48, "gpui home grid's foot room"),
				ui.column {
					id = "home-panel",
					w = width,
					rounded = "$radius.xl",
					border = 1,
					border_color = "$border",
					bg = "$zeron.home.panel",
					overflow = "hidden",
					cards.Cards(),
					ui.show(widgets.any_on, widgets.Widgets {}),
				},
			},
		},
	}
end)

return M
