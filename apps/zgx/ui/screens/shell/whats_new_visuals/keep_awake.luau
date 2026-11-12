local K = require("@app/screens/shell/whats_new_visuals/kit")

local DONE_AT = 3.2

local function switch_on(): Node
	return ui.div {
		shrink = 0,
		w = 40,
		h = 24,
		relative = true,
		rounded = "$radius.full",
		bg = "$zeron.switch.on",
		ui.div {
			absolute = true,
			left = 19,
			top = 3,
			w = 18,
			h = 18,
			rounded = "$radius.full",
			bg = "$zeron.switch.knob",
			shadow = "$zeron.switch.knob_lift",
		},
	}
end

local function build(clock: K.Clock): Node
	local done = ui.computed(function(): boolean
		return clock.t() >= DONE_AT
	end)
	local function swap(): number
		return K.ramp(clock.t(), DONE_AT, 220, K.EASE_OUT)
	end
	return K.card {
		w = 300,
		shrink = 0,
		px = K.sp(12),
		py = K.sp(8),
		gap = K.sp(4),
		ui.row {
			h = 28,
			gap = K.sp(8),
			items = "center",
			K.clip(11.5, "$text.primary", "Keep this Mac awake while agents work"),
			switch_on(),
		},
		ui.div { h = 1, bg = "$border" },
		ui.row {
			h = 22,
			gap = K.raw(7),
			items = "center",
			ui.switch(function(): string
				return if done:get() then "done" else "working"
			end, {
				done = K.glyph("lucide-check", 12, "$zeron.whats_new.done"),
				working = K.spinner(clock, 12),
			}),
			K.clip(11.5, "$text.primary", "Ship the shop"),
			K.mono(10.5, "$text.tertiary", function(): string
				return if done:get() then "Done" else `Working {42 + math.floor(clock.t())}s`
			end),
		},
		ui.row {
			h = 22,
			gap = K.raw(7),
			items = "center",
			opacity = function(): number
				return if done:get() then swap() else 1
			end,
			ui.switch(function(): string
				return if done:get() then "done" else "working"
			end, {
				done = K.glyph("moon", 12, "$text.secondary"),
				working = K.glyph("laptop", 12, "$zeron.whats_new.working"),
			}),
			K.text(10.5, function(): string
				return if done:get() then "$text.secondary" else "$zeron.whats_new.working"
			end, function(): string
				return if done:get() then "The Mac can sleep again a minute later" else "Awake while 1 agent works"
			end),
		},
	}
end

return { period = 6.0, rest = 1.8, build = build }
