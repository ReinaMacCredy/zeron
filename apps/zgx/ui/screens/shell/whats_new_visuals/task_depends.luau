local K = require("@app/screens/shell/whats_new_visuals/kit")

local TYPE_AT, MENU_AT, PICK_AT, SAVE_AT, DONE_AT = 0.3, 1.1, 2.1, 3.0, 4.6

local function todo(size: number): Node
	return K.glyph("lucide-circle-dashed", size, "$text.tertiary")
end

local function struck(size: number, copy: string): Node
	return ui.rich { ui.span { copy, strike = true }, text_size = K.raw(size), color = "$text.tertiary", wrap = false, shrink = 0 }
end

local function option(label: string, on: boolean): Node
	return ui.row {
		h = 22,
		px = K.raw(7),
		items = "center",
		gap = K.sp(6),
		rounded = K.raw(5),
		bg = if on then "$zeron.whats_new.wash_08" else nil,
		todo(10),
		K.clip(10.5, "$text.primary", label),
	}
end

local function build(clock: K.Clock): Node
	local done = ui.computed(function(): boolean
		return clock.t() >= DONE_AT
	end)
	local function state(): string
		return if done:get() then "done" else "open"
	end
	local function menu(): number
		local t = clock.t()
		return K.ramp(t, MENU_AT, 160, K.EASE_OUT) * (1 - K.ramp(t, PICK_AT, 120, K.EASE_IN))
	end
	local function picked(): number
		return K.ramp(clock.t(), PICK_AT, 160, K.EASE_OUT)
	end
	local function capture(): number
		return 1 - K.ramp(clock.t(), SAVE_AT, 180, K.EASE_IN)
	end
	local function list(): number
		return K.ramp(clock.t(), SAVE_AT + 0.1, 240, K.EASE_OUT)
	end
	local bar_row = ui.row(K.surface {
		absolute = true,
		top = function(): number
			return 10 - 6 * (1 - capture())
		end,
		left = 10,
		right = 10,
		h = 36,
		px = K.sp(10),
		items = "center",
		gap = K.raw(3),
		bg = "$zeron.whats_new.raised",
		opacity = capture,
		K.text(11.5, "$text.primary", function(): string
			return K.typed("Ship checkout ", clock.t(), TYPE_AT, 0.05)
		end),
		ui.show(function(): boolean
			local t = clock.t()
			return t >= MENU_AT and t < PICK_AT
		end, K.text(11.5, "$accent", "@")),
		ui.show(function(): boolean
			return picked() > 0
		end, ui.row {
			shrink = 0,
			h = 18,
			px = K.raw(5),
			items = "center",
			gap = K.raw(3),
			rounded = K.raw(5),
			bg = "$zeron.whats_new.wash_08",
			opacity = picked,
			todo(9),
			K.text(10.5, "$text.primary", "Fix login redirect"),
		}),
		ui.show(function(): boolean
			return picked() == 0
		end, K.caret(clock)),
	})
	local mentions = K.card {
		absolute = true,
		top = function(): number
			return 50 + 4 * (1 - menu())
		end,
		left = 10,
		w = 210,
		p = K.sp(4),
		bg = "$zeron.whats_new.raised",
		shadow = "$shadow.md",
		opacity = menu,
		option("Fix login redirect", true),
		option("Write the release notes", false),
	}
	local blocker = ui.row {
		h = 16,
		items = "center",
		gap = K.sp(4),
		K.text(10, "$text.tertiary", function(): string
			return if done:get() then "Depends on" else "Blocked by"
		end),
		ui.switch(state, {
			done = K.glyph("lucide-check", 10, "$zeron.whats_new.done"),
			open = todo(9),
		}),
		ui.switch(state, {
			done = struck(10, "Fix login redirect"),
			open = K.text(10, "$text.secondary", "Fix login redirect"),
		}),
	}
	local tasks = ui.column {
		absolute = true,
		top = function(): number
			return 10 + 6 * (1 - list())
		end,
		left = 10,
		right = 10,
		gap = K.sp(6),
		opacity = list,
		ui.row { items = "center", gap = K.raw(7), todo(12), ui.column { K.text(11.5, "$text.primary", "Ship checkout"), blocker } },
		ui.row {
			items = "center",
			gap = K.raw(7),
			ui.switch(state, {
				done = K.glyph("lucide-check", 12, "$zeron.whats_new.done"),
				open = todo(12),
			}),
			ui.switch(state, {
				done = struck(11.5, "Fix login redirect"),
				open = K.text(11.5, "$text.primary", "Fix login redirect"),
			}),
		},
	}
	return K.card {
		w = 320,
		h = 116,
		shrink = 0,
		relative = true,
		overflow = "hidden",
		ui.show(function(): boolean
			return capture() > 0
		end, bar_row),
		ui.show(function(): boolean
			return menu() > 0
		end, mentions),
		ui.show(function(): boolean
			return list() > 0
		end, tasks),
	}
end

return { period = 7.0, rest = 4.0, build = build }
