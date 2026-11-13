local K = require("@app/screens/shell/whats_new_visuals/kit")

local MODES = { { label = "Off", at = 0 }, { label = "Dark", at = 1.8 }, { label = "Glass", at = 3.6 } }
local WIDTH = 300
local ART = { angle = 120, stops = { "#306da6", "#bf40ab" } }
local ON_ART = ui.raw("#ffffffe6", "gpui's white at 0.9 on the artwork in either theme")
local SHADE = ui.raw("#000000", "gpui's black frame shade over the artwork in either theme")

local function layer(props: { [any]: any }): Node
	props.absolute = true
	props.top = 0
	props.left = 0
	props.right = 0
	props.bottom = 0
	props.rounded = K.raw(9)
	return ui.div(props)
end

local function build(clock: K.Clock): Node
	local function dark(): number
		return K.ramp(clock.t(), MODES[2].at, 320, K.EASE_OUT)
	end
	local function glass(): number
		return K.ramp(clock.t(), MODES[3].at, 320, K.EASE_OUT)
	end
	local current = ui.computed(function(): number
		local t, at = clock.t(), 1
		for ix, mode in MODES do
			if t >= mode.at then
				at = ix
			end
		end
		return at
	end)
	local function on_art(): boolean
		return dark() > 0.5
	end
	local function tab(x: number, title: string, on: boolean): Node
		return ui.row {
			absolute = true,
			top = 5,
			left = x,
			w = 78,
			h = 17,
			px = K.raw(7),
			items = "center",
			rounded_tl = K.r(6),
			rounded_tr = K.r(6),
			bg = if on then "$zeron.whats_new.panel" else nil,
			K.text(9.5, function(): string
				return if on then "$text.primary" elseif on_art() then ON_ART else "$text.secondary"
			end, title),
		}
	end
	local function rail_ink(): string
		return if on_art() then ON_ART else "$text.secondary"
	end
	local frame = layer {
		overflow = "hidden",
		layer { gradient = ART },
		layer {
			bg = SHADE,
			opacity = function(): number
				return K.lerp(0.5, 0.14, glass())
			end,
		},
		layer {
			bg = "$background",
			opacity = function(): number
				return 1 - dark()
			end,
		},
	}
	local rail = ui.column {
		absolute = true,
		top = 28,
		left = 7,
		gap = K.raw(7),
		K.glyph("chat-round-line", 11, rail_ink),
		K.glyph("checklist", 11, rail_ink),
		K.glyph("magic-stick", 11, rail_ink),
	}
	local inset = ui.column {
		absolute = true,
		top = 22,
		left = 26,
		right = 6,
		bottom = 6,
		p = K.raw(9),
		gap = K.sp(6),
		rounded = K.raw(9),
		bg = "$zeron.whats_new.panel",
		K.bar(150),
		K.bar(200),
		K.bar(120),
	}
	local window = ui.div {
		relative = true,
		shrink = 0,
		w = WIDTH,
		h = 100,
		overflow = "hidden",
		rounded = K.raw(10),
		border = 1,
		border_color = "$border",
		frame,
		tab(26, "Ship the shop", true),
		tab(108, "Fix login", false),
		rail,
		inset,
	}
	local options: { Node } = {}
	for ix, mode in MODES do
		local function on(): boolean
			return current:get() == ix
		end
		table.insert(options, ui.row {
			h = 20,
			px = K.sp(10),
			items = "center",
			rounded = K.r(6),
			bg = function(): string
				return if on() then "$zeron.whats_new.wash_10" else "#00000000"
			end,
			K.text(10.5, function(): string
				return if on() then "$text.primary" else "$text.secondary"
			end, mode.label),
		})
	end
	return ui.column {
		shrink = 0,
		items = "center",
		gap = K.sp(8),
		window,
		ui.row {
			items = "center",
			gap = K.sp(8),
			K.text(10.5, "$text.secondary", "Window frame"),
			ui.row {
				p = K.sp(2),
				gap = K.sp(2),
				rounded = K.r(8),
				border = 1,
				border_color = "$border",
				table.unpack(options),
			},
		},
	}
end

return { period = 5.6, rest = 2.6, build = build }
