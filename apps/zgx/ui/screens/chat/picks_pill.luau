local Icon = require("@ds/icon")
local P = require("@ds/pills")
local Menu = require("@ds/menu")
local HtmlPicks = require("@ds/html_picks")
local locale = require("@ds/locale")
local store = require("@app/screens/chat/store")
local shared = require("@app/screens/chat/shared")
local Pages = require("@app/screens/chat/pages")

local M = {}

local CARD_W = 480

local function raw(n: number, why: string): any
	return ui.raw(n, why)
end

local function chosen_of(chat: string, card: string): { [string]: string }
	local out: { [string]: string } = {}
	for _, it in shared.cargo_of(chat) do
		if it.kind == "pick" and it.card == card then
			out[it.group or ""] = it.code
		end
	end
	return out
end

local function option_row(chat: string, e: any, g: any, o: any, close: () -> ()): Node
	local function on(): boolean
		return chosen_of(chat, e.card)[g.code] == o.code
	end
	return ui.button {
		id = `picks-option-{g.code}-{o.code}`,
		class = "picks-option",
		label = `{o.code} {o.label}`,
		w_full = true,
		h = raw(32, "the mockup's option row"),
		px = raw(6, "the mockup's option row"),
		gap = raw(8, "the mockup's option row"),
		items = "center",
		rounded = raw(7, "the mockup's option row"),
		bg = "#00000000",
		hover = { bg = "$hover" },
		selected = on,
		on_click = function()
			local groups = {}
			for _, x in e.groups do
				table.insert(groups, { code = x.code, name = x.name })
			end
			Pages.place(chat, e.card, e.title, { group = g.code, code = o.code, label = o.label, groups = groups }, Pages.sent_of(store.messages:peek(), e.title, e.message))
			local _, left = Pages.waiting(chat, store.messages:peek())
			if left == 0 then
				close()
			end
		end,
		ui.row {
			w = raw(24, "the mockup's option code"),
			h = raw(16, "the mockup's option code"),
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = raw(4, "the mockup's option code"),
			bg = function(): any
				return if on() then raw(HtmlPicks.ink() :: any, "the pick colour") else "$surface.raised"
			end,
			ui.text {
				o.code,
				literal = true,
				truncate = true,
				text_size = "$font.size.xs",
				weight = "$font.weight.semibold",
				color = function(): any
					return if on() then raw("#1a0b12" :: any, "the mockup's ink on the pick colour") else "$text.secondary"
				end,
			},
		},
		ui.text { o.label, literal = true, shrink = 0, wrap = false, text_size = "$font.size.sm", color = "$text.primary" },
		ui.text { o.desc or "", literal = true, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.xs", color = "$text.tertiary" },
		ui.div {
			w = 14,
			shrink = 0,
			opacity = function(): number
				return if on() then 1 else 0
			end,
			Icon { name = "check", size = 14, color = raw(HtmlPicks.ink() :: any, "the pick colour") },
		},
	}
end

local function Card(props: { chat: () -> string?, close: () -> () }): Node
	local page = ui.computed(function(): any
		local e = Pages.waiting(props.chat(), store.messages:get())
		return e
	end)
	return ui.presence(function(): string
		local e = page:get()
		return if e then e.card else ""
	end, function(card: string): Node
		local e = page:peek()
		local chat = props.chat()
		if card == "" or not e or not chat then
			return ui.div {}
		end
		local rows: { Node } = {
			ui.row {
				w_full = true,
				items = "center",
				gap = "$space.2",
				px = raw(4, "the mockup's card header"),
				pb = raw(6, "the mockup's card header"),
				Icon { name = "document", size = 14, color = "$text.tertiary" },
				ui.text { e.title, literal = true, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.sm", color = "$text.secondary" },
				ui.button {
					id = "picks-card-close",
					label = locale.t("ds.html_picks.close"),
					w = 20,
					h = 20,
					p = 0,
					items = "center",
					justify = "center",
					rounded = "$radius.sm",
					bg = "#00000000",
					color = "$text.tertiary",
					hover = { color = "$text.primary", bg = "$hover" },
					on_click = props.close,
					Icon { name = "close", size = 12 },
				},
			},
		}
		for _, g in e.groups do
			table.insert(rows, ui.text { g.name, literal = true, px = raw(4, "the mockup's group head"), pt = raw(6, "the mockup's group head"), pb = raw(3, "the mockup's group head"), text_size = "$font.size.xs", color = "$text.tertiary" })
			for _, o in g.options or {} do
				table.insert(rows, option_row(chat, e, g, o, props.close))
			end
		end
		return ui.column { id = "picks-card", w_full = true, p = raw(8, "the mockup's card"), rows }
	end, { w_full = true })
end

function M.Pill(props: { open: any, width: () -> number, chat: () -> string? }): Node
	local waiting = ui.computed(function(): any
		local e, left = Pages.waiting(props.chat(), store.messages:get())
		return if e then { title = e.title, left = left } else nil
	end)
	ui.effect(function()
		if not waiting:get() and props.open:peek() then
			props.open:set(false)
		end
	end)
	local pill = P.Pill {
		id = "pills-picks",
		class = "pills-pill",
		label = function(): string
			local w = waiting:get()
			return if w then locale.s("ds.html_picks.pill", { title = w.title, n = w.left }) else ""
		end,
		active = function(): boolean
			return props.open:get()
		end,
		expanded = function(): boolean
			return props.open:get()
		end,
		pl = raw(8, "the mockup's pill"),
		pr = raw(5, "the X at the pill's end"),
		gap = raw(6, "the mockup's pill"),
		shrink = 100,
		min_w = 0,
		on_click = function()
			props.open:set(not props.open:peek())
		end,
		ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = function(): any
			return raw(HtmlPicks.ink() :: any, "the pick colour")
		end },
		ui.text { function(): string
			local w = waiting:get()
			return if w then w.title else ""
		end, id = "pills-picks-title", literal = true, shrink = 1, min_w = 0, truncate = true, color = "$text.primary" },
		ui.text { function(): string
			local w = waiting:get()
			return if w then "· " .. locale.s("ds.html_picks.left", { n = w.left }) else ""
		end, id = "pills-picks-count", shrink = 0, wrap = false, color = "$text.tertiary" },
		ui.button {
			id = "pills-picks-close",
			label = locale.t("ds.html_picks.close_page"),
			w = 18,
			h = 18,
			p = 0,
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = "$radius.full",
			bg = "#00000000",
			color = "$text.tertiary",
			hover = { color = "$text.primary", bg = "$hover" },
			on_click = function()
				local w, chat = waiting:peek(), props.chat()
				if w and chat then
					props.open:set(false)
					store.close_page(chat, w.title)
				end
			end,
			Icon { name = "close", size = 10 },
		},
	}
	return ui.presence(function(): string
		return if waiting:get() then "picks" else ""
	end, function(which: string): Node
		if which == "" then
			return ui.div { w = 0, h = 0 }
		end
		return ui.popover(Menu.panel {
			class = "pills-picks-panel",
			open = props.open,
			anchor = pill,
			w = function(): number
				return math.min(CARD_W, props.width())
			end,
			placement = { side = "top", align = "start", offset = 8 },
			build = function(): Node
				return P.while_open(props.open, function(): Node
					return Card {
						chat = props.chat,
						close = function()
							props.open:set(false)
						end,
					}
				end)
			end,
		})
	end, { enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 150 }, exit = { opacity = 0, duration = 120 }, shrink = 100, min_w = 0 })
end

return M
