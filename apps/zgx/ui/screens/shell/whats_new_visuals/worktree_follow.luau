local K = require("@app/screens/shell/whats_new_visuals/kit")

local TOOL_AT, FOLLOW_AT, AHEAD_AT, MERGED_AT = 0.8, 1.7, 2.8, 4.4

local function build(clock: K.Clock): Node
	local merged = ui.computed(function(): boolean
		return clock.t() >= MERGED_AT
	end)
	local function tool(): number
		return K.ramp(clock.t(), TOOL_AT, 220, K.EASE_OUT)
	end
	local function follow(): number
		return K.ramp(clock.t(), FOLLOW_AT, 240, K.EASE_OUT)
	end
	local function ahead(): number
		return K.ramp(clock.t(), AHEAD_AT, 220, K.EASE_OUT)
	end
	local function branch(): string
		return if merged:get() then "merged" else "ahead"
	end
	local function state(size: number): Node
		return ui.row {
			shrink = 0,
			items = "center",
			opacity = ahead,
			ui.switch(branch, {
				merged = K.glyph("git-merge", size + 1, "$zeron.whats_new.done"),
				ahead = K.mono(size, "$text.secondary", "\u{2191}2"),
			}),
		}
	end
	local row = ui.row {
		h = 26,
		px = K.sp(8),
		items = "center",
		gap = K.raw(7),
		rounded = K.raw(7),
		bg = "$zeron.whats_new.wash_06",
		ui.switch(branch, {
			merged = K.glyph("lucide-check", 12, "$zeron.whats_new.done"),
			ahead = K.spinner(clock, 12),
		}),
		K.clip(11.5, "$text.primary", "Ship the shop"),
		state(10.5),
	}
	local call = ui.row {
		h = 20,
		items = "center",
		opacity = tool,
		K.mono(10, "$text.secondary", "git worktree add ../shop-search"),
	}
	local checkout = ui.row {
		absolute = true,
		left = 0,
		top = 0,
		bottom = 0,
		items = "center",
		gap = K.raw(5),
		opacity = function(): number
			return 1 - follow()
		end,
		K.glyph("folder", 11, "$text.secondary"),
		K.text(10.5, "$text.secondary", "Local checkout"),
		K.mono(10, "$text.tertiary", "main"),
	}
	local moved = ui.row {
		absolute = true,
		left = 0,
		top = function(): number
			return 3 * (1 - follow())
		end,
		bottom = 0,
		items = "center",
		gap = K.raw(5),
		opacity = follow,
		K.glyph("worktree", 11, "$text.primary"),
		K.text(10.5, "$text.primary", "Worktree \u{b7} shop-search"),
		K.mono(10, "$text.tertiary", "zeron/search"),
		state(10),
	}
	return K.card {
		w = 320,
		shrink = 0,
		ui.column { p = K.sp(8), gap = K.sp(4), row, ui.div { px = K.sp(8), call } },
		ui.div {
			h = 24,
			px = K.sp(8),
			border_sides = { 1, 0, 0, 0 },
			border_color = "$border",
			ui.div { relative = true, w_full = true, h_full = true, checkout, moved },
		},
	}
end

return { period = 7.0, rest = 3.6, build = build }
