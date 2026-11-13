local K = require("@app/screens/shell/whats_new_visuals/kit")

local SELECT_AT, PILL_AT, ADD_AT, SAVE_AT = 0.4, 1.2, 2.1, 3.7

local function segment(label: string, pad: number): Node
	return ui.row { h_full = true, px = K.sp(pad), items = "center", K.text(11, "$text.primary", label) }
end

local function float(props: { [any]: any }): Node
	props.absolute = true
	props.items = "center"
	props.border = 1
	props.border_color = "$border.strong"
	props.bg = "$zeron.whats_new.raised"
	props.shadow = "$shadow.md"
	return ui.row(props)
end

local function visual(clock: K.Clock, hero: boolean): Node
	local function pick(): number
		return K.ramp(clock.t(), SELECT_AT, 600, K.EASE_OUT)
	end
	local function pill_in(): number
		local t = clock.t()
		return K.ramp(t, PILL_AT, 180, K.EASE_OUT) * (1 - K.ramp(t, ADD_AT, 120, K.EASE_IN))
	end
	local function editor_in(): number
		local t = clock.t()
		return K.ramp(t, ADD_AT, 180, K.EASE_OUT) * (1 - K.ramp(t, SAVE_AT, 140, K.EASE_IN))
	end
	local function saved(): number
		return K.ramp(clock.t(), SAVE_AT, 260, K.EASE_BOUNCE)
	end
	local function comment(): string
		return K.typed("Use the real total", clock.t(), ADD_AT + 0.25, 0.05)
	end
	local inset = if hero then 8 else 38
	local pad = if hero then 8 else 10
	local badge = ui.row {
		absolute = true,
		top = function(): number
			return -15 + 4 * (1 - saved())
		end,
		right = -7,
		w = 15,
		h = 15,
		items = "center",
		justify = "center",
		rounded = "$radius.full",
		bg = "$accent",
		opacity = function(): number
			return math.min(saved(), 1)
		end,
		K.text(9.5, "$accent.text", "1"),
	}
	local selected = ui.row {
		relative = true,
		shrink = 0,
		ui.div {
			absolute = true,
			top = -1,
			bottom = -1,
			left = 0,
			w = function(): string
				return `{pick() * 100}%`
			end,
			rounded = K.raw(3),
			bg = "$zeron.whats_new.selection",
		},
		K.text(11.5, "$text.primary", "shipping is added at checkout"),
		ui.show(function(): boolean
			return saved() > 0
		end, badge),
	}
	local pill = float {
		top = function(): number
			return 57 + 4 * (1 - pill_in())
		end,
		left = inset,
		h = 26,
		rounded = K.raw(13),
		opacity = pill_in,
		segment("Add to chat", pad),
		ui.div { w = 1, h_full = true, bg = "$border" },
		segment("Ask in side chat", pad),
	}
	local editor = float {
		top = function(): number
			return 55 + 4 * (1 - editor_in())
		end,
		left = inset,
		right = if hero then inset else 46,
		h = 30,
		pl = K.raw(11),
		pr = K.sp(4),
		gap = K.sp(6),
		rounded = K.raw(15),
		opacity = editor_in,
		ui.row {
			grow = 1,
			min_w = 0,
			items = "center",
			ui.switch(function(): string
				return if comment() == "" then "empty" else "typed"
			end, {
				empty = K.clip(11, "$text.tertiary", "Add an optional comment..."),
				typed = ui.row { items = "center", K.text(11, "$text.primary", comment), K.caret(clock) },
			}),
		},
		ui.row {
			shrink = 0,
			w = 22,
			h = 22,
			items = "center",
			justify = "center",
			rounded = "$radius.full",
			bg = "$text.primary",
			K.glyph("arrow-up", 11, "$background"),
		},
	}
	local reply = ui.column {
		absolute = true,
		top = 12,
		left = 14,
		gap = K.raw(5),
		K.text(11.5, "$text.primary", "The cart adds it up, and"),
		ui.row { items = "center", selected, K.text(11.5, "$text.primary", ".") },
	}
	local chip = ui.row {
		shrink = 0,
		h = 20,
		px = K.sp(8),
		items = "center",
		gap = K.sp(4),
		rounded = K.raw(7),
		border = 1,
		border_color = "$border",
		opacity = function(): number
			return math.min(saved(), 1)
		end,
		K.glyph("chat-round-line", 10, "$text.secondary"),
		K.text(10.5, "$text.primary", "1 annotation"),
	}
	local composer = ui.row {
		absolute = true,
		left = 8,
		right = 8,
		bottom = 8,
		h = 32,
		px = K.sp(6),
		items = "center",
		gap = K.raw(7),
		rounded = K.raw(10),
		border = 1,
		border_color = "$border.strong",
		ui.show(function(): boolean
			return saved() > 0
		end, chip),
		K.clip(11, "$text.tertiary", "Message"),
	}
	local card = K.card {
		w = if hero then nil else 320,
		w_full = if hero then true else nil,
		h = if hero then 92 else 136,
		shrink = 0,
		relative = true,
		overflow = "hidden",
		reply,
		ui.show(function(): boolean
			return pill_in() > 0
		end, pill),
		ui.show(function(): boolean
			return editor_in() > 0
		end, editor),
		if hero then nil else composer,
	}
	if hero then
		return ui.column { w_full = true, px = K.sp(6), card }
	end
	return card
end

return {
	card = {
		period = 6.6,
		rest = 5.2,
		build = function(clock: K.Clock): Node
			return visual(clock, false)
		end,
	},
	hero = {
		period = 6.6,
		rest = 5.2,
		build = function(clock: K.Clock): Node
			return visual(clock, true)
		end,
	},
}
