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

-- A new Bot: a chat of its own, named, pinned (so it stays a Bot) and
-- opened; its goal, when it has one, its first message.
local function create_bot(name: string?, goal: string?)
	local id = new_id()
	local title = if name and name ~= "" then name else `Bot {#sessions.places:peek().pinned + 1}`
	sessions.create({ id = id, title = title }):after(function()
		local sidebar: any = boundary.load("@app/screens/shell/sidebar")
		if sidebar and sidebar.pin then
			sidebar.pin(id)
		end
		route.open_session(id)
		if goal and goal ~= "" then
			local store: any = require("@app/screens/chat/store")
			store.send({ session = id, text = goal, fresh = true } :: any)
		end
	end)
end

-- The New Bot dialog: what it's for, from a template or in your words.
local TEMPLATES = {
	{ name = "Code Reaper", goal = "Find code nothing uses any more in my repositories and open a small pull request for each removal, with the evidence nothing depends on it." },
	{ name = "Reviewer", goal = "Review the pull requests opened in my repositories and leave clear, actionable comments." },
	{ name = "Release manager", goal = "Track what is ready to ship, draft release notes and tell me what blocks a release." },
	{ name = "Custom Bot", goal = "" },
}
local new_open = ui.state("zeron_bot_new", false)
local new_goal = ui.state("zeron_bot_new_goal", "")
local new_name = ui.state("zeron_bot_new_name", "")

