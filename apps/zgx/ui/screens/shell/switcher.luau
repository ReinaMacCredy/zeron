-- crates/ui shell/switcher.rs.

local ds = require("@ds")
local Glass = require("@ds/glass")
local SessionRow = require("@ds/session_row")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local view = require("@app/screens/shell/view")
local art = require("@app/screens/shell/art")
local tabs = require("@app/screens/shell/session_tabs")
local chat_shared = require("@app/screens/chat/shared")
local refocus = require("@app/screens/shell/refocus")
local swipe = require("@app/screens/shell/swipe")
local Pictures = require("@app/screens/shell/switcher_pictures")
local boundary = require("@app/screens/shell/boundary")

local M = {}

local CARD_W, THUMB_H, THUMB_PX = 220, 140, 440
local CARD_OUTER = CARD_W + 2 * 8 + 2
local CARD_GAP, PANEL_PAD = 6, 10
local PICTURES = 40
-- (A chat left sooner than this has hardly drawn: no picture, and none of its cost when
-- going through them fast.)
local SETTLED_MS = 300
-- (A session's card shuts on the press that goes to another chat; the frame showing still
-- has it.)
local CARD_LINGER_MS = 100

M.window = ui.persist("shell_switcher_window", "all")
M.limit = ui.persist("shell_switcher_limit", 0)
M.cycles_tabs = ui.persist("shell_switcher_cycles_tabs", false)
M.WINDOWS = {
	{ id = "all", hours = nil },
	{ id = "1h", hours = 1 },
	{ id = "2h", hours = 2 },
	{ id = "4h", hours = 4 },
	{ id = "8h", hours = 8 },
	{ id = "12h", hours = 12 },
	{ id = "24h", hours = 24 },
	{ id = "3d", hours = 72 },
	{ id = "7d", hours = 168 },
}
local function cutoff(now: number): number?
	for _, w in M.WINDOWS do
		if w.id == M.window:peek() and w.hours then
			return now - w.hours * 3600
		end
	end
	return nil
end

local cards = ui.state("shell_switcher_cards", nil :: { string }?)
local card_list = ui.computed(function(): { string }
	return cards:get() or {}
end)
local hi = ui.state("shell_switcher_hi", 1)
local held = ui.modifiers

local pictures = Pictures.Book(PICTURES)
local column: any = nil
function M.set_column(ref: any)
	column = ref
end
local card_was_up = false
local card_gone_at = -math.huge
ui.effect(function()
	local sidebar: any = boundary.load("@app/screens/shell/sidebar")
	local up = sidebar ~= nil and sidebar.card:get() ~= nil
	if card_was_up and not up then
		card_gone_at = ui.now()
	end
	card_was_up = up
end)
local shown_since = ui.now()
-- (A wrong picture is worse than an older one, or none.)
local function picture_of(id: string)
	local el = column and column.element
	if not el or el:covered() or swipe.showing() or card_was_up or ui.now() - card_gone_at < CARD_LINGER_MS or ui.now() - shown_since < SETTLED_MS then
		return
	end
	pictures.take(id, function(slot: string): string?
		local ok, src = pcall(el.snapshot, el, THUMB_PX, slot)
		return if ok then src else nil
	end)
end
local leaving: string? = route.session:peek() and (route.session:peek() :: any).id
local on_chat = route.current:peek().name == "session"
ui.effect(function()
	local s = route.session:get()
	local id = if s then s.id else nil
	if leaving and id ~= leaving and on_chat then
		picture_of(leaving)
	end
	if id ~= leaving then
		shown_since = ui.now()
	end
	leaving = id
	on_chat = route.current:get().name == "session"
end)

local function chat_row(id: string): any
	for _, c in sessions.now:peek().chats do
		if c.id == id then
			return c
		end
	end
	return nil
end

local function here_now(): string?
	local s = route.session:peek()
	return if s then s.id else nil
end

local function strip(live: boolean): { string }
	local out: { string } = {}
	local here = here_now()
	for _, key in tabs.shown:peek() do
		local id = tabs.face(key)
		local c = chat_row(id)
		if c and (not live or c.status or id == here) then
			table.insert(out, id)
		end
	end
	return out
end

function M.order(recent: { string }, sidebar: { string }, live_tabs: { string }, updated: (id: string) -> number?, since: number?, limit: number): { string }
	local candidates: { string } = {}
	for _, id in sidebar do
		local at = updated(id)
		if at and (not since or at >= since) then
			table.insert(candidates, id)
		end
	end
	for _, id in live_tabs do
		if not table.find(candidates, id) then
			table.insert(candidates, id)
		end
	end
	local out: { string } = {}
	for _, id in recent do
		if table.find(candidates, id) then
			table.insert(out, id)
		end
	end
	for _, id in candidates do
		if not table.find(out, id) then
			table.insert(out, id)
		end
	end
	if limit > 0 and #out > limit then
		local capped = table.move(out, 1, limit, 1, {})
		for _, id in live_tabs do
			if not table.find(capped, id) then
				table.insert(capped, id)
			end
		end
		return capped
	end
	return out
