--!strict
local motion = require("@self/motion")

local MOVE = { duration = 260, ease = "out(3)" }
-- (Wider than any chip: its room until its content is measured.)
local ROOM = 400

return ui.component(function(props: { shown: () -> boolean, [number]: Node }): Node
	local open = ui.computed(function(): boolean
		return props.shown() == true
	end)
	local present = ui.use_state(open:peek())
	local moving = ui.use_state(false)
	local natural = ui.use_state(0)
	local built = false
	ui.effect(function()
		local on = open:get()
		natural:get()
		if not built then
			built = true
			return
		end
		if on and not present:peek() then
			present:set(true)
		end
		moving:set(true)
		ui.after(motion.ms(MOVE.duration), function()
			-- (Read now, not as it was: shown again meanwhile, an earlier close mustn't unmount it.)
			if open:peek() ~= on then
				return
			end
			moving:set(false)
			if not on then
				present:set(false)
			end
		end)
	end)
	return ui.row {
		shrink = 0,
		items = "center",
		hidden = function(): boolean
			return not present:get()
		end,
		-- (Clipped only while it moves: at rest, its content's focus ring stands outside it.)
		overflow = function(): string
			return if moving:get() then "hidden" else "visible"
		end,
		min_w = motion.tween(
			ui.computed(function(): number
				return if open:get() then natural:get() else 0
			end),
			MOVE
		),
		max_w = motion.tween(
			ui.computed(function(): number
				if not open:get() then
					return 0
				end
				local w = natural:get()
				return if w > 0 then w else ROOM
			end),
			MOVE
		),
		opacity = motion.tween(
			ui.computed(function(): number
				return if open:get() then 1 else 0
			end),
			MOVE
		),
		ui.row {
			shrink = 0,
			items = "center",
			on_resize = function(size: { w: number, h: number })
				if math.abs(size.w - natural:peek()) >= 0.5 then
					natural:set(size.w)
				end
			end,
			ui.show(function(): boolean
				return present:get()
			end, props[1]),
		},
	}
end)
