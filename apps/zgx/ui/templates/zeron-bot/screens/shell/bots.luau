-- Zeron Bot's sidebar: the Bots first (each a lead chat: its avatar, its
-- name, the last thing it said), then what needs you, then the sessions by
-- folder, each with a status line. A Bot is a pinned chat, or, with none
-- pinned, a chat that has started others, or else one of the latest chats;
-- folders are the sidebar's sections.

local ds = require("@ds")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local boundary = require("@app/screens/shell/boundary")

local W = 264
local NEEDS = { input = true, failed = true, approval = true, waiting = true }

local function chats_by_id(): { [string]: any }
	local by = {}
	for _, c in sessions.now:get().chats do
		by[c.id] = c
	end
	return by
end

local function bots(): { any }
	local by = chats_by_id()
	local out = {}
	for _, id in sessions.places:get().pinned do
		local c = by[id]
		if c then
			table.insert(out, c)
		end
	end
	if #out == 0 then
		for _, c in sessions.now:get().chats do
			if (c.side_chats or 0) > 0 or (c.running_subagents or 0) > 0 then
				table.insert(out, c)
			end
		end
	end
	-- None yet: the latest chats stand in, so a first chat is a Bot at once.
	if #out == 0 then
		for _, c in sessions.now:get().chats do
			table.insert(out, c)
		end
		table.sort(out, function(a: any, b: any): boolean
			return (a.updated or 0) > (b.updated or 0)
		end)
		while #out > 5 do
			table.remove(out)
		end
	end
	return out
end

local function needs(): { any }
	local out = {}
	for _, c in sessions.now:get().chats do
		if c.status and NEEDS[c.status] then
			table.insert(out, c)
		end
	end
	table.sort(out, function(a: any, b: any): boolean
		return (a.updated or 0) > (b.updated or 0)
	end)
	return out
end

local function folders(): { any }
	local by = chats_by_id()
	local placed: { [string]: boolean } = {}
	for _, id in sessions.places:get().pinned do
		placed[id] = true
	end
	local out = {}
	for _, s in sessions.places:get().sections do
		local rows = {}
		for _, id in s.sessions do
			local c = by[id]
			if c then
				placed[id] = true
				table.insert(rows, c)
			end
		end
		table.insert(out, { id = s.id, name = s.name, rows = rows })
	end
	local rest = {}
	for _, c in sessions.now:get().chats do
		if not placed[c.id] then
			table.insert(rest, c)
		end
	end
	table.sort(rest, function(a: any, b: any): boolean
		return (a.updated or 0) > (b.updated or 0)
	end)
	while #rest > 12 do
		table.remove(rest)
	end
	if #rest > 0 then
		table.insert(out, { id = "recent", name = "Recent", rows = rest })
	end
	return out
end

local function status_line(c: any): (string, string)
	local s = c.status or ""
	if s == "working" then
		return "Working", "$text.tertiary"
	elseif s == "input" or s == "approval" or s == "waiting" then
		return "Needs you", "$warning"
	elseif s == "failed" or s == "aborted" then
		return "Failed", "$danger"
	elseif s == "done" or s == "complete" then
		return "Done", "$success"
	end
	return "Idle", "$text.tertiary"
end

local function new_id(): string
	local function hex(n: number): string
		local out = {}
		for _ = 1, n do
			table.insert(out, string.format("%x", math.random(0, 15)))
		end
		return table.concat(out)
	end
	return `{hex(8)}-{hex(4)}-4{hex(3)}-{string.format("%x", math.random(8, 11))}{hex(3)}-{hex(12)}`
end

-- A new Bot: a chat of its own, named, pinned (so it stays a Bot) and opened.
local function create_bot()
	local id = new_id()
	local name = `Bot {#sessions.places:peek().pinned + 1}`
	sessions.create({ id = id, title = name }):after(function()
		local sidebar: any = boundary.load("@app/screens/shell/sidebar")
		if sidebar and sidebar.pin then
			sidebar.pin(id)
		end
		route.open_session(id)
	end)
end

local function open(c: any)
	route.open_session(c.id)
end

local function selected(c: any): () -> boolean
	return function(): boolean
		return route.current:get().session == c.id
	end
end

local function Heading(text: string, count: (() -> number)?): Node
	return ui.row {
		w_full = true,
		h = 28,
		px = "$space.2",
		mt = "$space.3",
		items = "center",
		gap = "$space.1",
		ui.text { text, text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "$text.secondary" },
		if count
			then ui.text {
				function(): string
					local n = count()
					return if n > 0 then tostring(n) else ""
				end,
				text_size = "$font.size.sm",
				color = "$text.tertiary",
				tabular = true,
			}
			else nil,
	}
end

local function BotRow(c: any): Node
	return ui.button {
		class = "session-row bot-row",
		label = c.title,
		w_full = true,
		h = 48,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		justify = "start",
		rounded = "$radius.md",
		bg = function(): string
			return if selected(c)() then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		on_click = function()
			open(c)
		end,
		ui.div {
			w = 32,
			h = 32,
			shrink = 0,
			rounded = "$radius.md",
			overflow = "hidden",
			ds.Avatar { seed = c.id, size = 32, label = c.title },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			gap = 1,
			ui.text { c.title, literal = true, truncate = true, text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
			ui.text { c.preview or "", literal = true, truncate = true, text_size = "$font.size.sm", color = "$text.tertiary" },
		},
		if c.status and NEEDS[c.status] then ui.div { w = 7, h = 7, shrink = 0, rounded = "$radius.full", bg = "$accent" } else nil,
	}
end

local function NeedRow(c: any): Node
	local what = if c.waiting and c.waiting[1] then c.waiting[1].label else c.title
	return ui.button {
		class = "session-row",
		label = what,
		w_full = true,
		h = 28,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		justify = "start",
		rounded = "$radius.md",
		bg = "#00000000",
		hover = { bg = "$hover" },
		on_click = function()
			open(c)
		end,
		ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = "$warning" },
		ui.text { what, literal = true, truncate = true, min_w = 0, text_size = "$font.size.sm", color = "$text.primary" },
	}
