local K = require("@app/screens/shell/whats_new_visuals/kit")

type Step = { at: number, state: string }
type Thread = { title: string, steps: { Step } }

local THREADS: { Thread } = {
	{
		title = "Fix the cart total test",
		steps = { { at = 0, state = "planned" }, { at = 0.6, state = "working" }, { at = 3.2, state = "review" } },
	},
	{
		title = "Add rate limiting to checkout",
		steps = { { at = 0, state = "planned" }, { at = 0.9, state = "working" }, { at = 2.2, state = "waiting" } },
	},
	{
		title = "Export orders as CSV",
		steps = { { at = 0, state = "planned" }, { at = 1.2, state = "working" }, { at = 4.4, state = "done" } },
	},
}

local WORDS: { [string]: string } = {
	planned = "Planned",
	working = "Working",
	review = "Ready for review",
	waiting = "Waiting on you",
	done = "Done",
}

local INK: { [string]: string } = {
	planned = "$text.tertiary",
	working = "$text.secondary",
	review = "$accent",
	waiting = "$warning",
	done = "$text.tertiary",
}

local function state_at(thread: Thread, t: number): string
	local state = thread.steps[1].state
	for _, step in thread.steps do
		if t >= step.at then
			state = step.state
		end
	end
	return state
end

local function row(clock: K.Clock, thread: Thread): Node
	local state = ui.computed(function(): string
		return state_at(thread, clock.t())
	end)
	local function now(): string
		return state:get()
	end
	return ui.row {
		h = 22,
		items = "center",
		gap = K.raw(7),
		ui.switch(now, {
			planned = K.glyph("lucide-circle-dashed", 12, "$text.tertiary"),
			working = K.spinner(clock, 12),
			review = K.glyph("status-done", 12, "$accent"),
			waiting = K.glyph("clock-circle", 12, "$warning"),
			done = K.glyph("lucide-check", 12, "$zeron.whats_new.done"),
		}),
		K.clip(11.5, "$text.primary", thread.title),
		K.text(10, function(): string
			return INK[now()]
		end, function(): string
			return WORDS[now()]
		end),
	}
end

local function build(clock: K.Clock): Node
	local function working(): number
		local n = 0
		for _, thread in THREADS do
			if state_at(thread, clock.t()) == "working" then
				n += 1
			end
		end
		return n
	end
	return K.card {
		w = 320,
		h = 116,
		shrink = 0,
		px = K.sp(12),
		py = K.sp(8),
		gap = K.sp(2),
		ui.row {
			h = 20,
			items = "center",
			gap = K.sp(6),
			K.glyph("chat-round-line", 12, "$text.secondary"),
			K.text(11, "$text.secondary", "Threads"),
			ui.div { grow = 1 },
			ui.row {
				shrink = 0,
				h = 16,
				px = K.sp(6),
				items = "center",
				rounded = "$radius.full",
				bg = "$zeron.whats_new.wash_08",
				opacity = function(): number
					return if working() > 0 then 1 else 0
				end,
				K.text(9.5, "$text.secondary", function(): string
					return `{working()} Working`
				end),
			},
		},
		row(clock, THREADS[1]),
		row(clock, THREADS[2]),
		row(clock, THREADS[3]),
	}
end

return { period = 7.0, rest = 3.6, build = build }
