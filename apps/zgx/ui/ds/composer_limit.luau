-- A usage limit reached, over the draft (Zeron's composer/limit_notice.rs): a
-- line saying so and when it resets, and what can be done: switch, wait for
-- the reset; while waiting, cancel. The caller says the words and the acts.
--
--   Notice { limit = function() return { waiting, lead, rest, actions = { { label, primary, on_click } } } end }

local Icon = require("@self/icon")
local Button = require("@self/button")
local FadeText = require("@self/fade_text")

local M = {}

export type Action = { label: string, primary: boolean?, on_click: () -> () }
export type Limit = { waiting: boolean, lead: string?, rest: string, actions: { Action } }

function M.Notice(props: { limit: () -> Limit? }): Node
	local function now(): Limit
		return props.limit() or { waiting = false, rest = "", actions = {} }
	end
	-- (By label: closures don't pass through a list's values.)
	local labels = ui.computed(function(): { string }
		local out = {}
		for _, a in now().actions do
			table.insert(out, a.label)
		end
		return out
	end)
	local function action(label: string): Action?
		for _, a in now().actions do
			if a.label == label then
				return a
			end
		end
		return nil
	end
	return ui.div {
		id = "composer-limit",
		w_full = true,
		pt = ui.raw(8, "gpui's limit notice"),
		px = ui.raw(8, "gpui's limit notice"),
		ui.row {
			role = "status",
			w_full = true,
			h = 38,
			pl = ui.raw(10, "gpui's limit notice"),
			pr = ui.raw(6, "gpui's limit notice"),
			gap = ui.raw(8, "gpui's limit notice"),
			items = "center",
			rounded = ui.raw(10, "gpui's limit notice"),
			bg = function(): string
				return if now().waiting then "$hover" else "$zeron.tone.danger.wash"
			end,
			ui.div {
				shrink = 0,
				color = function(): string
					return if now().waiting then "$text.tertiary" else "$zeron.tone.danger.ink"
				end,
				Icon {
					name = function(): Icon.Name
						return if now().waiting then "clock-circle" else "danger-triangle"
					end,
					size = 14,
				},
			},
			ui.show(function(): boolean
				return now().lead ~= nil
			end, ui.text {
				function(): string
					return now().lead or ""
				end,
				shrink = 0,
				text_size = ui.raw(12.5, "gpui's limit notice"),
				weight = "$font.weight.semibold",
				color = "$zeron.tone.danger.ink",
			}),
			FadeText {
				function(): string
					return now().rest
				end,
				grow = 1,
				text_size = ui.raw(12.5, "gpui's limit notice"),
				color = "$text.primary",
			},
			ui.each(labels, function(label: string): Node
				local a = action(label)
				return Button {
					label,
					size = "sm",
					shape = "pill",
					tone = if a and a.primary then "primary" else "ghost",
					h = 26,
					px = 12,
					text_size = ui.raw(12.5, "gpui's limit notice"),
					on_click = function()
						local now_a = action(label)
						if now_a then
							now_a.on_click()
						end
					end,
				}
			end, {
				key = function(label: string): string
					return label
				end,
				direction = "row",
				gap = ui.raw(4, "gpui's limit notice"),
				items = "center",
				justify = "end",
			}),
		},
	}
end

return M
