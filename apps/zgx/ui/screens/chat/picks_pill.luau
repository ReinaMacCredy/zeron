local Icon = require("@ds/icon")
local P = require("@ds/pills")
local HtmlPicks = require("@ds/html_picks")
local locale = require("@ds/locale")
local motion = require("@ds/motion")
local store = require("@app/screens/chat/store")
local shared = require("@app/screens/chat/shared")
local Pages = require("@app/screens/chat/pages")

local M = {}

M.W, M.RADIUS = 520, 14
local MAX_H, INSET = 360, 8
local INK_ON = "#1a0b12"

local function raw(n: number, why: string): any
	return ui.raw(n, why)
end

local function ink(): any
	return raw(HtmlPicks.ink() :: any, "the pick colour")
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

local function marker(on: () -> boolean): Node
	return ui.row {
		w = raw(18, "the mockup's marker"),
		h = raw(18, "the mockup's marker"),
		shrink = 0,
		items = "center",
		justify = "center",
		rounded = "$radius.full",
		border = 1,
		border_color = function(): string
			return if on() then "#00000000" else "$border.strong"
		end,
		bg = function(): any
			return if on() then ink() else "#00000000"
		end,
		ui.div {
			opacity = function(): number
				return if on() then 1 else 0
			end,
			Icon { name = "check", size = 11, color = raw(INK_ON :: any, "the mockup's ink on the pick colour") },
		},
	}
end

local function option_row(chat: string, e: any, g: any, o: any, on: () -> boolean): Node
	return ui.button {
		id = `picks-option-{g.code}-{o.code}`,
		class = "picks-option",
		label = `{o.code} {o.label}`,
		w_full = true,
		px = raw(6, "the mockup's option row"),
		py = raw(7, "the mockup's option row"),
		gap = raw(9, "the mockup's option row"),
		items = "start",
		rounded = raw(8, "the mockup's option row"),
		bg = function(): string
			return if on() then "$zeron.secondary.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		selected = on,
		on_click = function()
			local groups = {}
			for _, x in e.groups do
				table.insert(groups, { code = x.code, name = x.name })
			end
			Pages.place(chat, e.card, e.title, { group = g.code, code = o.code, label = o.label, groups = groups }, Pages.sent_of(Pages.with_queue(chat, store.messages:peek()), e.title, e.message))
		end,
		ui.div { mt = raw(2, "the mockup's option marker"), shrink = 0, marker(on) },
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text { o.label, literal = true, weight = "$font.weight.medium", color = "$text.primary" },
			if o.desc and o.desc ~= "" then ui.text { o.desc, literal = true, text_size = "$font.size.sm", color = "$text.tertiary" } else nil,
		},
		ui.row {
			mt = raw(3, "the mockup's option code"),
			h = raw(16, "the mockup's option code"),
			min_w = raw(24, "the mockup's option code"),
			px = raw(4, "the mockup's option code"),
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = raw(4, "the mockup's option code"),
			bg = function(): any
				return if on() then ink() else "$zeron.secondary.bg"
			end,
			ui.text {
				o.code,
				literal = true,
				wrap = false,
				text_size = raw(11, "the mockup's option code"),
				weight = "$font.weight.semibold",
				color = function(): any
					return if on() then raw(INK_ON :: any, "the mockup's ink on the pick colour") else "$text.tertiary"
				end,
			},
		},
	}
end

