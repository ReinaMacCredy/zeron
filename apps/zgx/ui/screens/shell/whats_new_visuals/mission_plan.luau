local K = require("@app/screens/shell/whats_new_visuals/kit")

local PRESS_AT, APPROVED_AT, FIRST_DONE_AT = 1.6, 1.8, 4.4

local STEPS: { { title: string, start: number, done: number? } } = {
	{ title = "Fix the cart total test", start = 2.3, done = FIRST_DONE_AT },
	{ title = "Add rate limiting to checkout", start = 2.6 },
	{ title = "Export orders as CSV", start = 2.9 },
}

local function step(clock: K.Clock, n: number): Node
	local spec = STEPS[n]
	local function state(): string
		local t = clock.t()
		if spec.done and t >= spec.done then
			return "done"
		end
		return if t >= spec.start then "working" else "planned"
	end
	return ui.row {
		h = 19,
		items = "center",
		gap = K.raw(7),
		ui.row {
			shrink = 0,
			w = 12,
			justify = "center",
			ui.switch(state, {
				planned = K.mono(10, "$text.tertiary", tostring(n)),
				working = K.spinner(clock, 12),
				done = K.glyph("lucide-check", 12, "$zeron.whats_new.done"),
			}),
		},
		K.clip(11, "$text.primary", spec.title),
	}
end

local function build(clock: K.Clock): Node
	local function approved(): boolean
		return clock.t() >= APPROVED_AT
	end
	local function verdict(): string
		return if approved() then "approved" else "pending"
	end
	local function press(): number
		local t = clock.t()
		return K.ramp(t, PRESS_AT, 100, K.EASE_OUT) * (1 - K.ramp(t, PRESS_AT + 0.1, 120, K.EASE_IN))
	end
	return K.card {
		w = 320,
		h = 116,
		shrink = 0,
		px = K.sp(12),
		py = K.sp(8),
		ui.row {
			h = 22,
			items = "center",
			gap = K.sp(6),
			K.text(11, "$text.secondary", function(): string
				return if clock.t() >= FIRST_DONE_AT then "Plan 1/3" else "Plan · 3 steps"
			end),
			ui.div { grow = 1 },
			ui.switch(verdict, {
				pending = ui.row {
					shrink = 0,
					h = 18,
					px = K.sp(8),
					items = "center",
					rounded = K.r(6),
					bg = "$accent",
					scale = function(): number
						return 1 - 0.06 * press()
					end,
					K.text(10, "$accent.text", "Approve and run"),
				},
				approved = ui.row {
					shrink = 0,
					h = 18,
					items = "center",
					gap = K.sp(4),
					K.glyph("lucide-check", 11, "$zeron.whats_new.done"),
					K.text(10, "$zeron.whats_new.done", "Approved"),
				},
			}),
		},
		ui.div { h = 1, mt = K.sp(4), mb = K.sp(4), bg = "$border" },
		step(clock, 1),
		step(clock, 2),
		step(clock, 3),
	}
end

return { period = 7.0, rest = 3.4, build = build }