end

local function order_now(sidebar: { string }, going_home: boolean): { string }
	local list = M.order(tabs.recent:peek(), sidebar, strip(true), function(id: string): number?
		local c = chat_row(id)
		return if c then c.updated else nil
	end, cutoff(os.time()), M.limit:peek())
	if going_home or route.chat:peek().name == "home" then
		table.insert(list, 1, tabs.DRAFT)
	else
		-- (The first press goes one on from where it is: where it is comes first.)
		local at = table.find(list, here_now() or "")
		if at and at > 1 then
			table.insert(list, 1, table.remove(list, at) :: string)
		end
		if tabs.draft_shows:peek() then
			table.insert(list, tabs.DRAFT)
		end
	end
	return list
end

local function go(id: string?)
	if id == tabs.DRAFT then
		route.new_chat()
	elseif id then
		route.open_session(id)
	end
	refocus()
end

function M.commit()
	local list = cards:peek()
	if not list then
		return
	end
	local id = list[hi:peek()]
	cards:set(nil)
	local here = if route.chat:peek().name == "home" then tabs.DRAFT else here_now()
	-- (A chat deleted since the cards came up is no place to land.)
	if id ~= here and (id == tabs.DRAFT or chat_row(id)) then
		go(id)
	end
end

function M.close()
	cards:set(nil)
end

