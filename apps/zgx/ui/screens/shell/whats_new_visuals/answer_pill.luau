local K = require("@app/screens/shell/whats_new_visuals/kit")

local OPEN_AT, PICK_AT, NEXT_AT, DECIDE_AT, GONE_AT = 0.6, 1.8, 2.3, 3.7, 4.2

local function frame(props: { [any]: any }): Node
	props.shrink = 0
	props.h = 22
	props.px = K.raw(9)
	props.items = "center"
	props.gap = K.sp(6)
	props.rounded = "$radius.full"
	props.border = 1
	props.border_color = props.border_color or "$border.strong"
	props.bg = "$zeron.whats_new.block"
	return ui.row(props)
end

local function tab(mark: Node, label: string, on: () -> boolean): Node
	return ui.row {
		shrink = 0,
		h = 20,
		px = K.sp(6),
		items = "center",
		gap = K.raw(5),
		rounded = "$radius.md",
		bg = function(): string
			return if on() then "$zeron.whats_new.wash_08" else "#00000000"
		end,
		mark,
		K.text(10.5, function(): string
			return if on() then "$text.primary" else "$text.secondary"
		end, label),
	}
end

local function option(n: string, label: string, on: () -> boolean): Node
	return ui.row {
		h = 20,
		px = K.sp(6),
		items = "center",
		gap = K.raw(7),
		rounded = "$radius.md",
		bg = function(): string
			return if on() then "$zeron.whats_new.accent_wash" else "#00000000"
		end,
		K.key_chip(n),
		K.text(10.5, function(): string
			return if on() then "$text.primary" else "$text.secondary"
		end, label),
	}
end

local function never(): boolean
	return false
end

local function visual(clock: K.Clock, hero: boolean): Node
	local on_decision = ui.computed(function(): boolean
		return clock.t() >= NEXT_AT
	end)
	local function shown(): number
		local t = clock.t()
		return K.ramp(t, OPEN_AT, 220, K.EASE_OUT) * (1 - K.ramp(t, GONE_AT, 160, K.EASE_IN))
	end
	local function next_in(): number
		return K.ramp(clock.t(), NEXT_AT, 200, K.EASE_OUT)
	end
	local function pill_out(): number
		return K.ramp(clock.t(), GONE_AT + 0.1, 200, K.EASE_IN)
	end
	local function after(at: number): () -> boolean
		return function(): boolean
			return clock.t() >= at
		end
	end
	local function deciding(): boolean
		return on_decision:get()
	end
	local agents = frame {
		K.spinner(clock, 12),
		K.text(11, "$text.secondary", if hero then "Working" else "Audit pstack \u{b7} Working"),
	}
	local waiting = frame {
		opacity = function(): number
			return 1 - pill_out()
		end,
		border_color = function(): string
			return if shown() > 0 then "$text.secondary" else "$border.strong"
		end,
		K.question_mark(),
		K.text(11, "$accent", function(): string
			return if on_decision:get() then "1" else "2"
		end),
		K.text(11, "$text.secondary", "Waiting"),
	}
	local tabs = ui.row {
		grow = 1,
		items = "center",
		gap = K.raw(3),
		ui.show(function(): boolean
			return not on_decision:get()
		end, tab(K.question_mark(), if hero then "Pricing" else "Pricing research", function(): boolean
			return true
		end)),
		tab(K.glyph("lucide-shield", 11, "$text.secondary"), if hero then "Grouping" else "Sidebar grouping", deciding),
	}
	local body
	if hero then
		body = ui.switch(function(): string
			return if on_decision:get() then "decision" else "question"
		end, {
			question = ui.row { gap = K.sp(4), option("1", "Stripe", after(PICK_AT)), option("2", "Paddle", never) },
			decision = ui.row { gap = K.sp(4), option("1", "By project", after(DECIDE_AT)), option("2", "By date", never) },
		})
	else
		body = ui.switch(function(): string
			return if on_decision:get() then "decision" else "question"
		end, {
			question = ui.column {
				gap = K.sp(2),
				K.text(9.5, "$accent", "Question"),
				K.text(11.5, "$text.primary", "Which provider first?"),
				option("1", "Stripe", after(PICK_AT)),
				option("2", "Paddle", never),
			},
			decision = ui.column {
				gap = K.sp(2),
				K.text(9.5, "$accent", "Decision \u{b7} Supervisor"),
				K.text(11.5, "$text.primary", "Pick the sidebar grouping"),
				option("1", "By project", after(DECIDE_AT)),
				option("2", "By last update", never),
			},
		})
	end
	local card = ui.column {
		w = if hero then nil else 280,
		w_full = if hero then true else nil,
		shrink = 0,
		y = function(): number
			return (1 - shown()) * (if hero then -6 else 10)
		end,
		opacity = shown,
		rounded = K.raw(10),
		border = 1,
		border_color = "$border",
		bg = "$zeron.whats_new.raised",
		ui.row {
			h = if hero then 24 else 28,
			px = K.sp(6),
			items = "center",
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border",
			tabs,
			K.text(10.5, "$text.tertiary", "\u{d7}"),
		},
		ui.div {
			px = K.sp(if hero then 6 else 10),
			py = K.sp(if hero then 5 else 6),
			ui.div {
				x = function(): number
					return if on_decision:get() then (1 - next_in()) * 8 else 0
				end,
				body,
			},
		},
		if hero
			then nil
			else ui.row {
				h = 20,
				px = K.sp(10),
				items = "center",
				ui.div { grow = 1, K.text(9.5, "$text.tertiary", "Esc to answer later") },
				K.text(9.5, "$text.secondary", function(): string
					return if on_decision:get() then "Dismiss" else "Open thread"
				end),
			},
	}
	local pills = ui.row { gap = K.raw(5), agents, waiting }
	if hero then
		return ui.column { w_full = true, h = 84, shrink = 0, px = K.sp(10), gap = K.raw(5), pills, card }
	end
	local composer = ui.row {
		w = 300,
		h = 28,
		px = K.sp(10),
		items = "center",
		rounded = K.raw(10),
		border = 1,
		border_color = "$border",
		bg = "$zeron.whats_new.block",
		K.text(11, "$text.tertiary", "Do anything\u{2026}"),
	}
	return ui.column { h = 196, shrink = 0, justify = "end", gap = K.raw(5), card, pills, composer }
end

return {
	card = {
		period = 7.0,
		rest = 1.4,
		build = function(clock: K.Clock): Node
			return visual(clock, false)
		end,
	},
	hero = {
		period = 7.0,
		rest = 1.4,
		build = function(clock: K.Clock): Node
			return visual(clock, true)
		end,
	},
}
