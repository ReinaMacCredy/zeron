local K = require("@app/screens/shell/whats_new_visuals/kit")

local DRAG_AT, ARM_AT, COMMIT_AT = 0.5, 1.25, 2.0
local WIDTH, HEIGHT, DOT, GLYPH = 300, 96, 44, 18

local function line(width: number): Node
	return ui.div { h = 7, w = width, rounded = "$radius.full", bg = "$zeron.whats_new.wash_10" }
end

local function page(title: string, x: (() -> number)?, opacity: (() -> number)?): Node
	return ui.column {
		absolute = true,
		top = 0,
		bottom = 0,
		left = 0,
		right = 0,
		p = K.sp(12),
		pl = K.raw(24),
		gap = K.raw(7),
		x = x,
		opacity = opacity,
		ui.text { title, text_size = K.raw(11.5), weight = "$font.weight.medium", color = "$text.primary", wrap = false },
		line(170),
		line(220),
		line(140),
	}
end

local function build(clock: K.Clock): Node
	local committed = ui.computed(function(): boolean
		return clock.t() >= COMMIT_AT
	end)
	local function armed(): number
		return K.ramp(clock.t(), ARM_AT, 200, K.EASE_BOUNCE)
	end
	local function pop(): number
		return K.ramp(clock.t(), COMMIT_AT, 200, K.EASE_TAILWIND)
	end
	local function enter(): number
		return K.ramp(clock.t(), COMMIT_AT, 260, K.EASE_OUT_QUINT)
	end
	local function center(): number
		return -24 + 58 * K.ramp(clock.t(), DRAG_AT, 750, K.EASE_OUT)
	end
	local function size(): number
		return DOT * (1 + 0.1 * armed()) * (1 - 0.88 * pop())
	end
	return K.card {
		w = WIDTH,
		h = HEIGHT,
		shrink = 0,
		relative = true,
		overflow = "hidden",
		ui.switch(function(): string
			return if committed:get() then "next" else "current"
		end, {
			current = page("Ship the shop"),
			next = page("Fix login redirect", function(): number
				return -12 * (1 - enter())
			end, enter),
		}),
		ui.show(function(): boolean
			return clock.t() >= DRAG_AT and pop() < 1
		end, ui.row {
			absolute = true,
			top = function(): number
				return HEIGHT / 2 - size() / 2
			end,
			left = function(): number
				return center() - size() / 2
			end,
			w = size,
			h = size,
			items = "center",
			justify = "center",
			rounded = "$radius.full",
			border = 1,
			border_color = "$border",
			shadow = "$shadow.md",
			bg = "$zeron.whats_new.raised",
			opacity = function(): number
				return 1 - math.clamp((pop() - 0.55) / 0.45, 0, 1)
			end,
			ui.div {
				shrink = 0,
				scale = function(): number
					return (16 + 2 * armed()) * (1 - 0.88 * pop()) / GLYPH
				end,
				opacity = function(): number
					return 0.45 + 0.55 * math.min(armed(), 1)
				end,
				K.glyph("chevron-left", GLYPH, "$text.primary"),
			},
		}),
	}
end

return { period = 4.2, rest = 1.8, build = build }
