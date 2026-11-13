local K = require("@app/screens/shell/whats_new_visuals/kit")

local HOVER_AT, DIALOG_AT, TRY_AT = 0.5, 1.1, 3.0

local function nav(label: string, on: boolean): Node
	return ui.row {
		h = 20,
		px = K.raw(7),
		items = "center",
		rounded = K.raw(5),
		bg = if on then "$zeron.whats_new.wash_08" else nil,
		K.text(10.5, if on then "$text.primary" else "$text.secondary", label),
	}
end

local function row(title: string, harness: string, on: () -> boolean): Node
	return ui.row {
		h = 24,
		px = K.raw(7),
		items = "center",
		gap = K.raw(7),
		rounded = K.r(6),
		bg = function(): string
			return if on() then "$zeron.whats_new.wash_06" else "#00000000"
		end,
		K.glyph("magic-stick", 11, "$text.secondary"),
		K.clip(11, "$text.primary", title),
		K.text(9.5, "$text.tertiary", harness),
	}
end

local function never(): boolean
	return false
end

local function build(clock: K.Clock): Node
	local function dialog(): number
		local t = clock.t()
		return K.ramp(t, DIALOG_AT, 220, K.EASE_OUT) * (1 - K.ramp(t, TRY_AT, 160, K.EASE_IN))
	end
	local function used(): number
		return K.ramp(clock.t(), TRY_AT + 0.15, 220, K.EASE_OUT)
	end
	local function hovered(): boolean
		local t = clock.t()
		return t >= HOVER_AT and t < TRY_AT
	end
	local page = ui.row {
		absolute = true,
		top = 8,
		left = 8,
		right = 8,
		gap = K.sp(8),
		ui.column { shrink = 0, w = 84, gap = K.sp(2), nav("All skills", true), nav("Claude Code", false), nav("Codex", false) },
		ui.column {
			grow = 1,
			min_w = 0,
			gap = K.sp(2),
			row("Release notes", "Claude Code", never),
			row("Shop style", "Codex", hovered),
			row("Test plan", "Claude Code", never),
		},
	}
	local sheet = K.card {
		absolute = true,
		top = function(): number
			return 10 + 6 * (1 - dialog())
		end,
		left = 70,
		w = 180,
		p = K.sp(10),
		gap = K.sp(6),
		shadow = "$shadow.md",
		bg = "$zeron.whats_new.raised",
		opacity = dialog,
		ui.text { "Shop style", text_size = K.raw(12), weight = "$font.weight.semibold", color = "$text.primary", wrap = false },
		K.bar(150),
		K.bar(110),
		ui.row {
			mt = K.sp(2),
			h = 22,
			w = 74,
			items = "center",
			justify = "center",
			gap = K.raw(5),
			rounded = "$radius.full",
			bg = "$text.primary",
			K.glyph("pen-new-square", 10, "$background"),
			K.text(10.5, "$background", "Try now"),
		},
	}
	local skill = ui.row {
		shrink = 0,
		h = 20,
		px = K.raw(7),
		items = "center",
		gap = K.sp(4),
		rounded = K.r(6),
		bg = "$zeron.whats_new.accent_16",
		opacity = used,
		K.glyph("magic-stick", 10, "$accent"),
		K.text(10.5, "$accent", "Shop style"),
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
			return used() > 0
		end, skill),
		K.clip(11, "$text.tertiary", "Message"),
	}
	return K.card {
		w = 320,
		h = 136,
		shrink = 0,
		relative = true,
		overflow = "hidden",
		page,
		composer,
		ui.show(function(): boolean
			return dialog() > 0
		end, sheet),
	}
end

return { period = 6.0, rest = 4.4, build = build }
