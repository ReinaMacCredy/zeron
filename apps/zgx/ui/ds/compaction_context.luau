-- Claude Code's `/context` answer as Zeron's card draws it (crates/ui/src/
-- transcript/claude_cards.rs `render_context`): the context's parts as one
-- bar (drawn larger so the used part reads; free space isn't drawn), each
-- part in the legend with its tokens and share, the autocompact buffer and
-- what is free, the tip on what takes most, and what to do about it:
-- Compact now, Context settings. The host parses the answer
-- (`context_report` in src/chat.rs).
--
--   ContextCard { report = fn, on_compact = fn?, on_settings = fn? }

local Icon = require("@self/icon")
local Button = require("@self/button")
local locale = require("@self/locale")

local INK: { [string]: string } = {
	system = "$accent",
	mcp = "$zeron.syntax.function",
	agents = "$zeron.syntax.keyword",
	memory = "$warning",
	skills = "$success",
	messages = "$danger",
	other = "$text.secondary",
	buffer = "$border.strong",
}

local function s(key: string, args: { [string]: any }?): string
	return locale.s(`ds.compaction.context.{key}`, args)
end

local function legend_item(p: any): Node
	local ink = INK[p.ink]
	return ui.row {
		w = "50%",
		gap = ui.raw(8, "gpui's /context card"),
		px = ui.raw(6, "gpui's /context card"),
		py = ui.raw(3, "gpui's /context card"),
		items = "center",
		text_size = "$font.size.sm",
		ui.div {
			w = 8,
			h = 8,
			shrink = 0,
			rounded = ui.raw(2, "gpui's /context card"),
			bg = ink or "#00000000",
			border = if ink then nil else 1,
			border_color = if ink then nil else "$border.subtle",
		},
		ui.text { if p.ink == "buffer" then s("buffer") elseif p.ink == "free" then s("free") else p.name, literal = true, truncate = true, grow = 1, min_w = 0, color = "$text.primary" },
		ui.text { p.tokens, tabular = true, color = "$text.secondary" },
		ui.text { p.percent, tabular = true, w = 44, align = "right", color = "$text.secondary" },
	}
end

return function(props: { report: () -> any, on_compact: (() -> ())?, on_settings: (() -> ())? }): Node
	local r = props.report()
	local bar: { [any]: any } = {
		w_full = true,
		h = 10,
		rounded = ui.raw(5, "gpui's /context card"),
		overflow = "hidden",
		gap = ui.raw(1, "gpui's /context card"),
		bg = "$hover",
		aria_hidden = true,
	}
	for _, p in r.parts do
		if p.w > 0 then
			table.insert(bar, ui.div { h = 10, w = `{math.clamp(p.w, 0, 1) * 100}%`, shrink = 0, bg = INK[p.ink] or "$text.secondary" })
		end
	end
	local legend: { [any]: any } = { w_full = true, mt = ui.raw(10, "gpui's /context card"), flex_wrap = true }
	for _, p in r.parts do
		table.insert(legend, legend_item(p))
	end
	table.insert(legend, legend_item(r.free))
	return ui.column {
		class = "claude-context-card",
		role = "group",
		label = s("title"),
		w_full = true,
		rounded = ui.raw(12, "gpui's /context card"),
		border = 1,
		border_color = "$border.subtle",
		bg = "$surface.raised",
		overflow = "hidden",
		ui.row {
			w_full = true,
			px = ui.raw(12, "gpui's /context card"),
			py = ui.raw(8, "gpui's /context card"),
			gap = ui.raw(8, "gpui's /context card"),
			items = "center",
			ui.div { color = "$text.secondary", Icon { name = "speedometer", size = 14 } },
			ui.text { s("title"), weight = "$font.weight.medium", text_size = "$font.size.sm", color = "$text.primary" },
			ui.text { s("subtitle", { model = r.model }), literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.xs", color = "$text.tertiary" },
			ui.text { r.pill, tabular = true, shrink = 0, text_size = "$font.size.xs", color = "$text.secondary" },
		},
		ui.column {
			w_full = true,
			p = ui.raw(12, "gpui's /context card"),
			pt = 0,
			ui.row(bar),
			ui.text { s("caption", { n = string.format("%.0f", r.scale) }), mt = ui.raw(4, "gpui's /context card"), text_size = "$font.size.xs", color = "$text.tertiary" },
			ui.row(legend),
			if r.tip
				then ui.text { r.tip, literal = true, wrap = true, mt = ui.raw(10, "gpui's /context card"), px = ui.raw(10, "gpui's /context card"), py = ui.raw(8, "gpui's /context card"), rounded = ui.raw(8, "gpui's /context card"), bg = "$hover", text_size = "$font.size.sm", color = "$text.secondary" }
				else nil,
			if r.note then ui.text { r.note, literal = true, wrap = true, mt = ui.raw(8, "gpui's /context card"), text_size = "$font.size.xs", color = "$warning" } else nil,
		},
		ui.row {
			w_full = true,
			px = ui.raw(12, "gpui's /context card"),
			py = ui.raw(8, "gpui's /context card"),
			gap = ui.raw(6, "gpui's /context card"),
			border_sides = { 1, 0, 0, 0 },
			border_color = "$border.subtle",
			Button { s("compact_now"), class = "claude-context-compact", tone = "primary", size = "sm", on_click = props.on_compact },
			Button { s("settings"), class = "claude-context-settings", tone = "ghost", size = "sm", on_click = props.on_settings },
		},
	}
end
