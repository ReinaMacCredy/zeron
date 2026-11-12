local K = require("@app/screens/shell/whats_new_visuals/kit")

local DONE_AT = 2.6

local function build(clock: K.Clock): Node
	local done = ui.computed(function(): boolean
		return clock.t() >= DONE_AT
	end)
	local function swap(): number
		return K.ramp(clock.t(), DONE_AT, 200, K.EASE_OUT)
	end
	local function fill(): number
		local t = clock.t()
		return K.ramp(t, DONE_AT, 320, K.EASE_OUT) * (1 - K.ramp(t, DONE_AT + 0.77, 400, K.EASE_OUT))
	end
	local function streak(): number
		return 1 - K.ramp(clock.t(), DONE_AT, 300, K.EASE_OUT)
	end
	local function sweep(): number
		return (clock.t() * 0.8) % 1
	end
	local function pct(f: number): string
		return `{f * 100}%`
	end
	local function rule(toward_left: boolean): Node
		local function at(): string
			return pct(sweep() * 0.75)
		end
		return ui.div {
			grow = 1,
			h = 1,
			relative = true,
			bg = "$border",
			ui.div {
				absolute = true,
				top = 0,
				bottom = 0,
				left = if toward_left then nil else 0,
				right = if toward_left then 0 else nil,
				w = function(): string
					return pct(fill())
				end,
				bg = "$zeron.whats_new.success_line",
			},
			ui.show(function(): boolean
				return streak() > 0 and not clock.reduced()
			end, ui.div {
				absolute = true,
				top = 0,
				bottom = 0,
				w = "25%",
				left = if toward_left then nil else at,
				right = if toward_left then at else nil,
				bg = "$accent",
				opacity = function(): number
					return streak() * (1 - sweep())
				end,
			}),
		}
	end
	local label = ui.row {
		shrink = 0,
		gap = K.sp(6),
		items = "center",
		ui.row {
			w = 14,
			h = 14,
			shrink = 0,
			items = "center",
			justify = "center",
			ui.switch(function(): string
				return if done:get() then "done" else "working"
			end, {
				done = ui.div { opacity = swap, K.glyph("lucide-check", 13, "$success") },
				working = K.spinner(clock, 12),
			}),
		},
		K.text(12, "$text.secondary", function(): string
			return if done:get() then "Compacted" else "Compacting"
		end),
		K.text(11, "$text.tertiary", "\u{b7} 120.0k"),
		ui.show(function(): boolean
			return done:get()
		end, ui.div { opacity = swap, K.text(11, "$text.tertiary", "\u{2192} 40.2k") }),
		ui.div {
			shrink = 0,
			overflow = "hidden",
			hidden = function(): boolean
				return swap() <= 0
			end,
			w = function(): number
				return 16 * swap()
			end,
			K.glyph("chevron-right", 12, "$text.tertiary"),
		},
	}
	return ui.row {
		w = 400,
		shrink = 0,
		gap = K.sp(10),
		items = "center",
		rule(true),
		label,
		rule(false),
	}
end

return { period = 5.2, rest = 4.2, build = build }