local active: any = (ui :: any).window and (ui :: any).window.active
ui.effect(function()
	local down = held.control:get() or held.meta:get() or held.alt:get()
	-- (The window losing focus lets go of what's held here; nobody let go on a card.)
	local away = active ~= nil and active:get() == false
	-- (Up with nothing held, a tap or the Go menu: it lands at once, as Zeron's.)
	if not cards:get() then
		return
	end
	if away then
		M.close()
	elseif not down then
		M.commit()
	end
end)

function M.step(by: number): boolean
	local list = cards:peek()
	if list then
		hi:set((hi:peek() - 1 + by) % #list + 1)
		return true
	end
	local el = column and column.element
	if view.palette:peek() or (el and el:overlaid()) then
		return true
	end
	local where = route.current:peek().name
	if where == "settings" then
		route.leave_settings()
	elseif where == "plugin" then
		route.new_chat()
	end
	if M.cycles_tabs:peek() then
		local strip_now = strip(false)
		if tabs.draft_shows:peek() then
			table.insert(strip_now, tabs.DRAFT)
		end
		if #strip_now == 0 then
			return true
		end
		local here = if route.chat:peek().name == "home" then tabs.DRAFT else here_now()
		local at = if here then table.find(strip_now, here) else nil
		local i = if at then (at - 1 + by) % #strip_now + 1 elseif by > 0 then 1 else #strip_now
		go(strip_now[i])
		return true
	end
	local sidebar: any = boundary.load("@app/screens/shell/sidebar")
	local order = order_now(if sidebar then sidebar.order:peek() else {}, where == "plugin")
	if #order == 0 then
		return true
	elseif #order == 1 then
		if route.chat:peek().name == "home" and order[1] ~= tabs.DRAFT then
			go(order[1])
		end
		return true
	end
	local s = route.session:peek()
	if s and where == "session" then
		picture_of(s.id)
	end
	local start = (1 - 1 + by) % #order + 1
	ui.batch(function()
		hi:set(start)
		cards:set(order)
	end)
	return true
end

local STATE_WORDS: { [string]: string } = {
	working = "shell.switcher.working",
	connecting = "shell.switcher.working",
	input = "shell.switcher.asking",
	approval = "shell.switcher.asking",
	failed = "shell.switcher.failed",
	limited = "shell.switcher.failed",
	done = "shell.switcher.done",
}

local function target_of(c: any): string
	local now = sessions.now:peek()
	local folder = "~"
	for _, p in now.projects do
		if p.id == c.project then
			folder = p.name
		end
	end
	for _, d in now.devices do
		if d.id == c.device then
			return `{folder} @ {d.name}`
		end
	end
	return folder
end

local function Card(id: string, at: number): Node
	local is_draft = id == tabs.DRAFT
	local c = if is_draft then nil else chat_row(id)
	local function lit(): boolean
		return hi:get() == at
	end
	local picture = pictures.src(id)
	local text = if is_draft then chat_shared.draft_of("").text elseif c then c.preview else nil
	local visual: Node = if picture
		then ui.image {
			src = picture,
			w = CARD_W,
			h = THUMB_H,
			fit = "cover",
			rounded = ui.raw(8, "Zeron's picture's corner"),
			border = 1,
			border_color = "$border",
		}
		else ui.div {
			w = CARD_W,
			h = THUMB_H,
			overflow = "hidden",
			rounded = ui.raw(8, "Zeron's picture's corner"),
			border = 1,
			border_color = "$border",
			bg = "$hover",
			p = ui.raw(10, "Zeron's text's inset"),
			color = "$text.secondary",
			ui.text { if text and text ~= "" then text else ui.t("shell.switcher.no_messages"), wrap = true, text_size = "$font.size.xs" },
		}
	local title: any = if c then c.title else ui.t("shell.tabs.new_session")
	local word = if c and c.status then STATE_WORDS[c.status] else nil
	return ui.column {
		id = `switcher-card-{id}`,
		role = "button",
		label = title,
		selected = lit,
		shrink = 0,
		wrap = false,
		p = ui.raw(8, "Zeron's card's inset"),
		rounded = ui.raw(12, "Zeron's card's corner"),
		cursor = "pointer",
		border = 1,
		border_color = function(): string
			return if lit() then "$border" else "#00000000"
		end,
		bg = function(): string
			return if lit() then "$zeron.selected.bg" else "#00000000"
		end,
		-- (Moved over it, not its coming up under a pointer at rest, which would be where a quick press lands.)
		on_pointer_move = function()
			if hi:peek() ~= at then
				hi:set(at)
			end
		end,
		on_click = function()
			hi:set(at)
			M.commit()
		end,
		visual,
		ui.row {
			mt = ui.raw(8, "Zeron's gap under the picture"),
			w = CARD_W,
			gap = ui.raw(6, "Zeron's gap in a card's foot"),
			items = "center",
			if c
				then SessionRow.mark(function(): any
					return c
				end, function(): boolean
					return true
				end)
				else ui.div { color = "$warning", ds.Icon { name = "pen", size = 13 } },
			ds.FadeText { title, grow = 1, min_w = 0, text_size = "$font.size.sm", weight = "$font.weight.medium" },
		},
		ui.row {
			w = CARD_W,
			gap = ui.raw(6, "Zeron's gap in a card's foot"),
			items = "center",
			color = "$text.tertiary",
			ds.FadeText { if c then target_of(c) else "~", grow = 1, min_w = 0, text_size = ui.raw(11, "Zeron's card's foot") },
			if word
				then ui.text {
					ui.t(word),
					shrink = 0,
					text_size = ui.raw(11, "Zeron's card's foot"),
					color = `$zeron.status.{c.status}`,
				}
				else nil,
		},
	}
end

function M.Overlay(): Node
	local function width(): number
		local n = #(cards:get() or {})
		local most = math.max(art.window:get().w - 48, CARD_OUTER + 2 * PANEL_PAD)
		local across = math.max(math.floor((most - 2 * PANEL_PAD + CARD_GAP) / (CARD_OUTER + CARD_GAP)), 1)
		local fit = math.clamp(n, 1, across)
		return fit * CARD_OUTER + (fit - 1) * CARD_GAP + 2 * PANEL_PAD + 2
	end
	return ui.modal {
		open = function(): boolean
			return cards:get() ~= nil
		end,
		on_close = function()
			M.close()
		end,
		label = ui.t("shell.switcher.label"),
		w = width,
		p = 0,
		gap = 0,
		bg = "#00000000",
		rounded = ui.raw(16, "Zeron's switcher's corner"),
		build = function()
			local panel: { [any]: any } = {
				id = "session-switcher",
				w_full = true,
				rounded = ui.raw(16, "Zeron's switcher's corner"),
				autofocus = true,
				focusable = true,
				focus = { outline = 0, border = 0 },
				-- (Tab moves on, not to the next control, and no command runs under it.)
				capture_keys = true,
				on_key = function(e: any): boolean?
					if e.key == "Tab" then
						M.step(if e.shift then -1 else 1)
					elseif e.key == "Escape" then
						M.close()
					elseif e.key == "Enter" then
						M.commit()
					end
					return true
				end,
				max_h = function(): number
					return math.max(art.window:get().h - 48, 200)
				end,
				ui.scroll {
					w_full = true,
					ui.row {
						w_full = true,
						wrap = true,
						p = ui.raw(PANEL_PAD, "Zeron's switcher's inset"),
						gap = ui.raw(CARD_GAP, "Zeron's gap between cards"),
						ui.each(card_list, function(id: string, at: number): Node
							return Card(id, at)
						end, { key = function(id: string): string
							return id
						end, direction = "row", wrap = true, gap = ui.raw(CARD_GAP, "Zeron's gap between cards") }),
					},
				},
			}
			Glass.style(panel, "menu")
			return ui.column(panel)
		end,
	}
end

return M