end

local function SessionRow(c: any): Node
	local line, tone = status_line(c)
	return ui.button {
		class = "session-row",
		label = c.title,
		w_full = true,
		h = 40,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		justify = "start",
		rounded = "$radius.md",
		bg = function(): string
			return if selected(c)() then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		on_click = function()
			open(c)
		end,
		ui.div {
			w = 6,
			h = 6,
			shrink = 0,
			rounded = "$radius.full",
			bg = if c.status == "working" then "$accent" else "#00000000",
			border = 1,
			border_color = "$text.disabled",
		},
		ui.column {
			grow = 1,
			min_w = 0,
			gap = 1,
			ui.text { c.title, literal = true, truncate = true, text_size = "$font.size.sm", color = "$text.primary" },
			ui.text { line, text_size = "$font.size.xs", color = tone },
		},
	}
end

local function Folder(f: any): Node
	return ui.column {
		w_full = true,
		gap = 1,
		ui.row {
			w_full = true,
			h = 24,
			px = "$space.2",
			mt = "$space.2",
			items = "center",
			gap = "$space.1",
			ui.text { f.name, literal = true, truncate = true, text_size = "$font.size.xs", weight = "$font.weight.medium", color = "$text.tertiary" },
			ui.text { tostring(#f.rows), text_size = "$font.size.xs", color = "$text.tertiary", tabular = true },
		},
		ui.each(f.rows, SessionRow),
	}
end

-- What a list says while it has nothing to show.
local function Empty(list: any, say: string): Node
	return ui.text {
		say,
		hidden = function(): boolean
			return #list:get() > 0
		end,
		w_full = true,
		px = "$space.2",
		py = "$space.1",
		text_size = "$font.size.sm",
		color = "$text.tertiary",
	}
end

-- The Bots' open Inbox cards (what only you can do), asked of the engine
-- for each Bot as the list changes and every 15 seconds.
local cards = ui.state("zeron_bot_cards", {} :: { any })
local function load_cards(list: { any })
	local got: { any } = {}
	local left = #list
	if left == 0 then
		cards:set({})
		return
	end
	for _, b in list do
		ui.call("bots.inbox", { botId = b.id, action = "list" }):after(function(reply: any)
			for _, item in (reply and reply.items) or {} do
				item.bot = b.id
				table.insert(got, item)
			end
			left -= 1
			if left == 0 then
				cards:set(got)
			end
		end, function()
			left -= 1
			if left == 0 then
				cards:set(got)
			end
		end)
	end
end

local function CardRow(item: any): Node
	return ui.button {
		class = "session-row inbox-card",
		label = item.title,
		w_full = true,
		h = 40,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		justify = "start",
		rounded = "$radius.md",
		bg = "#00000000",
		hover = { bg = "$hover" },
		on_click = function()
			route.open_session(item.bot)
		end,
		ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = if item.urgency == "blocking" then "$danger" else "$warning" },
		ui.column {
			grow = 1,
			min_w = 0,
			gap = 1,
			ui.text { item.title or "", literal = true, truncate = true, text_size = "$font.size.sm", color = "$text.primary" },
			ui.text { item.summary or item.kind or "", literal = true, truncate = true, text_size = "$font.size.xs", color = "$text.tertiary" },
		},
	}
end

local function Sidebar(props: { collapsed: any?, [string]: any }): Node
	local bot_list = ui.computed(bots)
	local need_list = ui.computed(needs)
	local folder_list = ui.computed(folders)
	ui.effect(function()
		load_cards(bot_list:get())
	end)
	ui.every(15000, function()
		load_cards(bot_list:peek())
	end)
	return ui.column {
		id = "sidebar",
		hook = "shell.sidebar",
		w = function(): number
			return if props.collapsed and props.collapsed:get() then 0 else W
		end,
		h_full = true,
		shrink = 0,
		overflow = "hidden",
		pt = props.top or 42,
		px = "$space.2",
		bg = "$background",
		border_sides = { 0, 1, 0, 0 },
		border_color = "$border",
		ui.row {
			w_full = true,
			h = 28,
			px = "$space.2",
			items = "center",
			ui.text { "Bots", grow = 1, text_size = "$font.size.md", weight = "$font.weight.semibold", color = "$text.primary" },
			ui.button {
				label = "New Bot",
				w = 24,
				h = 24,
				rounded = "$radius.sm",
				bg = "#00000000",
				hover = { bg = "$hover" },
				on_click = create_bot,
				ui.text { "+", text_size = "$font.size.md", color = "$text.secondary" },
			},
		},
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			overscroll = "contain",
			ui.column {
				w_full = true,
				gap = 1,
				pb = "$space.4",
				ui.each(bot_list, BotRow),
				Empty(bot_list, "No Bots yet. Press + to make one."),
				Heading("Needs you", function(): number
					return #need_list:get() + #cards:get()
				end),
				ui.each(cards, CardRow),
				ui.each(need_list, NeedRow),
				Empty(ui.computed(function(): { any }
					local all = table.clone(need_list:get())
					for _, c in cards:get() do
						table.insert(all, c)
					end
					return all
				end), "Nothing needs you."),
				Heading("Sessions"),
				ui.each(folder_list, Folder),
				Empty(folder_list, "No sessions yet."),
			},
		},
	}
end

return Sidebar
