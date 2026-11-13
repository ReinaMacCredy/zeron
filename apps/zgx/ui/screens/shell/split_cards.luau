local ds = require("@ds")
local motion = require("@ds/motion")

local M = {}

local EASE_OUT = "cubicBezier(0,0,.58,1)"
local ENTER_MS, GROW_MS = 120, 160

local function lerp(a: number, b: number, t: number): number
	return a + (b - a) * t
end

local function follow(on: () -> boolean, ms: number, from: number): any
	local t = ui.use_state(from)
	local running: any = nil
	ui.effect(function()
		local to = if on() then 1 else 0
		if running then
			running:cancel()
		end
		running = anime.animate(t, motion.params({ value = to, duration = ms, ease = EASE_OUT }))
	end)
	return t
end

local function dark(): boolean
	return string.find(ui.theme.kind(), "dark", 1, true) ~= nil
end

local function card(pane: string, side: string, layer: any, grow: any, lit: any, enter: any): Node
	local function width(): number
		return lerp(120, lerp(160, 200, lit:get()), grow:get())
	end
	local inset = function(): number
		return ui.raw(lerp(-width() - 30, 12, enter:get()), "gpui's split card sliding in from its edge")
	end
	return ui.column {
		id = `split-card-{pane}-{side}`,
		absolute = true,
		left = if side == "left" then inset else nil,
		right = if side == "right" then inset else nil,
		top = function(): number
			return ui.raw(lerp(22, 18, grow:get()), "gpui's split card")
		end,
		w = function(): number
			return ui.raw(width(), "gpui's split card")
		end,
		h = function(): number
			local h = layer.height or 0
			return ui.raw(lerp(math.clamp(h * 0.42, 160, 320), math.max(h - 36, 160), grow:get()), "gpui's split card")
		end,
		rounded = ui.raw(16, "gpui's split card"),
		bg = function(): string
			return ui.raw(if dark() then "#050505" else "#ffffff", "gpui's split card: Helium's black on dark, white on light")
		end,
		border = 1,
		border_color = function(): string
			return ui.raw(if dark() then "#ffffff12" else "#0000000d", "gpui's split card ring")
		end,
		shadow = "$shadow.lg",
		items = "center",
		justify = "center",
		gap = ui.raw(6, "gpui's split card"),
		pointer_events = "none",
		color = motion.tween(ui.computed(function(): boolean
			return lit:get() > 0.5
		end), { from = "$text.secondary", to = "$text.primary", duration = GROW_MS, ease = EASE_OUT }),
		ui.div {
			absolute = true,
			left = 5,
			right = 5,
			top = 5,
			bottom = 5,
			rounded = ui.raw(12, "gpui's split card's rule"),
			border = 1,
			border_dash = { 2, 1 },
			border_color = function(): string
				return ui.raw(if dark() then "#ffffff29" else "#00000024", "gpui's split card rule")
			end,
		},
		ds.Icon { name = "split-columns", size = 16 },
	}
end

export type Props = {
	pane: string,
	shown: () -> boolean,
	tall: () -> boolean,
	hot: () -> string?,
	inert: () -> boolean,
	bottom: () -> any,
}

function M.Cards(props: Props): Node
	local layer = ui.ref()
	local enter = follow(props.shown, ENTER_MS, 0)
	local grow = follow(props.tall, GROW_MS, if props.tall() then 1 else 0)
	local left = follow(function(): boolean
		return props.hot() == "left"
	end, GROW_MS, 0)
	local right = follow(function(): boolean
		return props.hot() == "right"
	end, GROW_MS, 0)
	return ui.div {
		ref = layer,
		id = `split-cards-{props.pane}`,
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = props.bottom,
		overflow = "hidden",
		rounded = "$zeron.inset.radius",
		opacity = function(): number
			return if props.inert() then 0.3 else 1
		end,
		pointer_events = "none",
		card(props.pane, "left", layer, grow, left, enter),
		card(props.pane, "right", layer, grow, right, enter),
	}
end

return M
