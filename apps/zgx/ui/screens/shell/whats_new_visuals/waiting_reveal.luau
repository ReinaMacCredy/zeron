local K = require("@app/screens/shell/whats_new_visuals/kit")

local ASK_AT, FOLD_AT = 0.8, 4.8
local OPEN_W = 196

local function frame(props: { [any]: any }): Node
	props.shrink = 0
	props.h = 22
	props.px = K.raw(9)
	props.items = "center"
	props.gap = K.raw(5)
	props.rounded = "$radius.full"
	props.border = 1
	props.border_color = "$border.strong"
	props.bg = "$zeron.whats_new.block"
	return ui.row(props)
end

local function build(clock: K.Clock): Node
	local asked = ui.computed(function(): boolean
		return clock.t() >= ASK_AT
	end)
	local function open(): number
		local t = clock.t()
		return K.ramp(t, ASK_AT, 240, K.EASE_OUT) * (1 - K.ramp(t, FOLD_AT, 240, K.EASE_IN))
	end
	local agents = frame {
		K.spinner(clock, 12),
		K.text(11, "$text.secondary", "Working"),
	}
	local waiting = frame {
		K.glyph("clock-circle", 12, "$warning"),
		K.text(11, "$warning", function(): string
			return if asked:get() then "2" else "1"
		end),
		K.text(11, "$warning", function(): string
			return if asked:get() then "questions waiting" else "question waiting"
		end),
		ui.show(function(): boolean
			return open() > 0
		end, ui.row {
			shrink = 0,
			overflow = "hidden",
			items = "center",
			gap = K.raw(5),
			w = function(): number
				return OPEN_W * open()
			end,
			opacity = open,
			ui.div { shrink = 0, w = 1, h = 10, mx = K.raw(3), bg = "$border.strong" },
			K.text(11, "$text.primary", "Checkout"),
			K.text(11, "$text.secondary", "asks: Stripe or Paddle?"),
		}),
	}
	return ui.column {
		shrink = 0,
		w_full = true,
		gap = K.sp(4),
		ui.row { gap = K.raw(5), agents, waiting },
		K.card {
			h = 30,
			px = K.sp(8),
			justify = "center",
			K.text(11, "$text.tertiary", "Message"),
		},
	}
end

return { period = 6.0, rest = 1.2, build = build }
