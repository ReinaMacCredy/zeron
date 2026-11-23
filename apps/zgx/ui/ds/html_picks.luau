-- Options picked on an agent's page: the agent's block (zeron_proto `with_picks`), the tray and a sent message's chips.

local locale = require("@self/locale")
local palettes = require("@self/palettes")

local M = {}

-- The agent's words, locked in DECISIONS whatever the app's language.
M.PICKED_IN = "Đã chọn trong"
M.NOT_PICKED = "Chưa chọn:"
M.OPEN_PAGES = "Open pages: close with html_render `close` each one this message answered or moved past."

export type Pick = { group: string?, code: string, label: string }
export type Group = { code: string, name: string }
export type Card = { card: string, picks: { Pick }, missing: { Group } }

local function one_line(s: string): string
	-- (Not %s: in the app's locale it takes bytes of UTF-8 letters, à's second among them, for spaces.)
	return string.match((string.gsub(s, "[ \t\r\n\f\v]+", " ")), "^ ?(.-) ?$") or s
end

function M.block(card: string, picks: { Pick }, missing: { Group }): string
	local lines = { `{M.PICKED_IN} "{one_line(card)}":` }
	for _, p in picks do
		if p.group and p.group ~= "" then
			table.insert(lines, `{p.group} = {p.code} ({one_line(p.label)})`)
		else
			table.insert(lines, `- {one_line(p.label)}`)
		end
	end
	if #missing > 0 then
		local names = {}
		for _, g in missing do
			table.insert(names, `{g.code} ({one_line(g.name)})`)
		end
		table.insert(lines, `{M.NOT_PICKED} {table.concat(names, ", ")}`)
	end
	return table.concat(lines, "\n")
end

function M.with_picks(words: string, cards: { Card }): string
	local out = string.match(words, "^(.-)[ \t\r\n\f\v]*$") or words
	for _, c in cards do
		if #c.picks > 0 then
			if out ~= "" then
				out ..= "\n\n"
			end
			out ..= M.block(c.card, c.picks, c.missing)
		end
	end
	return out
end

function M.with_open(text: string, open: { { title: string, left: { Group } } }): string
	local out = string.match(text, "^(.-)[ \t\r\n\f\v]*$") or text
	if #open == 0 then
		return out
	end
	local lines = { M.OPEN_PAGES }
	for _, p in open do
		local names = {}
		for _, g in p.left do
			table.insert(names, `{g.code} ({one_line(g.name)})`)
		end
		table.insert(lines, `"{one_line(p.title)}": {table.concat(names, ", ")}`)
	end
	return (if out ~= "" then out .. "\n\n" else "") .. table.concat(lines, "\n")
end

