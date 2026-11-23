-- An option a person picks inside a reply (a quiz's, an inline question's): its wash, its mark, its words.
--   Choice.Row { class, multi, label, chosen = fn, on_pick = fn, look = fn, settled = fn?, opacity = fn?, letter?, body = Node }

local Icon = require("@self/icon")

local M = {}

M.CLEAR = "#00000000"
local CLEAR = M.CLEAR

-- What a mark shows: its fill, its ring, and inside it "letter:<ink>", "check:<ink>", "cross:<ink>" or "".
export type Look = () -> (string, string, string)

export type RowProps = {
	class: string,
	mark_class: string?,
	multi: boolean,
	label: string,
	letter: string?,
	chosen: () -> boolean,
	on_pick: () -> (),
	look: Look,
	settled: (() -> boolean)?,
	opacity: (() -> number)?,
	body: Node,
}

local function mark(props: RowProps): Node
	return ui.row {
		class = props.mark_class,
		w = 20,
		h = 20,
		shrink = 0,
		mt = ui.raw(1, "a mark level with its words' first line, as the design's"),
		items = "center",
		justify = "center",
		rounded = if props.multi then ui.raw(6, "a multi choice's square mark, as the design's") else "$radius.full",
		bg = function(): string
			local fill = props.look()
			return fill
		end,
		border = function(): number
			local _, ring = props.look()
			return if ring == CLEAR then 0 elseif ring == "$success" then 1.5 else 1
		end,
		border_color = function(): string
			local _, ring = props.look()
			return ring
		end,
		ui.presence(function(): string
			local _, _, inside = props.look()
			return inside
		end, function(inside: string): Node
			local what, ink = string.match(inside, "^(%a+):(.+)$")
			if what == "letter" then
				return ui.text {
					props.letter or "",
					literal = true,
					text_size = ui.raw(11, "a mark's letter, as the design's"),
					weight = "$font.weight.semibold",
					color = ink,
				}
			elseif what == "check" or what == "cross" then
				return Icon { name = if what == "check" then "check" else "close", size = 12, color = ink }
			end
			return ui.div {}
		end),
	}
end

function M.Row(props: RowProps): Node
	local hovered = ui.use_state(false)
	local function settled(): boolean
		return props.settled ~= nil and props.settled()
	end
	return ui.row {
		class = props.class,
		role = if props.multi then "checkbox" else "radio",
		label = props.label,
		checked = props.chosen,
		focusable = true,
		w_full = true,
		min_w = 0,
		gap = ui.raw(10, "an option's mark and words, as the design's"),
		items = "start",
		px = "$space.2",
		py = ui.raw(7, "an option's room, as the design's"),
		rounded = ui.raw(10, "an option's wash, as the design's"),
		cursor = "pointer",
		bg = function(): string
			if settled() then
				return CLEAR
			end
			if props.chosen() then
				return "$zeron.secondary.bg"
			end
			return if hovered:get() then "$hover" else CLEAR
		end,
		opacity = props.opacity,
		on_hover = function(inside: boolean)
			hovered:set(inside)
		end,
		on_click = props.on_pick,
		on_key = function(e: any): boolean
			if e.key == "Enter" or e.key == " " then
				props.on_pick()
				return true
			end
			return false
		end,
		mark(props),
		props.body,
	}
end

return M
