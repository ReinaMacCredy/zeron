local K = require("@app/screens/shell/whats_new_visuals/kit")

local ON_AT, OFF_AT = 1.3, 4.1

local function bullet(copy: string): Node
	return ui.row {
		items = "center",
		gap = K.raw(7),
		ui.div { shrink = 0, w = 4, h = 4, rounded = "$radius.full", bg = "$text.secondary" },
		K.text(11, "$text.primary", copy),
	}
end

local function fill(props: { [any]: any }): Node
	props.absolute = true
	props.top = 0
	props.left = 0
	props.right = 0
	props.bottom = 0
	props.px = K.sp(12)
	props.py = K.sp(10)
	return ui.column(props)
end

local function build(clock: K.Clock): Node
	local function rendered(): number
		local t = clock.t()
		return K.ramp(t, ON_AT, 260, K.EASE_OUT) * (1 - K.ramp(t, OFF_AT, 260, K.EASE_OUT))
	end
	local function on(): boolean
		local t = clock.t()
		return t >= ON_AT and t < OFF_AT
	end
	local function press(): number
		local t = clock.t()
		local function tap(at: number): number
			return K.ramp(t, at - 0.08, 80, K.EASE_OUT) * (1 - K.ramp(t, at, 120, K.EASE_OUT))
		end
		return math.max(tap(ON_AT), tap(OFF_AT))
	end
	local eye = ui.row {
		shrink = 0,
		w = 20,
		h = 20,
		items = "center",
		justify = "center",
		rounded = K.r(6),
		bg = function(): string
			return if on() then "$zeron.whats_new.wash_10" else "#00000000"
		end,
		scale = function(): number
			return 1 - 0.12 * press()
		end,
		ui.switch(function(): string
			return if on() then "on" else "off"
		end, {
			on = K.glyph("eye", 12, "$text.primary"),
			off = K.glyph("eye", 12, "$text.secondary"),
		}),
	}
	local header = ui.row {
		h = 26,
		pl = K.sp(10),
		pr = K.sp(4),
		items = "center",
		gap = K.sp(4),
		border_sides = { 0, 0, 1, 0 },
		border_color = "$border",
		ui.div { grow = 1, min_w = 0, K.mono(10, "$text.tertiary", "markdown") },
		eye,
		ui.row {
			shrink = 0,
			h = 20,
			px = K.sp(6),
			items = "center",
			gap = K.sp(4),
			K.glyph("copy", 11, "$text.secondary"),
			K.text(10, "$text.secondary", "Copy"),
		},
	}
	local source = fill {
		gap = K.sp(4),
		opacity = function(): number
			return 1 - rendered()
		end,
		K.mono(10.5, "$text.primary", "# Ship the shop"),
		K.mono(10.5, "$text.secondary", "- Real cart total"),
		K.mono(10.5, "$text.secondary", "- Fix login redirect"),
	}
	local view = fill {
		gap = K.sp(6),
		opacity = rendered,
		y = function(): number
			return 4 * (1 - rendered())
		end,
		ui.text { "Ship the shop", text_size = K.raw(13), weight = "$font.weight.semibold", color = "$text.primary", wrap = false },
		bullet("Real cart total"),
		bullet("Fix login redirect"),
	}
	return K.card {
		w = 320,
		shrink = 0,
		overflow = "hidden",
		header,
		ui.div { relative = true, h = 84, source, view },
	}
end

return { period = 5.6, rest = 2.4, build = build }
