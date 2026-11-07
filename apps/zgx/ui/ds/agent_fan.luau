local AgentMark = require("@self/agent_mark")
local Icon = require("@self/icon")
local motion = require("@self/motion")

local M = {}

M.MAX = 3
M.WIDE = { { x = 0, turn = -3 }, { x = 14, turn = 3 }, { x = 28, turn = 7 } }

export type Tile = { key: string, mark: string?, icon: string?, asking: boolean?, lead: boolean? }
export type Props = {
	tiles: () -> { Tile },
	lead: ((size: number) -> Node)?,
	size: number?,
	label: (() -> string)?,
	on_click: (() -> ())?,
	id: string?,
	class: string?,
	ref: any?,
}

local SLIDE = { duration = 150, ease = "out(3)" }

function M.shown(tiles: { Tile }): { Tile }
	local out: { Tile } = {}
	for i, t in tiles do
		if i > M.MAX then
			break
		end
		out[i] = t
	end
	return out
end

function M.width(n: number, size: number): number
	if n <= 0 then
		return 0
	end
	return size + M.WIDE[math.min(n, M.MAX)].x * size / 20
end

local function look_tile(t: Tile, size: number, lead: ((size: number) -> Node)?): Node
	if t.lead and lead then
		return lead(size)
	end
	if t.mark or t.icon then
		return AgentMark { label = t.key, mark = t.mark, icon = t.icon, seed = t.key, size = size }
	end
	return ui.column {
		w = size,
		h = size,
		rounded = ui.raw(size * 0.25, "a mark tile's radius scales with it"),
		items = "center",
		justify = "center",
		bg = "$zeron.mark.bg",
		border = 1,
		border_color = "$zeron.mark.border",
		color = "#26251E",
		Icon { name = "bot", size = size * 0.6 },
	}
end

function M.Fan(props: Props): Node
	local size = props.size or 20
	local k = size / 20
	local list = ui.computed(function(): { Tile }
		return M.shown(props.tiles())
	end)
	-- Those behind first: each paints under the one after it.
	local behind_first = ui.computed(function(): { Tile }
		local now = list:get()
		local out: { Tile } = {}
		for i = #now, 1, -1 do
			table.insert(out, now[i])
		end
		return out
	end)
	local function place(key: string): number
		for i, t in list:get() do
			if t.key == key then
				return i
			end
		end
		return 1
	end
	local reduced = motion.reduced_now()
	local enter = if reduced
		then { opacity = { 0, 1 }, duration = motion.FADE }
		else { opacity = { 0, 1 }, scale = { 0.6, 1 }, duration = 220, ease = "outBack(1.6)" }
	local exit = if reduced
		then { opacity = 0, duration = motion.FADE }
		else { opacity = 0, scale = 0.6, duration = 120, ease = "inQuad" }
	local box: { [any]: any } = {
		id = props.id,
		class = props.class,
		ref = props.ref,
		relative = true,
		shrink = 0,
		h = size,
		w = motion.tween(ui.computed(function(): number
			return M.width(#list:get(), size)
		end), SLIDE),
		ui.each(behind_first, function(t: Tile): Node
			local at = ui.computed(function(): number
				return place(t.key)
			end)
			return ui.div {
				absolute = true,
				top = 0,
				w = size,
				h = size,
				left = motion.tween(ui.computed(function(): number
					return M.WIDE[at:get()].x * k
				end), SLIDE),
				rotate = motion.tween(ui.computed(function(): number
					return if #list:get() > 1 then M.WIDE[at:get()].turn else 0
				end), SLIDE),
				shadow = function(): any
					return if #list:get() > 1 and at:get() == 1 then "$shadow.sm" else false
				end,
				rounded = ui.raw(size * 0.25, "a mark tile's radius scales with it"),
				look_tile(t, size, props.lead),
				ui.show(function(): boolean
					for _, other in list:get() do
						if other.key == t.key then
							return other.asking == true
						end
					end
					return false
				end, ui.div {
					absolute = true,
					right = -2,
					top = -2,
					w = 7,
					h = 7,
					rounded = "$radius.full",
					bg = "$warning",
					border = 1,
					border_color = "$background",
				}),
			}
		end, {
			key = function(t: Tile): string
				return t.key
			end,
			enter = enter,
			exit = exit,
			absolute = true,
			left = 0,
			top = 0,
		}),
	}
	if props.on_click then
		box.role = "button"
		box.cursor = "pointer"
		box.label = props.label
		box.on_click = props.on_click
	else
		box.aria_hidden = true
		box.pointer_events = "none"
	end
	return ui.div(box)
end

return setmetatable(M, {
	__call = function(_self: any, props: Props): Node
		return M.Fan(props)
	end,
}) :: any