local Group = ui.component(function(props: { chat: string, e: any, g: any, index: number, cur: any, picks: any }): Node
	local e, g = props.e, props.g
	local hover = ui.use_state(false)
	local reduced = motion.reduced_now()
	local function picked(): string?
		return props.picks:get().picks[g.code]
	end
	local function locked(): boolean
		return props.picks:get().locked[g.code] == true
	end
	local function line(): Node
		return ui.button {
			id = `picks-group-{g.code}`,
			class = "picks-group",
			label = g.name,
			w_full = true,
			min_h = raw(34, "the mockup's folded group"),
			px = raw(6, "the mockup's folded group"),
			gap = raw(9, "the mockup's folded group"),
			items = "center",
			rounded = raw(8, "the mockup's folded group"),
			bg = "#00000000",
			hover = { bg = "$hover" },
			text_size = "$font.size.sm",
			disabled = locked,
			on_hover = function(inside: boolean)
				hover:set(inside)
			end,
			on_click = function()
				props.cur:set(g.code)
			end,
			marker(function(): boolean
				return picked() ~= nil
			end),
			ui.text { g.name, literal = true, shrink = 1, min_w = 0, truncate = true, color = "$text.tertiary" },
			ui.show(function(): boolean
				return picked() ~= nil
			end, ui.text { "→", shrink = 0, wrap = false, color = "$text.tertiary" }),
			ui.text { function(): string
				local code = picked()
				for _, o in g.options or {} do
					if o.code == code then
						return o.label
					end
				end
				return code or ""
			end, literal = true, shrink = 100, min_w = 0, truncate = true, color = "$text.primary" },
			ui.div { grow = 1 },
			ui.text {
				function(): string
					return locale.s(if picked() then "ds.html_picks.change" else "ds.html_picks.not_picked")
				end,
				shrink = 0,
				wrap = false,
				text_size = "$font.size.xs",
				color = "$text.tertiary",
				opacity = function(): number
					return if not picked() or (hover:get() and not locked()) then 1 else 0
				end,
			},
		}
	end
	local function block(): Node
		local rows: { Node } = {
			ui.row {
				items = "baseline",
				gap = raw(8, "the mockup's group head"),
				px = raw(6, "the mockup's group head"),
				pb = raw(4, "the mockup's group head"),
				ui.text { `{props.index}/{#e.groups}`, literal = true, shrink = 0, wrap = false, tabular = true, text_size = raw(11, "the mockup's group index"), color = "$text.tertiary" },
				ui.text { g.name, literal = true, min_w = 0, weight = "$font.weight.semibold", color = "$text.primary" },
			},
		}
		for _, o in g.options or {} do
			table.insert(rows, option_row(props.chat, e, g, o, function(): boolean
				return picked() == o.code
			end))
		end
		return ui.column { id = `picks-group-open-{g.code}`, w_full = true, pt = raw(8, "the mockup's open group"), pb = raw(6, "the mockup's open group"), rows }
	end
	return ui.column {
		w_full = true,
		min_w = 0,
		border_sides = { 1, 0, 0, 0 },
		border_color = "$border",
		overflow = "hidden",
		layout = "height",
		layout_clip = true,
		layout_duration = if reduced then 0 else 200,
		layout_ease = "cubicBezier(0.2, 0.8, 0.2, 1)",
		ui.presence(function(): string
			return if props.cur:get() == g.code then "open" else "line"
		end, function(which: string): Node
			return if which == "open" then block() else line()
		end, { w_full = true, min_w = 0, enter = { opacity = { 0, 1 }, duration = if reduced then motion.FADE else 120 } }),
	}
end)

