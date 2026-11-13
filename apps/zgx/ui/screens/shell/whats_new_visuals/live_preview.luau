local K = require("@app/screens/shell/whats_new_visuals/kit")

local CLICK_AT, STOP_AT, RETRY_AT = 1.5, 3.2, 4.9

local function build(clock: K.Clock): Node
	local function stopped(): number
		local t = clock.t()
		return K.ramp(t, STOP_AT, 200, K.EASE_OUT) * (1 - K.ramp(t, RETRY_AT, 200, K.EASE_OUT))
	end
	local function press(): number
		local t = clock.t()
		return K.ramp(t, CLICK_AT - 0.08, 80, K.EASE_OUT) * (1 - K.ramp(t, CLICK_AT, 120, K.EASE_OUT))
	end
	local header = ui.row {
		h = 26,
		px = K.raw(9),
		items = "center",
		gap = K.raw(7),
		border_sides = { 0, 0, 1, 0 },
		border_color = "$border",
		ui.div { grow = 1, min_w = 0, K.mono(10, "$text.secondary", "localhost:3000") },
		K.glyph("lucide-rotate-cw", 11, "$text.secondary"),
		K.glyph("arrow-up-right", 11, "$text.secondary"),
	}
	local page = ui.column {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		p = K.raw(11),
		gap = K.raw(7),
		opacity = function(): number
			return 1 - stopped()
		end,
		ui.row {
			items = "center",
			ui.text { "The shop", text_size = K.raw(11.5), weight = "$font.weight.semibold", color = "$text.primary", wrap = false, grow = 1 },
			K.text(10.5, "$text.secondary", function(): string
				return if clock.t() >= CLICK_AT then "Cart 1" else "Cart 0"
			end),
		},
		K.bar(180),
		K.bar(130),
		ui.row {
			mt = K.sp(2),
			h = 22,
			w = 82,
			items = "center",
			justify = "center",
			rounded = K.raw(7),
			bg = "$accent",
			opacity = function(): number
				return 1 - 0.25 * press()
			end,
			K.text(10.5, "$accent.text", "Add to cart"),
		},
	}
	local refused = ui.column {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		items = "center",
		justify = "center",
		gap = K.raw(7),
		opacity = stopped,
		K.text(11, "$text.secondary", "localhost:3000 is not responding"),
		ui.row {
			h = 22,
			px = K.sp(10),
			items = "center",
			rounded = K.raw(7),
			border = 1,
			border_color = "$border.strong",
			K.text(10.5, "$text.primary", "Retry"),
		},
	}
	return K.card {
		w = 320,
		shrink = 0,
		overflow = "hidden",
		header,
		ui.div {
			relative = true,
			h = 96,
			page,
			ui.show(function(): boolean
				return stopped() > 0
			end, refused),
		},
	}
end

return { period = 6.8, rest = 2.4, build = build }
