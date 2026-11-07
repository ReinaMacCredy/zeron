-- Options picked on an agent's page (`data-zeron-pick` buttons): what the
-- composer sends for them, and how the tray over its input and a sent
-- message show them. A click on a page only places its pick in the tray:
-- sending stays with the composer.
--
--   picks.with_picks(words, cards)   the prompt: its words, then each card's
--                                    block, as zeron_proto html_render
--                                    `with_picks` writes it
--   picks.ink()                      the pick colour: the accent, deeper
--                                    in light themes
--   picks.Code { code }              a pick's code chip
--   picks.Tray { items, on_clear }   the tray: a fixed slot per group (each
--                                    card's), free picks after
--   picks.Sent { chips }             a sent message's picks
--
-- An item (the composer's cargo, kind "pick"): { id, card, card_title,
-- group ("" for a free pick), group_name, code, label, groups = { { code,
-- name } }, locked = { code } }.

local Icon = require("@self/icon")
local control = require("@self/control")
local locale = require("@self/locale")

local M = {}

M.PICKED_IN = "Picked in"
M.NOT_PICKED = "Not picked:"

export type Pick = { group: string?, code: string, label: string }
export type Group = { code: string, name: string }
export type Card = { card: string, picks: { Pick }, missing: { Group } }

function M.block(card: string, picks: { Pick }, missing: { Group }): string
	local lines = { `{M.PICKED_IN} "{card}":` }
	for _, p in picks do
		if p.group and p.group ~= "" then
			table.insert(lines, `{p.group} = {p.code} ({p.label})`)
		else
			table.insert(lines, `- {p.label}`)
		end
	end
	if #missing > 0 then
		local names = {}
		for _, g in missing do
			table.insert(names, `{g.code} ({g.name})`)
		end
		table.insert(lines, `{M.NOT_PICKED} {table.concat(names, ", ")}`)
	end
	return table.concat(lines, "\n")
end

function M.with_picks(words: string, cards: { Card }): string
	local out = string.match(words, "^(.-)%s*$") or words
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

-- The cards a send's pick items make, in the order they were placed: each
-- card's picks in its groups' order (free picks as placed), and the groups
-- neither picked nor sent before.
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
		-- (A group the page no longer lists still goes, after the rest.)
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

-- A colour's hue, saturation and lightness, and back.
local function hsl(hex: string): (number, number, number)
	local r = (tonumber(string.sub(hex, 2, 3), 16) or 0) / 255
	local g = (tonumber(string.sub(hex, 4, 5), 16) or 0) / 255
	local b = (tonumber(string.sub(hex, 6, 7), 16) or 0) / 255
	local hi, lo = math.max(r, g, b), math.min(r, g, b)
	local l = (hi + lo) / 2
	local d = hi - lo
	if d < 1e-6 then
		return 0, 0, l
	end
	local s = d / (1 - math.abs(2 * l - 1))
	local h
	if hi == r then
		h = ((g - b) / d) % 6
	elseif hi == g then
		h = (b - r) / d + 2
	else
		h = (r - g) / d + 4
	end
	return h * 60, s, l
end

local function from_hsl(h: number, s: number, l: number): string
	local c = (1 - math.abs(2 * l - 1)) * s
	local x = c * (1 - math.abs((h / 60) % 2 - 1))
	local m = l - c / 2
	local r, g, b = 0, 0, 0
	if h < 60 then
		r, g = c, x
	elseif h < 120 then
		r, g = x, c
	elseif h < 180 then
		g, b = c, x
	elseif h < 240 then
		g, b = x, c
	elseif h < 300 then
		r, b = x, c
	else
		r, b = c, x
	end
	local function byte(v: number): number
		return math.clamp(math.floor((v + m) * 255 + 0.5), 0, 255)
	end
	return string.format("#%02x%02x%02x", byte(r), byte(g), byte(b))
end

-- The pick colour now (tracked): the theme's accent; in a light theme no
-- lighter than 48% nor more saturated than 57%, so the mock's #ec83b7
-- deepens to #c0347a on white and the tick and send read.
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
	local h, s, l = hsl(accent)
	return from_hsl(h, math.min(s, 0.57), math.min(l, 0.48))
end

local function raw(n: number, why: string): any
	return ui.raw(n, why)
end

function M.Code(props: { code: string, id: string? }): Node
	return ui.row {
		id = props.id,
		min_w = raw(28, "the mock's code chip"),
		h = raw(18, "the mock's code chip"),
		px = raw(6, "the mock's code chip"),
		items = "center",
		justify = "center",
		shrink = 0,
		rounded = raw(5, "the mock's code chip"),
		bg = "$surface.raised",
		border = 0.5,
		border_color = "$border",
		ui.text { props.code, literal = true, text_size = "$font.size.xs", weight = "$font.weight.semibold", color = "$text.primary" },
	}
end

local SLOT_H, SLOT_GAP, COLUMNS = 28, 6, 3
-- Narrower than this, a slot keeps its code alone (a split pane).
local NARROW = 420

-- `items`: the tray's picks; `on_clear(id)`: a slot's ×; `lit`: the pick id
-- that just landed, its slot lit for a moment.
M.Tray = ui.component(function(props: { items: () -> { any }, on_clear: (id: string) -> (), lit: (() -> string?)? }): Node
	local width = ui.use_state(0)
	local slots = ui.computed(function(): { any }
		local out = {}
		local seen: { [string]: boolean } = {}
		local list = props.items()
		for _, it in list do
			if not seen[it.card] then
				seen[it.card] = true
				local locked: { [string]: boolean } = {}
				for _, code in it.locked or {} do
					locked[code] = true
				end
				for _, g in it.groups or {} do
					if not locked[g.code] then
						local filled: any = nil
						for _, other in list do
							if other.card == it.card and other.group == g.code then
								filled = other
							end
						end
						table.insert(out, { id = `{it.card}-{g.code}`, name = g.name, item = filled })
					end
				end
				for _, other in list do
					if other.card == it.card and (other.group or "") == "" then
						table.insert(out, { id = other.id, item = other })
					end
				end
			end
		end
		return out
	end)
	local function slot_w(): number
		local inset = ui.theme.get("zeron.composer.chip.inset")
		local w = width:get() - 2 * (if type(inset) == "number" then inset else 16)
		if w <= 0 then
			return 160
		end
		return math.max(56, math.floor((w - SLOT_GAP * (COLUMNS - 1)) / COLUMNS))
	end
	local function narrow(): boolean
		local w = width:get()
		return w > 0 and w < NARROW
	end
	return ui.row {
		id = "composer-picks",
		label = locale.t("ds.html_picks.tray"),
		w_full = true,
		flex_wrap = true,
		gap = raw(SLOT_GAP, "the mock's tray"),
		px = "$zeron.composer.chip.inset",
		pt = "$space.2",
		on_resize = function(size: Size)
			if math.abs(width:peek() - size.w) >= 0.5 then
				width:set(size.w)
			end
		end,
		ui.each(slots, function(slot: any, _i: number, row: State<any>): Node
			local function item(): any
				return row:get().item
			end
			local function lit(): boolean
				local it = item()
				return it ~= nil and props.lit ~= nil and props.lit() == it.id
			end
			return ui.row {
				id = `pick-slot-{slot.id}`,
				w = slot_w,
				h = raw(SLOT_H, "the mock's slot"),
				shrink = 0,
				items = "center",
				gap = raw(6, "the mock's slot"),
				pl = raw(5, "the mock's slot"),
				pr = raw(4, "the mock's slot"),
				rounded = raw(8, "the mock's slot"),
				bg = function(): any
					if not item() then
						return "#00000000"
					end
					return ui.raw(M.ink() .. (if lit() then "40" else "24"), "the pick colour at 14%, lit as it lands")
				end,
				border = function(): number
					return if item() then 0.5 else 1
				end,
				border_dash = function(): any
					return if item() then nil else { 4, 3 }
				end,
				border_color = function(): any
					if not item() then
						return "$border"
					end
					return ui.raw(M.ink() .. "73", "the pick colour at 45%")
				end,
				ui.presence(function(): string
					return if item() then "filled" else "empty"
				end, function(kind: string): Node
					if kind == "empty" then
						return ui.text {
							row:peek().name or "",
							literal = true,
							w_full = true,
							align = "center",
							truncate = true,
							text_size = "$font.size.sm",
							color = "$text.tertiary",
						}
					end
					return ui.row {
						grow = 1,
						min_w = 0,
						items = "center",
						gap = raw(6, "the mock's slot"),
						M.Code {
							code = function(): string
								local it = item()
								return if it then (if (it.group or "") == "" then it.label else it.code) else ""
							end :: any,
						},
						ui.show(function(): boolean
							return not narrow() and item() ~= nil and (item().group or "") ~= ""
						end, ui.text {
							function(): string
								local it = item()
								return if it then it.label else ""
							end,
							literal = true,
							grow = 1,
							min_w = 0,
							truncate = true,
							text_size = "$font.size.sm",
							color = "$text.primary",
						}),
						ui.button {
							id = `pick-clear-{slot.id}`,
							label = locale.t("ds.html_picks.clear"),
							tooltip = locale.s("ds.html_picks.clear"),
							w = 20,
							h = 20,
							p = 0,
							ml = "auto",
							shrink = 0,
							items = "center",
							justify = "center",
							rounded = "$radius.sm",
							bg = "#00000000",
							color = "$text.tertiary",
							hover = { color = "$text.primary", bg = "$hover" },
							focus = control.ring,
							on_click = function()
								local it = item()
								if it then
									props.on_clear(it.id)
								end
							end,
							Icon { name = "close", size = 12 },
						},
					}
				end, { grow = 1, min_w = 0, h_full = true, items = "center" }),
			}
		end, {
			key = function(slot: any): string
				return slot.id
			end,
			direction = "row",
			flex_wrap = true,
			gap = raw(SLOT_GAP, "the mock's tray"),
			w_full = true,
		}),
	}
end)

-- A sent message's picks: each its code and label.
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
