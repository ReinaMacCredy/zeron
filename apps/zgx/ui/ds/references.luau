local tokens = require("@ds/mention_tokens")
local SVGS: { [string]: string } = require("@ds/icon_svgs")

local M = {}

local NB = "\u{a0}"
local LEAD = string.rep(NB, 6)
local TRAIL = string.rep(NB, 2)
local TASK_MAX = 32

export type Kind = "skill" | "command" | "task"
export type Ref = { kind: Kind, link: string, task: string? }

local shared: State<{ [string]: { [string]: Ref } }>? = nil
-- (Made on first use, not at load: a plugin's markdown requires this module for `humanize` alone, and a plugin may not hold app state.)
local function maps(): State<{ [string]: { [string]: Ref } }>
	if not shared then
		shared = ui.shared("ds.references", {} :: { [string]: { [string]: Ref } })
	end
	return shared :: State<{ [string]: { [string]: Ref } }>
end

function M.humanize(name: string): string
	local tail = string.match(name, "([^:]*)$") or name
	local words: { string } = {}
	for word in string.gmatch(tail, "[^%-_%s]+") do
		table.insert(words, string.upper(string.sub(word, 1, 1)) .. string.lower(string.sub(word, 2)))
	end
	return table.concat(words, " ")
end

local function cut(title: string): string
	local n = utf8.len(title) or #title
	if n <= TASK_MAX then
		return title
	end
	return string.sub(title, 1, (utf8.offset(title, TASK_MAX) or #title + 1) - 1) .. "…"
end

function M.label(kind: Kind, name: string): string
	local words = if kind == "skill" then M.humanize(name) elseif kind == "command" then "/" .. name else cut(name)
	words = (string.gsub(words, " ", NB))
	if kind == "task" then
		return LEAD .. words .. NB
	end
	return LEAD .. words .. TRAIL
end

local function map_of(key: string): { [string]: Ref }
	return maps():peek()[key] or {}
end

function M.add(key: string, ref: { kind: Kind, name: string, link: string, task: string? }): string
	local map = map_of(key)
	local shown = M.label(ref.kind, ref.name)
	while map[shown] and map[shown].link ~= ref.link do
		shown ..= NB
	end
	if not map[shown] then
		local next = table.clone(map)
		next[shown] = { kind = ref.kind, link = ref.link, task = ref.task }
		local all = table.clone(maps():peek())
		all[key] = next
		maps():set(all)
	end
	return shown
end

function M.clear(key: string)
	if maps():peek()[key] then
		local all = table.clone(maps():peek())
		all[key] = nil
		maps():set(all)
	end
end

local function labels(map: { [string]: Ref }): { string }
	local out: { string } = {}
	for shown in map do
		table.insert(out, shown)
	end
	table.sort(out, function(a: string, b: string): boolean
		return #a > #b
	end)
	return out
end

function M.encode(key: string, text: string): string
	local map = map_of(key)
	if next(map) == nil or not string.find(text, NB, 1, true) then
		return text
	end
	for _, shown in labels(map) do
		local out: { string } = {}
		local at = 1
		while true do
			local s, e = string.find(text, shown, at, true)
			if not s or not e then
				break
			end
			table.insert(out, string.sub(text, at, s - 1))
			table.insert(out, map[shown].link)
			at = e + 1
		end
		table.insert(out, string.sub(text, at))
		text = table.concat(out)
	end
	return text
end

function M.round_trips(key: string, raw: string): boolean
	return M.encode(key, M.decode(key, raw)) == raw
end

function M.decode(key: string, raw: string): string
	if not string.find(raw, "](zeron-", 1, true) then
		return raw
	end
	local out: { string } = {}
	local i = 1
	while true do
		local close, after, scheme, target = string.find(raw, "%]%(zeron%-(%a+):([^%)%s]*)%)", i)
		if not close then
			break
		end
		local open: number? = nil
		local j = close - 1
		while j >= i do
			local ch = string.sub(raw, j, j)
			if (ch == "[" or ch == "]") and string.sub(raw, j - 1, j - 1) ~= "\\" then
				open = if ch == "[" then j else nil
				break
			end
			j -= 1
		end
		local shown: string? = nil
		if open then
			local label = string.gsub(string.sub(raw, open + 1, close - 1), "\\(.)", "%1")
			local link = string.sub(raw, open, after :: number)
			local trigger = string.sub(label, 1, 1)
			if scheme == "invoke" and (trigger == "/" or trigger == "$") then
				shown = M.add(key, { kind = if trigger == "$" then "skill" else "command", name = string.sub(label, 2), link = link })
			elseif scheme == "task" then
				shown = M.add(key, { kind = "task", name = label, link = link, task = target })
			end
		end
		if shown and open then
			table.insert(out, string.sub(raw, i, open - 1))
			table.insert(out, shown)
		else
			table.insert(out, string.sub(raw, i, after :: number))
		end
		i = (after :: number) + 1
	end
	table.insert(out, string.sub(raw, i))
	return table.concat(out)
end

function M.spans(key: string, text: string): { { from: number, to: number, shown: string, ref: Ref } }
	local map = map_of(key)
	local out = {}
	if next(map) == nil or not string.find(text, NB, 1, true) then
		return out
	end
	local taken: { [number]: boolean } = {}
	for _, shown in labels(map) do
		local at = 1
		while true do
			local s, e = string.find(text, shown, at, true)
			if not s or not e then
				break
			end
			if not taken[s] then
				taken[s] = true
				table.insert(out, { from = s, to = e, shown = shown, ref = map[shown] })
			end
			at = e + 1
		end
	end
	table.sort(out, function(a, b): boolean
		return a.from < b.from
	end)
	return out
end

function M.highlight(key_of: () -> string): (line: string, state: number) -> ({ { any } }, number)
	return function(line: string, state: number): ({ { any } }, number)
		local spans, next_state = tokens.highlight(line, state, true)
		if state ~= 0 then
			return spans, next_state
		end
		for _, at in M.spans(key_of(), line) do
			local ink = if at.ref.kind == "skill" then "$zeron.reference.skill" else "$zeron.reference.command"
			local from = if at.ref.kind == "task" then at.from else at.from + #LEAD
			local to = if at.ref.kind == "task" then at.to else at.to - #TRAIL
			table.insert(spans, 1, { from, to, ink, weight = 500 } :: any)
		end
		return spans, next_state
	end
end

local TASK_MARKS = {
	todo = { "lucide-circle-dashed", "$text.tertiary" },
	working = { "lucide-circle-dashed", "$zeron.reference.working" },
	needs = { "lucide-message-circle-question", "$zeron.reference.needs" },
	done = { "lucide-circle-check", "$zeron.reference.done" },
}
local function src(name: string): string
	return `code:{SVGS[name] or ""}`
end

-- A skill's or command's icon drawn `dy` points lower in its `size` square:
-- its row centres it on the caps, and a label mostly in lowercase reads lower.
function M.lowered(name: string, size: number, dy: number): string
	local svg = SVGS[name] or ""
	return "code:" .. (string.gsub(svg, 'viewBox="0 0 24 24"', string.format('viewBox="0 %.2f 24 24"', -dy * 24 / size), 1))
end

function M.chips(key_of: () -> string, value: State<string>, line: number, column_of: ((id: string) -> string?)?): () -> { any }
	local size = math.min(14, line - 3 - 6)
	return function(): { any }
		local out = {}
		for _, at in M.spans(key_of(), value:get()) do
			if at.ref.kind == "task" then
				local mark = TASK_MARKS[(column_of and at.ref.task and column_of(at.ref.task)) or "todo"] or TASK_MARKS.todo
				table.insert(out, { from = at.from, to = at.to, icon = src(mark[1]), icon_color = mark[2], icon_size = 14, icon_x = 3.4 })
			else
				local skill = at.ref.kind == "skill"
				table.insert(out, {
					from = at.from,
					to = at.to,
					bg = if skill then "$zeron.reference.skill_fill" else "$zeron.reference.command_fill",
					radius = 5,
					inset = 1.5,
					icon = M.lowered(if skill then "magic-stick" else "command", size, 0.5),
					icon_color = if skill then "$zeron.reference.skill" else "$zeron.reference.command",
					icon_size = size,
					icon_x = 4,
				})
			end
		end
		return out
	end
end

local function grown(key: string, text: string, from: number, to: number): (number?, number?)
	local a, b = math.min(from, to), math.max(from, to)
	local moved = false
	for _, at in M.spans(key, text) do
		local s, e = at.from - 1, at.to
		if from == to then
			if a > s and a < e then
				a = if a - s > e - a then e else s
				b, moved = a, true
			end
		else
			if a > s and a < e then
				a, moved = s, true
			end
			if b > s and b < e then
				b, moved = e, true
			end
		end
	end
	if not moved then
		return nil, nil
	end
	return a, b
end

function M.unit(key_of: () -> string, value: State<string>, field: any)
	local snapping = false
	local function on_caret(_line: number, _column: number, anchor: number, focus: number)
		if snapping then
			return
		end
		local el = field:peek()
		if not el then
			return
		end
		local a, b = grown(key_of(), value:peek(), anchor, focus)
		if a and b then
			snapping = true
			if anchor <= focus then
				el:set_selection_bytes(a, b)
			else
				el:set_selection_bytes(b, a)
			end
			snapping = false
		end
	end
	local function keys(e: any): boolean
		local el = field:peek()
		if not el or e.cmd or e.ctrl then
			return false
		end
		local a, b = el:selection_bytes()
		if a ~= b then
			return false
		end
		local text = value:peek()
		for _, at in M.spans(key_of(), text) do
			local s, t = at.from - 1, at.to
			local back = a == t and (e.key == "Backspace" or e.key == "ArrowLeft")
			local ahead = a == s and (e.key == "Delete" or e.key == "ArrowRight")
			if back or ahead then
				if e.key == "Backspace" or e.key == "Delete" then
					el:set_selection_bytes(s, t)
					el:replace_selection("")
				elseif e.shift then
					el:set_selection_bytes(a, if back then s else t)
				else
					snapping = true
					el:set_selection_bytes(if back then s else t, if back then s else t)
					snapping = false
				end
				return true
			end
		end
		return false
	end
	return { on_caret = on_caret, keys = keys }
end

return M
