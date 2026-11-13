local K = require("@app/screens/shell/whats_new_visuals/kit")

local PILL_AT, COUNT_AT, TICK, LEAVE_AT = 0.45, 0.65, 0.045, 4.6
local CHATS, DRAFTS, SETTINGS = 12, 2, 9
local TOTAL = CHATS + DRAFTS + SETTINGS
local STEP = math.ceil(TOTAL / 9)
local MOVED_AT = COUNT_AT + math.ceil(TOTAL / STEP) * TICK + 0.12

local function tab(x: number, title: string, on: boolean): Node
	return ui.row {
		absolute = true,
		top = 4,
		left = x,
		w = 82,
		h = 18,
		px = K.raw(7),
		items = "center",
		rounded_tl = K.r(6),
		rounded_tr = K.r(6),
		bg = if on then "$zeron.whats_new.panel" else nil,
		K.text(9.5, if on then "$text.primary" else "$text.secondary", title),
	}
end

local function thread(title: string, on: boolean): Node
	return ui.row {
		h = 18,
		px = K.raw(5),
		items = "center",
		gap = K.raw(5),
		rounded = K.raw(5),
		bg = if on then "$zeron.whats_new.wash_08" else nil,
		ui.div { shrink = 0, w = 5, h = 5, rounded = "$radius.full", bg = if on then "$zeron.whats_new.working" else "$text.tertiary" },
		K.clip(9.5, if on then "$text.primary" else "$text.secondary", title),
	}
end

local function build(clock: K.Clock): Node
	local moved = ui.computed(function(): boolean
		return clock.t() >= MOVED_AT
	end)
	local function settle(): number
		return K.ramp(clock.t(), 0, 400, K.EASE_OUT_QUINT)
	end
	local function shown(): number
		local t = clock.t()
		return K.ramp(t, PILL_AT, 180, K.EASE_OUT) * (1 - K.ramp(t, LEAVE_AT, 160, K.EASE_IN))
	end
	local function said(): string
		if moved:get() then
			return `Moved {CHATS} chats, {DRAFTS} drafts and {SETTINGS} settings`
		end
		local ticks = math.max(0, math.floor((clock.t() - COUNT_AT) / TICK) + 1)
		local n = if clock.t() < COUNT_AT then 0 else math.min(TOTAL, ticks * STEP)
		return `Moving your chats\u{2026} {n}/{TOTAL}`
	end
	local rail = ui.column {
		absolute = true,
		top = 26,
		left = 0,
		bottom = 0,
		w = 26,
		items = "center",
		gap = K.raw(7),
		K.glyph("chat-round-line", 11, "$text.primary"),
		K.glyph("checklist", 11, "$text.secondary"),
		K.glyph("magic-stick", 11, "$text.secondary"),
		K.glyph("settings", 11, "$text.secondary"),
	}
	local sidebar = ui.column {
		absolute = true,
		top = 26,
		left = 26,
		w = 84,
		gap = K.sp(2),
		thread("Ship the shop", true),
		thread("Fix login redirect", false),
		thread("Usage page", false),
	}
	local chat = ui.column {
		absolute = true,
		top = 22,
		left = 112,
		right = 6,
		bottom = 6,
		p = K.raw(9),
		gap = K.sp(6),
		rounded = K.raw(9),
		bg = "$zeron.whats_new.panel",
		ui.row {
			justify = "end",
			ui.row {
				h = 18,
				px = K.sp(8),
				items = "center",
				rounded = K.raw(9),
				bg = "$zeron.whats_new.wash_08",
				K.text(10, "$text.primary", "Ship the shop"),
			},
		},
		K.bar(150),
		K.bar(110),
	}
	local window = ui.div {
		relative = true,
		shrink = 0,
		w = 320,
		h = 108,
		overflow = "hidden",
		rounded = K.raw(10),
		border = 1,
		border_color = "$border",
		bg = "$zeron.whats_new.block",
		opacity = function(): number
			return K.ramp(clock.t(), 0, 300, K.EASE_OUT)
		end,
		scale = function(): number
			return 1.02 - 0.02 * settle()
		end,
		blur = function(): number
			return 8 * (1 - settle())
		end,
		tab(112, "Ship the shop", true),
		tab(196, "Fix login", false),
		rail,
		sidebar,
		chat,
	}
	local pill = ui.row {
		shrink = 0,
		h = 24,
		pl = K.sp(8),
		pr = K.sp(10),
		items = "center",
		gap = K.sp(6),
		rounded = "$radius.full",
		border = 1,
		border_color = "$border.strong",
		bg = "$zeron.whats_new.raised",
		shadow = "$shadow.md",
		opacity = shown,
		y = function(): number
			return 8 * (1 - K.ramp(clock.t(), PILL_AT, 260, K.EASE_OUT_QUINT)) + 6 * K.ramp(clock.t(), LEAVE_AT, 160, K.EASE_IN)
		end,
		ui.switch(function(): string
			return if moved:get() then "moved" else "moving"
		end, {
			moved = K.glyph("lucide-check", 12, "$zeron.whats_new.done"),
			moving = K.spinner(clock, 12),
		}),
		ui.text { said, text_size = K.raw(10.5), color = "$text.primary", tabular = true, wrap = false, shrink = 0 },
	}
	return ui.column { shrink = 0, items = "center", gap = K.sp(8), window, pill }
end

return { period = 6.0, rest = 2.4, build = build }
