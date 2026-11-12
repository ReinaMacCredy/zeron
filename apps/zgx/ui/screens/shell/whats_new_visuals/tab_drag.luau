local K = require("@app/screens/shell/whats_new_visuals/kit")

local DRAG_AT, OUT_AT, AIM_AT, DROP_AT = 0.4, 1.5, 2.1, 3.0
local WIDTH, HEIGHT, STRIP, TAB_W = 300, 96, 24, 84
local SLOT = TAB_W + 4
local PAGE = "$zeron.whats_new.block"
local lerp = K.lerp

local function line(width: number): Node
	return ui.div { h = 6, w = width, shrink = 0, rounded = "$radius.full", bg = "$zeron.whats_new.wash_10" }
end

local function column(title: string, props: { [any]: any }): Node
	props.min_w = 0
	props.h_full = true
	props.p = K.sp(10)
	props.gap = K.sp(6)
	props.overflow = "hidden"
	table.insert(props, ui.text { title, text_size = K.raw(10.5), weight = "$font.weight.medium", color = "$text.primary", wrap = false })
	table.insert(props, line(110))
	table.insert(props, line(80))
	return ui.column(props)
end

local function build(clock: K.Clock): Node
	local function slide(): number
		return K.ramp(clock.t(), DRAG_AT, 900, K.EASE_OUT) * 2 * SLOT
	end
	local function out(): number
		return K.ramp(clock.t(), OUT_AT, 320, K.EASE_OUT)
	end
	local function aim(): number
		return K.ramp(clock.t(), AIM_AT, 600, K.EASE_OUT)
	end
	local function hot(): number
		return K.ramp(clock.t(), AIM_AT + 0.4, 160, K.EASE_OUT)
	end
	local function enter(): number
		return K.ramp(clock.t(), DRAG_AT, 120, K.EASE_OUT)
	end
	local function split(): number
		return K.ramp(clock.t(), DROP_AT, 260, K.EASE_OUT_QUINT)
	end
	local function sibling(slot: number): number
		local pass = math.clamp(slide() / SLOT - slot + 0.5, 0, 1)
		return 8 + SLOT * slot - SLOT * K.EASE_OUT(pass)
	end
	local function tab(title: string, slot: number, selected: () -> boolean): Node
		return ui.row {
			absolute = true,
			top = 4,
			left = 0,
			x = function(): number
				return sibling(slot)
			end,
			w = TAB_W,
			h = STRIP - 4,
			px = K.sp(8),
			items = "center",
			rounded_tl = K.raw(7),
			rounded_tr = K.raw(7),
			bg = function(): string
				return if selected() then PAGE else "#00000000"
			end,
			K.text(10, function(): string
				return if selected() then "$text.primary" else "$text.secondary"
			end, title),
		}
	end
	local function split_card(left: boolean, lit: () -> number): Node
		local function w(): number
			return lerp(32, lerp(46, 60, lit()), out())
		end
		local function inset(): number
			return lerp(-w() - 12, 8, enter())
		end
		return ui.div {
			absolute = true,
			top = STRIP + 8,
			left = if left then inset else nil,
			right = if left then nil else inset,
			w = w,
			h = function(): number
				return lerp(40, HEIGHT - STRIP - 16, out())
			end,
			rounded = K.raw(10),
			bg = "$zeron.whats_new.split.fill",
			shadow = "$shadow.md",
			p = K.raw(3),
			opacity = function(): number
				return 1 - split()
			end,
			ui.overlay {
				w_full = true,
				h_full = true,
				rounded = "$radius.lg",
				border = 1,
				border_dash = { 3, 3 },
				border_color = "$zeron.whats_new.split.rule",
				items = "center",
				justify = "center",
				K.glyph("split-columns", 11, "$text.secondary"),
				ui.div { opacity = lit, K.glyph("split-columns", 11, "$text.primary") },
			},
		}
	end
	local page = ui.row {
		absolute = true,
		top = STRIP,
		left = 0,
		right = 0,
		bottom = 0,
		bg = PAGE,
		ui.show(function(): boolean
			return split() > 0
		end, column("Ship the shop", {
			shrink = 0,
			w = function(): number
				return WIDTH / 2 * split()
			end,
			border_sides = { 0, 1, 0, 0 },
			border_color = "$border",
		})),
		column("Fix login redirect", { grow = 1 }),
	}
	local thumb = ui.column {
		absolute = true,
		left = 0,
		top = 0,
		x = function(): number
			return lerp(8 + slide(), lerp(170, 30, aim()), out())
		end,
		y = function(): number
			return lerp(4, lerp(40, 44, aim()), out())
		end,
		w = function(): number
			return lerp(TAB_W, 64, out())
		end,
		h = function(): number
			return lerp(STRIP - 4, 40, out())
		end,
		rounded = function(): any
			return K.raw(lerp(7, 8, out()))
		end,
		bg = PAGE,
		border = function(): number
			return if out() > 0 then 1 else 0
		end,
		border_color = "$border.strong",
		shadow = function(): any
			return if out() > 0 then "$shadow.lg" else false
		end,
		px = K.sp(8),
		pt = function(): any
			return K.raw(lerp(4, 6, out()))
		end,
		gap = K.sp(4),
		items = "start",
		opacity = function(): number
			return 1 - split()
		end,
		K.text(10, "$text.primary", "Ship the shop"),
		ui.div { opacity = out, line(54) },
		ui.div { opacity = out, line(40) },
	}
	return K.card {
		w = WIDTH,
		h = HEIGHT,
		shrink = 0,
		relative = true,
		overflow = "hidden",
		page,
		tab("Fix login redirect", 1, function(): boolean
			return split() > 0
		end),
		tab("Usage page", 2, function(): boolean
			return false
		end),
		ui.show(function(): boolean
			return clock.t() >= DRAG_AT and split() < 1
		end, ui.overlay {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			split_card(true, hot),
			split_card(false, function(): number
				return 0
			end),
		}),
		ui.show(function(): boolean
			return split() < 1
		end, ui.overlay { absolute = true, top = 0, left = 0, right = 0, bottom = 0, thumb }),
	}
end

return { period = 4.4, rest = 1.8, build = build }