local function NewBotBody(): Node
	return ui.row {
		w_full = true,
		h = 420,
		ui.column {
			w = 240,
			h_full = true,
			shrink = 0,
			p = "$space.4",
			justify = "end",
			bg = "#C026A31F",
			ui.text { "Your custom Bot", text_size = "$font.size.lg", weight = "$font.weight.semibold", color = "$text.primary" },
			ui.text { "A Bot keeps its memory, files what needs you and starts sessions for its work.", text_size = "$font.size.sm", color = "$text.secondary" },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			h_full = true,
			p = "$space.4",
			gap = "$space.2",
			bg = "$background",
			ui.text { "What's your goal?", text_size = "$font.size.lg", weight = "$font.weight.semibold", color = "$text.primary" },
			ui.input {
				label = "Goal",
				placeholder = "Describe what this Bot should do",
				placeholder_color = "$text.tertiary",
				value = function(): string
					return new_goal:get()
				end,
				on_change = function(v: string)
					new_goal:set(v)
				end,
			},
			ui.each(TEMPLATES, function(t: any): Node
				return ui.button {
					label = t.name,
					w_full = true,
					h = 44,
					px = "$space.2",
					gap = "$space.2",
					items = "center",
					justify = "start",
					rounded = "$radius.md",
					bg = function(): string
						return if new_name:get() == t.name then "$zeron.selected.flat.bg" else "#00000000"
					end,
					hover = { bg = "$hover" },
					on_click = function()
						new_name:set(t.name)
						if t.goal ~= "" then
							new_goal:set(t.goal)
						end
					end,
					ui.div { w = 24, h = 24, shrink = 0, rounded = "$radius.md", overflow = "hidden", ds.Avatar { seed = t.name, size = 24, label = t.name } },
					ui.column {
						grow = 1,
						min_w = 0,
						ui.text { t.name, text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.primary" },
						ui.text { if t.goal ~= "" then t.goal else "Start from your own goal." , truncate = true, text_size = "$font.size.xs", color = "$text.tertiary" },
					},
				}
			end),
			ui.div { grow = 1 },
			ui.row {
				w_full = true,
				justify = "end",
				gap = "$space.2",
				ui.button {
					label = "Cancel",
					h = 32,
					px = "$space.3",
					rounded = "$radius.md",
					bg = "#00000000",
					hover = { bg = "$hover" },
					on_click = function()
						new_open:set(false)
					end,
					ui.text { "Cancel", text_size = "$font.size.sm", color = "$text.secondary" },
				},
				ui.button {
					label = "Continue",
					h = 32,
					px = "$space.3",
					rounded = "$radius.md",
					bg = "$accent",
					on_click = function()
						local name = new_name:peek()
						create_bot(if name == "Custom Bot" then nil else name, new_goal:peek())
						new_open:set(false)
						new_goal:set("")
						new_name:set("")
					end,
					ui.text { "Continue", text_size = "$font.size.sm", weight = "$font.weight.medium", color = "#FFFFFF" },
				},
			},
		},
	}
end

local function NewBot(): Node
	return ui.modal {
		open = new_open,
		label = "New Bot",
		w = 720,
		p = 0,
		gap = 0,
		rounded = "$radius.xl",
		build = NewBotBody,
	}
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

-- A card opened over the window: what it asks, and its answer.
local open_card = ui.state("zeron_bot_card", nil :: any)
local function answer_card(action: string, resolution: string?)
	local item = open_card:peek()
	if not item then
		return
	end
	open_card:set(nil)
	ui.call("bots.inbox", { botId = item.bot, action = action, id = item.id, resolution = resolution })
		:after(function()
			load_cards(bots())
		end)
end

local function CardButton(text: string, primary: boolean, act: () -> ()): Node
	return ui.button {
		label = text,
		h = 32,
		px = "$space.3",
		rounded = "$radius.md",
		bg = if primary then "$accent" else "#00000000",
		hover = { bg = if primary then "$accent" else "$hover" },
		on_click = act,
		ui.text { text, text_size = "$font.size.sm", weight = "$font.weight.medium", color = if primary then "#FFFFFF" else "$text.secondary" },
	}
end

local function CardBody(): Node
	local item = open_card:peek() or {}
	local approve = if item.kind == "approval" then (item.label or "Approve") else "Done"
	return ui.column {
		w_full = true,
		p = "$space.4",
		gap = "$space.2",
		bg = "$background",
		ui.row {
			gap = "$space.2",
			items = "center",
			ui.div { w = 8, h = 8, rounded = "$radius.full", bg = "$warning" },
			ui.text { "Needs you", text_size = "$font.size.xs", weight = "$font.weight.medium", color = "$warning" },
		},
		ui.text { item.title or "", literal = true, wrap = true, text_size = "$font.size.lg", weight = "$font.weight.semibold", color = "$text.primary" },
		ui.text { item.summary or "", literal = true, wrap = true, text_size = "$font.size.sm", color = "$text.secondary" },
		ui.text { item.details or "", literal = true, wrap = true, text_size = "$font.size.sm", color = "$text.tertiary" },
		ui.row {
			w_full = true,
			justify = "end",
			gap = "$space.2",
			pt = "$space.2",
			CardButton("Open Bot", false, function()
				open_card:set(nil)
				route.open_session(item.bot)
			end),
			CardButton("Dismiss", false, function()
				answer_card("close", "Dismissed")
			end),
			CardButton(approve, true, function()
				answer_card("resolve", approve)
			end),
		},
	}
end

local function CardOverlay(): Node
	return ui.modal {
		open = ui.computed(function(): boolean
			return open_card:get() ~= nil
		end),
		on_close = function()
			open_card:set(nil)
		end,
		label = "Needs you",
		w = 560,
		p = 0,
		gap = 0,
		rounded = "$radius.xl",
		build = CardBody,
	}
end

-- Review cards (a PR to review or merge) get their own section; the rest need you.
local function cards_where(review: boolean): () -> { any }
	return function(): { any }
		local out = {}
		for _, c in cards:get() do
			if (c.kind == "review") == review then
				table.insert(out, c)
			end
		end
		return out
	end
end
local need_cards = ui.computed(cards_where(false))
local review_cards = ui.computed(cards_where(true))

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
			open_card:set(item)
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
				on_click = function()
					new_open:set(true)
				end,
				ui.text { "+", text_size = "$font.size.md", color = "$text.secondary" },
			},
		},
		NewBot(),
		CardOverlay(),
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
					return #need_list:get() + #need_cards:get()
				end),
				ui.each(need_cards, CardRow),
				ui.each(need_list, NeedRow),
				Empty(ui.computed(function(): { any }
					local all = table.clone(need_list:get())
					for _, c in need_cards:get() do
						table.insert(all, c)
					end
					return all
				end), "Nothing needs you."),
				Heading("Review", function(): number
					return #review_cards:get()
				end),
				ui.each(review_cards, CardRow),
				Empty(review_cards, "Nothing to review."),
				Heading("Sessions"),
				ui.each(folder_list, Folder),
				Empty(folder_list, "No sessions yet."),
			},
		},
	}
end

return Sidebar
