local M = {}

M.H, M.TILE, M.MOST, M.GLYPH = 28, 20, 3, 16
-- (The gap is cut out of the circle behind, not painted: what's under the button shows through.)
local GAP_CUT = 2.5
local STEP = 15
-- (Nudged right on those behind, to sit in the middle of what shows of them.)
local MARK_SIDE, MARK_NUDGE = 11, 2
local INSET_L, INSET_R = 4, 6
M.GAP = 5

function M.stack_w(n: number): number
	return M.TILE + STEP * (math.max(n, 1) - 1)
end

function M.width(n: number, after: number): number
	return INSET_L + M.stack_w(n) + M.GAP + after + INSET_R
end

local CRESCENT = (function(): string
	local tile = M.TILE
	local r, far = tile / 2, tile / 2 + GAP_CUT
	local a = (STEP * STEP + r * r - far * far) / (2 * STEP)
	local h = math.sqrt(r * r - a * a)
	local x, top, bottom = r - a, r - h, r + h
	return `code:<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {tile} {tile}"><path fill="#ffffff" d="M{x} {top}A{r} {r} 0 1 1 {x} {bottom}A{far} {far} 0 0 0 {x} {top}Z"/></svg>`
end)()

export type StackProps = {
	id: string,
	marks: () -> { any },
	key: (any) -> string,
	mark: (any, number) -> Node,
	enter: any?,
}

local function tile(props: StackProps, at: number): Node
	local behind = at > 1
	return ui.show(function(): boolean
		return #props.marks() >= at
	end, ui.div {
		id = `{props.id}-{at}`,
		absolute = true,
		top = 0,
		left = (at - 1) * STEP,
		w = M.TILE,
		h = M.TILE,
		if behind
			then ui.svg { src = CRESCENT, tint = "$zeron.mark.bg", absolute = true, left = 0, top = 0, w = M.TILE, h = M.TILE }
			else ui.div { absolute = true, left = 0, top = 0, w = M.TILE, h = M.TILE, rounded = "$radius.full", bg = "$zeron.mark.bg" },
		ui.row {
			absolute = true,
			left = if behind then MARK_NUDGE else 0,
			top = 0,
			w = M.TILE,
			h = M.TILE,
			items = "center",
			justify = "center",
			color = ui.raw("#000000", "an ink-only mark on its white tile, whatever the theme"),
			ui.presence(function(): string
				local r = props.marks()[at]
				return if r then props.key(r) else ""
			end, function(key: string): Node
				for _, r in props.marks() do
					if props.key(r) == key then
						return props.mark(r, MARK_SIDE)
					end
				end
				return ui.div {}
			end, if at == 1 and props.enter then { enter = props.enter } else nil),
		},
	})
end

function M.Stack(props: StackProps): Node
	local stack: { any } = {
		relative = true,
		shrink = 0,
		h = M.TILE,
		w = function(): number
			return M.stack_w(#props.marks())
		end,
	}
	-- (Those behind first: each paints under the one after it.)
	for at = M.MOST, 1, -1 do
		table.insert(stack, tile(props, at))
	end
	return ui.div(stack)
end

export type TriggerProps = {
	id: string,
	label: () -> string,
	tooltip: string?,
	open: State<boolean>,
	hovered: State<boolean>,
	on_click: () -> (),
	stack: Node,
	[number]: Node,
}

function M.Trigger(props: TriggerProps): Node
	local row: { any } = {
		id = props.id,
		role = "button",
		label = props.label,
		tooltip = props.tooltip,
		tooltip_placement = if props.tooltip then { side = "bottom" } else nil,
		expanded = props.open,
		h = M.H,
		pl = ui.raw(INSET_L, "as far in as they are from its top"),
		pr = ui.raw(INSET_R, "clear of its end"),
		gap = ui.raw(M.GAP, "its marks and what follows them"),
		items = "center",
		rounded = "$radius.lg",
		cursor = "pointer",
		pointer_events = "auto",
		bg = function(): string
			if props.open:get() then
				return "$zeron.selected.flat.bg"
			end
			return if props.hovered:get() then "$hover" else "#00000000"
		end,
		color = function(): string
			return if props.open:get() or props.hovered:get() then "$text.primary" else "$text.secondary"
		end,
		on_hover = function(inside: boolean)
			props.hovered:set(inside)
		end,
		on_click = props.on_click,
		props.stack,
	}
	for _, child in ipairs(props) do
		table.insert(row, child)
	end
	return ui.row(row)
end

return M
