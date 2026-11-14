local Csv = require("@self/csv")

local STEPS = 2000
local SAMPLE = 500
local MOST_CHARS = 40
local ROW = 28

local function shown(v: string?): string
	if not v then
		return ""
	end
	return if string.find(v, "[\r\n]") then (string.gsub(v, "\r?\n", " ↵ ")) else v
end

local function chars(v: string): number
	return utf8.len(v) or #v
end

export type Props = {
	text: () -> string,
	tsv: boolean?,
	label: Label?,
	waiting: (() -> Node)?,
	on_bad: (line: number) -> (),
}

return ui.component(function(props: Props): Node
	local sep = if props.tsv then "\t" else ","
	local rows: { { string } } = {}
	local widths: { number } = {}
	local ready = ui.use_state(false)
	local columns = ui.use_state(0)
	local count = ui.use_state(0)
	local version = ui.use_state(0)
	local room = ui.use_state(0)
	local size = (ui.theme.get("font.size.sm") :: number?) or 12
	local pad = (ui.theme.get("space.3") :: number?) or 12
	local per_char = size * 0.62
	local function width_of(n: number): number
		return math.ceil(math.clamp(n, 2, MOST_CHARS) * per_char) + pad * 2
	end
	local function number_width(): number
		version:get()
		return width_of(#tostring(math.max(#rows - 1, 1)))
	end
	local function total(): number
		version:get()
		local w = number_width()
		for _, cw in widths do
			w += cw
		end
		return w
	end
	local generation = 0
	ui.effect(function()
		local text = props.text()
		generation += 1
		local mine = generation
		local p = Csv.parser(text, sep)
		local function step()
			if mine ~= generation then
				return
			end
			if not p.step(STEPS) then
				ui.after(0, step)
				return
			end
			if p.bad then
				props.on_bad(p.bad)
				return
			end
			local most: { number } = {}
			for c = 1, p.columns do
				most[c] = 0
			end
			for r = 1, math.min(#p.rows, SAMPLE) do
				for c, v in p.rows[r] do
					most[c] = math.max(most[c], chars(shown(v)))
				end
			end
			rows = p.rows
			widths = {}
			for c = 1, p.columns do
				widths[c] = width_of(most[c])
			end
			ui.batch(function()
				columns:set(p.columns)
				count:set(math.max(#rows - 1, 0))
				version:set(version:peek() + 1)
				ready:set(true)
			end)
		end
		step()
	end)
	ui.on_cleanup(function()
		generation += 1
	end)

	local function cell(r: number, c: number, head: boolean): Node
		return ui.text {
			function(): string
				version:get()
				local row = rows[r]
				return shown(if row then row[c] else nil)
			end,
			literal = true,
			truncate = true,
			shrink = 0,
			w = function(): any
				version:get()
				return ui.raw(widths[c] or width_of(2), "the column's width, from its content")
			end,
			px = "$space.3",
			font = "$font.mono",
			text_size = "$font.size.sm",
			weight = if head then "$font.weight.medium" else nil,
			color = if head then "$text.secondary" else "$text.primary",
		}
	end
	local function number_cell(content: () -> string): Node
		return ui.text {
			content,
			literal = true,
			shrink = 0,
			w = function(): any
				return ui.raw(number_width(), "the row numbers' width, from the last")
			end,
			px = "$space.3",
			align = "end",
			tabular = true,
			text_size = "$font.size.sm",
			color = "$text.tertiary",
		}
	end
	local function line(r: number, n: number, head: boolean): Node
		local out: ContainerProps = {
			w_full = true,
			h = ROW,
			shrink = 0,
			bg = if head then "$surface.raised" else nil,
			items = "center",
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border.subtle",
			number_cell(function(): string
				return if head then "" else tostring(r - 1)
			end),
		}
		for c = 1, n do
			table.insert(out, cell(r, c, head))
		end
		return ui.row(out)
	end

	return ui.presence(function(): string
		return if ready:get() then `table:{columns:get()}` else "waiting"
	end, function(key: string): Node
		if key == "waiting" then
			return if props.waiting then props.waiting() else ui.column {}
		end
		local n = columns:peek()
		return ui.scroll_x {
			w_full = true,
			h_full = true,
			scrollbar = true,
			label = props.label,
			on_resize = function(size)
				room:set(size.w)
			end,
			ui.column {
				h_full = true,
				w = function(): any
					return ui.raw(math.max(total(), room:get()), "the columns' widths, from their content, or the view's")
				end,
				line(1, n, true),
				ui.list {
					w_full = true,
					grow = 1,
					min_h = 0,
					count = count,
					height = ROW,
					scrollbar = true,
					row = function(i: number): Node
						return line(i + 1, n, false)
					end,
					row_text = function(i: number): string?
						local row = rows[i + 1]
						return if row then table.concat(row, "\t") else nil
					end,
				},
			},
		}
	end, { w_full = true, h_full = true })
end)
