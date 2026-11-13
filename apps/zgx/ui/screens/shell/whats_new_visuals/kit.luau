local Icon = require("@ds/icon")

local K = {}

export type Clock = { t: () -> number, ms: () -> number, reduced: () -> boolean }

local WHY = "gpui's What's New visual at its own size, fixed as its frame is, whatever the text size"

ui.theme.tokens {
	zeron = {
		whats_new = {
			selection = "alpha({accent}, 0.24)",
			accent_16 = "alpha({accent}, 0.16)",
			accent_rule = "alpha({accent}, 0.6)",
			panel = "over({background}, {zeron.whats_new.block})",
		},
	},
}

function K.raw(n: number): any
	return ui.raw(n, WHY)
end

local SPACE: { [number]: string } = {
	[2] = "$zeron.space.hair",
	[4] = "$space.1",
	[6] = "$zeron.space.tight",
	[8] = "$space.2",
	[10] = "$zeron.space.snug",
	[12] = "$space.3",
	[16] = "$space.4",
}
function K.sp(n: number): any
	return if n == 0 then 0 else SPACE[n] or K.raw(n)
end

local RADIUS: { [number]: string } = { [4] = "$radius.sm", [6] = "$radius.md", [8] = "$radius.lg", [12] = "$radius.xl" }
function K.r(n: number): any
	return RADIUS[n] or K.raw(n)
end

local function bezier(x1: number, y1: number, x2: number, y2: number): (number) -> number
	local function coef(a: number, b: number): (number, number, number)
		local c = 3 * a
		local bb = 3 * (b - a) - c
		return 1 - c - bb, bb, c
	end
	local ax, bx, cx = coef(x1, x2)
	local ay, by, cy = coef(y1, y2)
	local function x_at(t: number): number
		return ((ax * t + bx) * t + cx) * t
	end
	local function solve(x: number): number
		local t = x
		for _ = 1, 8 do
			local err = x_at(t) - x
			if math.abs(err) < 1e-6 then
				return t
			end
			local d = (3 * ax * t + 2 * bx) * t + cx
			if math.abs(d) < 1e-6 then
				break
			end
			t -= err / d
		end
		local lo, hi = 0, 1
		for _ = 1, 32 do
			local mid = (lo + hi) / 2
			if x_at(mid) < x then
				lo = mid
			else
				hi = mid
			end
		end
		return (lo + hi) / 2
	end
	return function(x: number): number
		if x <= 0 then
			return 0
		elseif x >= 1 then
			return 1
		end
		local t = solve(x)
		return math.clamp(((ay * t + by) * t + cy) * t, 0, 1)
	end
end

K.EASE = bezier(0.25, 0.1, 0.25, 1)
K.EASE_OUT = bezier(0, 0, 0.58, 1)
K.EASE_IN = bezier(0.42, 0, 1, 1)
K.EASE_IN_OUT = bezier(0.42, 0, 0.58, 1)
K.EASE_OUT_QUINT = bezier(0.22, 1, 0.36, 1)
K.EASE_SNAP = bezier(0.22, 1.1, 0.36, 1)
K.EASE_BOUNCE = bezier(0.3, 1.25, 0.5, 1)
K.EASE_TAILWIND = bezier(0.4, 0, 0.2, 1)

function K.ramp(t: number, start: number, ms: number, curve: (number) -> number): number
	return curve(math.clamp((t - start) * 1000 / ms, 0, 1))
end

function K.lerp(from: number, to: number, t: number): number
	return from + (to - from) * t
end

type Copy = string | () -> string
type Ink = string | () -> string

function K.text(size: number, color: Ink, copy: Copy): Node
	return ui.text { copy, text_size = K.raw(size), color = color, wrap = false, shrink = 0 }
end

function K.clip(size: number, color: Ink, copy: Copy): Node
	return ui.text { copy, text_size = K.raw(size), color = color, truncate = true, min_w = 0, grow = 1 }
end

function K.mono(size: number, color: Ink, copy: Copy): Node
	return ui.text { copy, text_size = K.raw(size), color = color, font = "$font.mono", tabular = true, wrap = false, shrink = 0 }
end

function K.glyph(name: Icon.Name, size: number, color: Ink): Node
	return ui.div { shrink = 0, color = color, Icon { name = name, size = size } }
end

function K.surface(props: { [any]: any }): { [any]: any }
	props.rounded = K.raw(10)
	props.border = 1
	props.border_color = "$border"
	props.bg = props.bg or "$zeron.whats_new.block"
	return props
end

function K.card(props: { [any]: any }): Node
	return ui.column(K.surface(props))
end

function K.bar(width: number): Node
	return ui.div { shrink = 0, h = 6, w = width, rounded = "$radius.full", bg = "$zeron.whats_new.wash_10" }
end

function K.typed(copy: string, t: number, start: number, per_char: number): string
	return string.sub(copy, 1, math.clamp(math.floor((t - start) / per_char), 0, #copy))
end

function K.caret(clock: Clock): Node
	return ui.div {
		shrink = 0,
		w = 1.5,
		h = 14,
		bg = "$text.primary",
		opacity = function(): number
			return if (clock.t() * 2) % 1 < 0.5 then 1 else 0
		end,
	}
end

function K.key_chip(label: string): Node
	return ui.row {
		shrink = 0,
		h = 15,
		px = K.sp(4),
		items = "center",
		rounded = K.r(4),
		border = 1,
		border_color = "$border",
		K.text(9.5, "$text.tertiary", label),
	}
end

function K.question_mark(): Node
	return ui.row {
		shrink = 0,
		w = 12,
		h = 12,
		items = "center",
		justify = "center",
		rounded = "$radius.full",
		border = 1,
		border_color = "$zeron.whats_new.accent_edge",
		ui.text { "?", text_size = K.raw(8.5), weight = "$font.weight.bold", color = "$accent", wrap = false },
	}
end

function K.spinner(clock: Clock, size: number): Node
	local scale = size / 24
	local radius, width = 8.5 * scale, 2.5 * scale
	return ui.overlay {
		shrink = 0,
		w = size,
		h = size,
		ui.canvas {
			w = size,
			h = size,
			draw = function(c, w, h)
				c:stroke(ui.path():arc(w / 2, h / 2, radius, 0, 360), "$zeron.whats_new.track", width)
			end,
		},
		ui.canvas {
			w = size,
			h = size,
			rotate = function(): number
				return if clock.reduced() then 0 else (clock.ms() / 900 % 1) * 360
			end,
			draw = function(c, w, h)
				c:stroke(ui.path():arc(w / 2, h / 2, radius, -90, 90), "$zeron.whats_new.working", width, { cap = "round" })
			end,
		},
	}
end

return K