local Tray = ui.component(function(props: { chat: string, e: any, close: () -> () }): Node
	local chat, e = props.chat, props.e
	local picks = ui.computed(function(): any
		local sent = Pages.sent_of(Pages.with_queue(chat, store.messages:get()), e.title, e.message)
		local out: { [string]: string } = {}
		for _, s in sent.list do
			if s.group ~= "" then
				out[s.group] = s.code
			end
		end
		for g, c in chosen_of(chat, e.card) do
			out[g] = c
		end
		return { picks = out, locked = sent.locked }
	end)
	local function next_open(after: string?): string
		local p = picks:peek().picks
		local at = 0
		for i, g in e.groups do
			if g.code == after then
				at = i
			end
		end
		for k = 1, #e.groups do
			local g = e.groups[(at + k - 1) % #e.groups + 1]
			if not p[g.code] then
				return g.code
			end
		end
		return ""
	end
	local cur = ui.use_state(next_open(nil))
	local function done(): number
		local n = 0
		for _, g in e.groups do
			n += if picks:get().picks[g.code] then 1 else 0
		end
		return n
	end
	local was = picks:peek().picks
	ui.effect(function()
		local now = picks:get().picks
		local moved: string? = nil
		for _, g in e.groups do
			if now[g.code] and now[g.code] ~= was[g.code] then
				moved = g.code
			end
		end
		was = now
		if not moved then
			return
		end
		cur:set(next_open(moved))
		if cur:peek() == "" then
			ui.after(900, function()
				if cur:peek() == "" and next_open(nil) == "" then
					props.close()
				end
			end)
		end
	end)
	local seen = shared.draft_peek(chat).text
	ui.effect(function()
		local text = shared.draft_of(chat).text
		if text ~= seen then
			seen = text
			if not Pages.take_placed(chat) then
				props.close()
			end
		end
	end)
	local hover = ui.use_state(false)
	local segs: { Node } = {}
	for _, g in e.groups do
		table.insert(segs, ui.div {
			w = raw(14, "the mockup's progress"),
			h = raw(4, "the mockup's progress"),
			rounded = raw(2, "the mockup's progress"),
			bg = function(): any
				return if picks:get().picks[g.code] then ink() else "$border.strong"
			end,
		})
	end
	local rows: { Node } = {
		ui.button {
			id = "picks-card-head",
			label = locale.t("ds.html_picks.collapse"),
			w_full = true,
			items = "center",
			gap = "$space.2",
			px = raw(4, "the mockup's card header"),
			pt = raw(4, "the mockup's card header"),
			pb = raw(6, "the mockup's card header"),
			rounded = raw(8, "the mockup's card header"),
			bg = "#00000000",
			on_hover = function(inside: boolean)
				hover:set(inside)
			end,
			on_click = props.close,
			Icon { name = "document", size = 14, color = "$text.tertiary" },
			ui.text { e.title, literal = true, shrink = 1, min_w = 0, truncate = true, text_size = "$font.size.sm", color = "$text.secondary" },
			ui.div { grow = 1 },
			ui.row { gap = raw(3, "the mockup's progress"), shrink = 0, items = "center", segs },
			ui.text { function(): string
				return `{done()}/{#e.groups}`
			end, id = "picks-card-count", shrink = 0, wrap = false, tabular = true, text_size = "$font.size.xs", color = "$text.tertiary" },
			ui.row {
				w = raw(22, "the mockup's chevron"),
				h = raw(22, "the mockup's chevron"),
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = raw(6, "the mockup's chevron"),
				bg = function(): string
					return if hover:get() then "$hover" else "#00000000"
				end,
				color = function(): string
					return if hover:get() then "$text.primary" else "$text.tertiary"
				end,
				Icon { name = "alt-arrow-down", size = 14 },
			},
		},
	}
	for i, g in e.groups do
		table.insert(rows, Group { chat = chat, e = e, g = g, index = i, cur = cur, picks = picks })
	end
	table.insert(rows, ui.row {
		w_full = true,
		items = "center",
		gap = raw(8, "the mockup's footer"),
		px = raw(6, "the mockup's footer"),
		pt = raw(7, "the mockup's footer"),
		pb = raw(2, "the mockup's footer"),
		border_sides = { 1, 0, 0, 0 },
		border_color = "$border",
		ui.text { function(): string
			return locale.s(if done() < #e.groups then "ds.html_picks.hint" else "ds.html_picks.hint_done")
		end, id = "picks-card-hint", grow = 1, min_w = 0, text_size = "$font.size.xs", color = "$text.tertiary" },
		ui.button {
			id = "picks-card-skip",
			label = locale.t("ds.html_picks.skip_page"),
			shrink = 0,
			p = 0,
			bg = "#00000000",
			color = "$text.tertiary",
			hover = { color = "$text.primary" },
			on_click = function()
				props.close()
				store.close_page(chat, e.title)
			end,
			ui.text { locale.t("ds.html_picks.skip_page"), wrap = false, underline = true, text_size = "$font.size.xs" },
		},
	})
	local content_h = ui.use_state(0)
	return ui.scroll {
		id = "picks-card",
		w_full = true,
		focus_on_press = false,
		h = function(): number
			return math.min(content_h:get() + 2 * INSET, MAX_H)
		end,
		p = raw(INSET, "the mockup's tray"),
		ui.column {
			w_full = true,
			on_resize = function(size: { w: number, h: number })
				if math.abs(content_h:peek() - size.h) >= 0.5 then
					content_h:set(size.h)
				end
			end,
			rows,
		},
	}
end)

function M.Card(props: { chat: () -> string?, close: () -> () }): Node
	local page = ui.computed(function(): any
		return (Pages.waiting(props.chat(), store.messages:get()))
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
		return Tray { chat = chat, e = e, close = props.close }
	end, { w_full = true })
end

function M.Pill(props: { open: any, chat: () -> string?, on_press: () -> (), on_click: () -> () }): Node
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
			return if not w then "" elseif w.left == 0 then locale.s("ds.html_picks.pill_done", { title = w.title }) else locale.s("ds.html_picks.pill", { title = w.title, n = w.left })
		end,
		active = function(): boolean
			return props.open:get()
		end,
		expanded = function(): boolean
			return props.open:get()
		end,
		pl = raw(8, "the mockup's pill"),
		gap = raw(6, "the mockup's pill"),
		shrink = 100,
		min_w = 0,
		on_press = props.on_press,
		on_click = props.on_click,
		ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = ink },
		ui.text { function(): string
			local w = waiting:get()
			return if w then w.title else ""
		end, id = "pills-picks-title", literal = true, shrink = 1, min_w = 0, truncate = true, color = "$text.primary" },
		ui.text { function(): string
			local w = waiting:get()
			return if not w then "" elseif w.left == 0 then "· " .. locale.s("ds.html_picks.left_done") else "· " .. locale.s("ds.html_picks.left", { n = w.left })
		end, id = "pills-picks-count", shrink = 0, wrap = false, color = "$text.tertiary" },
	}
	return ui.presence(function(): string
		return if waiting:get() then "picks" else ""
	end, function(which: string): Node
		return if which == "" then ui.div { w = 0, h = 0 } else pill
	end, { enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 150 }, exit = { opacity = 0, duration = 120 }, shrink = 100, min_w = 0 })
end

return M