local function sent_card(par: string): (string?, { Pick })
	local lines = string.split(par, "\n")
	local card = string.match(lines[1], `^{M.PICKED_IN} "(.+)":$`)
	local picks: { Pick } = {}
	if not card then
		return nil, picks
	end
	for i = 2, #lines do
		local line = lines[i]
		local free = string.match(line, "^%- (.*%S.*)$")
		local group, code, label = string.match(line, "^(%S+) = (%S+) %((.*)%)$")
		if free then
			table.insert(picks, { code = free, label = free })
		elseif group then
			table.insert(picks, { group = group, code = code, label = label })
		elseif not (i == #lines and #picks > 0 and string.sub(line, 1, #M.NOT_PICKED) == M.NOT_PICKED) then
			return nil, picks
		end
	end
	return if #picks > 0 then card else nil, picks
end

-- A prompt as `with_picks` and `with_open` left it (zeron_proto `extract_picks`): its words, each page's picks, the blocks after them.
function M.split(text: string): (string, { { card: string, picks: { Pick } } }, string)
	local pars = string.split(text, "\n\n")
	local cards = {}
	local from, to = 0, 0
	for i, par in pars do
		local card, picks = sent_card(par)
		if card then
			from = if from == 0 then i else from
			to = i
			table.insert(cards, { card = card, picks = picks })
		elseif from > 0 then
			break
		end
	end
	if from == 0 then
		return text, cards, ""
	end
	local after = to + 1
	if pars[after] and string.split(pars[after], "\n")[1] == M.OPEN_PAGES then
		after += 1
	end
	return table.concat(pars, "\n\n", 1, from - 1), cards, table.concat(pars, "\n\n", after)
end

-- A sent text as a queue row reads it: without its pick blocks and its Open pages block.
function M.shown(text: string): string
	local out: { string } = {}
	for _, par in string.split(text, "\n\n") do
		if not sent_card(par) and string.split(par, "\n")[1] ~= M.OPEN_PAGES then
			table.insert(out, par)
		end
	end
	return table.concat(out, "\n\n")
end

function M.cards(items: { any }): { Card }
	local order: { string } = {}
	local by: { [string]: { any } } = {}
	for _, it in items do
		if it.kind == "pick" then
			if not by[it.card] then
				by[it.card] = {}
				table.insert(order, it.card)
			end
			table.insert(by[it.card], it)
		end
	end
	local out: { Card } = {}
	for _, card in order do
		local list = by[card]
		local first = list[1]
		local groups: { Group } = first.groups or {}
		local picked: { [string]: any } = {}
		local free: { Pick } = {}
		for _, it in list do
			if it.group and it.group ~= "" then
				picked[it.group] = it
			else
				table.insert(free, { code = it.code, label = it.label })
			end
		end
		local locked: { [string]: boolean } = {}
		for _, code in first.locked or {} do
			locked[code] = true
		end
		local picks: { Pick } = {}
		local missing: { Group } = {}
		for _, g in groups do
			local it = picked[g.code]
			if it then
				table.insert(picks, { group = g.code, code = it.code, label = it.label })
				picked[g.code] = nil
			elseif not locked[g.code] then
				table.insert(missing, g)
			end
		end
		for _, it in list do
			if it.group and it.group ~= "" and picked[it.group] then
				table.insert(picks, { group = it.group, code = it.code, label = it.label })
				picked[it.group] = nil
			end
		end
		for _, p in free do
			table.insert(picks, p)
		end
		table.insert(out, { card = first.card_title or card, picks = picks, missing = missing })
	end
	return out
end

function M.capped(hex: string, s: number, l: number): string
	local h, sat, light = palettes.hsl(hex)
	return palettes.from_hsl(h, math.min(sat, s), math.min(light, l))
end

-- In light themes capped at 48% lightness and 57% saturation, so the tick and send read on white.
function M.ink(): string
	ui.theme.current:get()
	local accent = ui.theme.get("accent")
	if type(accent) ~= "string" or #accent < 7 then
		return "#c0347a"
	end
	local kind = ui.theme.kind()
	if kind == "dark" or kind == "high-contrast" then
		return string.sub(accent, 1, 7)
	end
	-- (DECISIONS' colour for the mock's pink exactly; the formula rounds it to #c0357a.)
	if string.lower(string.sub(accent, 1, 7)) == "#ec83b7" then
		return "#c0347a"
	end
	return M.capped(accent, 0.57, 0.48)
end

local function raw(n: number, why: string): any
	return ui.raw(n, why)
end

function M.Code(props: { code: string, id: string? }): Node
	return ui.row {
		id = props.id,
		min_w = raw(28, "the mock's code chip"),
		max_w = raw(140, "a free pick's label as its code"),
		h = raw(18, "the mock's code chip"),
		px = raw(6, "the mock's code chip"),
		items = "center",
		justify = "center",
		shrink = 0,
		rounded = raw(5, "the mock's code chip"),
		bg = "$surface.raised",
		border = 0.5,
		border_color = "$border",
		ui.text { props.code, literal = true, min_w = 0, truncate = true, text_size = "$font.size.xs", weight = "$font.weight.semibold", color = "$text.primary" },
	}
end

function M.Sent(props: { chips: { any } }): Node
	local cells = {}
	for _, c in props.chips do
		table.insert(cells, ui.row {
			items = "center",
			gap = raw(5, "the mock's sent chip"),
			M.Code { code = if (c.group or "") ~= "" then c.code else "·" },
			ui.text { c.label or c.name or "", literal = true, text_size = "$font.size.sm", color = "$text.primary" },
		})
	end
	return ui.row {
		class = "message-picks",
		label = locale.t("ds.html_picks.sent"),
		selectable = false,
		max_w = "80%",
		flex_wrap = true,
		justify = "end",
		gap = raw(6, "the mock's sent chips"),
		cells,
	}
end

return M
