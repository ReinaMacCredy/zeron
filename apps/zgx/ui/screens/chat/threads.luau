local ds = require("@ds")
local Icon = require("@ds/icon")
local Mark = require("@ds/mark")
local FadeText = require("@ds/fade_text")
local Menu = require("@ds/menu")
local ContextMenu = require("@ds/context_menu")
local IconButton = require("@ds/icon_button").IconButton
local locale = require("@ds/locale")
local status = require("@ds/status")
local sessions = require("@account/sessions")
local lanes = require("@app/screens/chat/lanes")
local state = require("@app/screens/panel/state")

local M = {}

local published = ui.shared("chat.threads", {} :: { [string]: any })
local shut = ui.state("chat_threads_shut", {} :: { [string]: boolean })

local GROUPS = { "review", "waiting", "working", "planned", "done" }
local GROUP_OF: { [string]: string } = {
	readyForReview = "review",
	waitingOnYou = "waiting",
	failed = "waiting",
	working = "working",
	planned = "planned",
	done = "done",
	skipped = "done",
	cancelled = "done",
}

function M.of(chat: string?): any
	return if chat then published:get()[chat] else nil
end

function M.rows(chat: string?): { any }
	local entry = M.of(chat)
	return if entry and entry.rows then entry.rows else {}
end

function M.group_of(row: any): string
	return GROUP_OF[row.state] or "working"
end

function M.working(chat: string?): number
	local n = 0
	for _, r in M.rows(chat) do
		if M.group_of(r) == "working" then
			n += 1
		end
	end
	return n
end

M.opener = nil :: ((session: string, thread: string?) -> ())?
M.toggler = nil :: ((session: string) -> ())?
M.showing = nil :: ((session: string) -> boolean)?

function M.open(session: string?, thread: string?)
	local opener = M.opener
	if session and opener then
		opener(session, thread)
	end
end

function M.toggle(session: string?)
	local toggler = M.toggler
	if session and toggler then
		toggler(session)
	end
end

function M.shown(session: string?): boolean
	local showing = M.showing
	return session ~= nil and showing ~= nil and showing(session)
end

local function slot_of(thread: string): string
	return `threads-{thread}`
end

function M.lane_of(tab: any): any
	return if tab.thread then { id = slot_of(tab.thread), kind = "side_chat", chat = tab.thread } else nil
end

function M.release(tab: any)
	if tab.thread then
		lanes.watch(slot_of(tab.thread), nil)
	end
end

M.host = {
	follow = function(chats: { string })
		pcall(ui.invoke, "threads.follow", { chats = chats })
	end,
	call = function(name: string, args: any): any
		local ok, p = pcall(ui.call, name, args)
		return if ok then p else nil
	end,
}

local mounted: { [string]: number } = {}
local function follow()
	local chats: { string } = {}
	for id in mounted do
		table.insert(chats, id)
	end
	table.sort(chats)
	M.host.follow(chats)
end

local function mount(chat: string)
	mounted[chat] = (mounted[chat] or 0) + 1
	if mounted[chat] == 1 then
		follow()
	end
	ui.on_cleanup(function()
		local n = (mounted[chat] or 1) - 1
		mounted[chat] = if n > 0 then n else nil
		if n <= 0 then
			follow()
		end
	end)
end

local function failed(err: any)
	local why = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
	ds.toast.show { title = locale.s("chat.threads.failed", { err = why }), tone = "danger" }
end

local function settled(p: any)
	if p then
		p:after(function() end, failed)
	else
		failed("failed")
	end
end

local function call(name: string, args: any)
	settled(M.host.call(name, args))
end

function M.state_icon(st: string, size: number): Node
	if st == "working" then
		return status.StatusIcon { kind = "working", size = size }
	elseif st == "readyForReview" then
		return status.StatusIcon { kind = "done", size = size }
	elseif st == "failed" then
		return status.StatusIcon { kind = "failed", size = size }
	elseif st == "waitingOnYou" then
		return ui.div { shrink = 0, color = "$warning", Icon { name = "clock-circle", size = size } }
	elseif st == "cancelled" then
		return ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "close-circle", size = size } }
	elseif st == "planned" then
		return ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "status-working", size = size } }
	end
	return ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "status-done", size = size } }
end

type Ctx = { session: string, key: string, tab: string }

local function open_row(ctx: Ctx, row: any)
	if row.kind == "subagent" then
		local ref = string.sub(row.key, #"subagent:" + 1)
		for _, sub in lanes.subagents() do
			if sub.ref == ref then
				lanes.open_subagent(ctx.session, sub)
				return
			end
		end
		lanes.open_subagent(ctx.session, { ref = ref, title = row.title, status = if row.state == "working" then "running" else "done" })
		return
	end
	if not row.chat then
		return
	end
	if row.kind == "peer" then
		lanes.open_thread(row.chat)
		return
	end
	state.patch(ctx.key, ctx.tab, { thread = row.chat })
end

local function entries(ctx: Ctx, row: any): { any }
	local out: { any } = {}
	local function add(list: { any }, label: string, icon: string, act: () -> ())
		table.insert(list, { label = label, icon = icon, on_select = act })
	end
	if row.chat or row.kind == "subagent" then
		add(out, locale.s("chat.threads.menu.open"), "chat-round-line", function()
			open_row(ctx, row)
		end)
	end
	if row.read_only then
		return out
	end
	if row.chat then
		if row.state == "working" or row.state == "waitingOnYou" then
			add(out, locale.s("chat.threads.menu.stop"), "stop", function()
				settled(sessions.stop(row.chat))
			end)
		end
		add(out, locale.s("common.archive"), "lucide-archive", function()
			sessions.archive(row.chat)
		end)
	end
	local marks: { any } = {}
	if row.step then
		local closed = row.state == "done" or row.state == "skipped"
		if row.state ~= "done" then
			add(marks, locale.s("chat.threads.menu.done"), "status-done", function()
				call("threads.mark", { step = row.step, state = "done" })
			end)
		end
		if not closed then
			add(marks, locale.s("chat.threads.menu.skip"), "arrow-right", function()
				call("threads.mark", { step = row.step, state = "skipped" })
			end)
		else
			add(marks, locale.s("chat.threads.menu.reopen"), "lucide-undo-2", function()
				call("threads.mark", { step = row.step, state = "open" })
			end)
		end
		if row.chat and row.state ~= "working" then
			add(marks, locale.s("chat.threads.menu.retry"), "restart", function()
				call("threads.retry", { step = row.step })
			end)
		end
	elseif row.chat then
		if row.state ~= "done" then
			add(marks, locale.s("chat.threads.menu.done"), "status-done", function()
				settled(sessions.settle(row.chat, true))
			end)
		else
			add(marks, locale.s("chat.threads.menu.reopen"), "lucide-undo-2", function()
				settled(sessions.settle(row.chat, false))
			end)
		end
	end
	if #marks > 0 and #out > 0 then
		table.insert(out, { separator = true })
	end
	for _, e in marks do
		table.insert(out, e)
	end
	return out
end

local function dom_id(key: string): string
	return (string.gsub(key, "[^%w%-]", "-"))
end

local function tag_of(row: any): string?
	if row.kind == "subagent" then
		return `{locale.s("chat.threads.subagent")} · {locale.s("chat.threads.read_only")}`
	elseif row.kind == "peer" then
		return `{locale.s("chat.threads.peer")} · {locale.s("chat.threads.read_only")}`
	elseif row.state == "skipped" then
		return locale.s("chat.threads.skipped")
	elseif row.state == "cancelled" then
		return locale.s("chat.threads.cancelled")
	end
	return nil
end

local function mark_of(row: any, size: number): Node?
	if not row.mark then
		return nil
	end
	return ui.div { shrink = 0, tooltip = row.model, Mark { name = row.mark :: any, size = size, bare = true } }
end

local function more_menu(ctx: Ctx, row: any, list: { any }, open: any): Node
	local anchor = IconButton {
		icon = "more-horizontal",
		label = locale.s("chat.threads.more", { title = row.title }),
		size = "sm",
		tone = "quiet",
		on_click = function()
			open:set(not open:peek())
		end,
	}
	local panel: { [any]: any } = { open = open, anchor = anchor, placement = { side = "bottom", align = "end", offset = 4 }, w = 200 }
	local group: { any } = {}
	local function flush()
		if #group > 0 then
			if #panel > 0 then
				table.insert(panel, Menu.Rule {})
			end
			table.insert(panel, Menu.Group(group))
			group = {}
		end
	end
	for _, e in list do
		if e.separator then
			flush()
		else
			table.insert(group, Menu.Row {
				icon = e.icon,
				label = e.label,
				role = "menuitem",
				on_click = function()
					open:set(false)
					e.on_select()
				end,
			})
		end
	end
	flush()
	return ui.menu(Menu.panel(panel))
end

local function RowView(ctx: Ctx, row: any): Node
	local hovered = ui.use_state(false)
	local menu_open = ui.use_state(false)
	local list = entries(ctx, row)
	local tag = tag_of(row)
	local opens = row.chat ~= nil or row.kind == "subagent"
	local title: { any } = { items = "baseline", gap = ui.raw(6, "the mockup's step number"), min_w = 0 }
	if row.number then
		table.insert(title, ui.text { tostring(row.number), color = "$text.tertiary", tabular = true, shrink = 0, wrap = false })
	end
	table.insert(title, FadeText {
		row.title,
		literal = true,
		min_w = 0,
		shrink = 1,
		color = if row.state == "planned" then "$text.secondary" else "$text.primary",
	})
	local lines: { any } = { grow = 1, min_w = 0, gap = ui.raw(1, "the mockup's row"), ui.row(title) }
	if row.line and row.line ~= "" then
		table.insert(lines, ui.text { row.line, literal = true, wrap = true, lines = 2, text_size = ui.raw(12, "the mockup's row"), color = "$text.secondary" })
	end
	if tag then
		table.insert(lines, ui.text { tag, literal = true, wrap = false, truncate = true, text_size = ui.raw(11.5, "the mockup's row"), color = "$text.tertiary" })
	end
	local corner: { any } = { shrink = 0, items = "end", gap = ui.raw(6, "the mockup's row") }
	if row.state ~= "planned" and row.updated then
		table.insert(corner, ui.text {
			function(): string
				return status.ago(status.since(row.updated))
			end,
			text_size = ui.raw(11.5, "the mockup's row"),
			color = "$text.tertiary",
			wrap = false,
			opacity = function(): number
				return if hovered:get() or menu_open:get() then 0 else 1
			end,
		})
	end
	local mark = mark_of(row, 14)
	if mark then
		table.insert(corner, mark)
	end
	return ContextMenu {
		ring = "$radius.lg",
		w_full = true,
		items = function(): { any }?
			return if #list > 0 then list else nil
		end,
		ui.row {
			id = `threads-row-{dom_id(row.key)}`,
			class = `threads-row threads-row-{row.state}`,
			role = if opens then "button" else nil,
			label = row.title,
			relative = true,
			w_full = true,
			p = ui.raw(8, "the mockup's row"),
			gap = ui.raw(10, "the mockup's row"),
			items = "start",
			rounded = ui.raw(9, "the mockup's row"),
			cursor = if opens then "pointer" else nil,
			bg = function(): string
				return if hovered:get() or menu_open:get() then "$hover" else "#00000000"
			end,
			on_hover = function(inside: boolean)
				hovered:set(inside)
			end,
			on_click = if opens
				then function()
					open_row(ctx, row)
				end
				else nil,
			ui.div { mt = ui.raw(1, "the mockup's row"), M.state_icon(row.state, 18) },
			ui.column(lines),
			ui.column(corner),
			if #list > 0
				then ui.div {
					absolute = true,
					right = ui.raw(6, "the mockup's row"),
					top = ui.raw(6, "the mockup's row"),
					opacity = function(): number
						return if hovered:get() or menu_open:get() then 1 else 0
					end,
					more_menu(ctx, row, list, menu_open),
				}
				else nil,
		},
	}
end

local function items_of(chat: string, closed: { [string]: boolean }): { any }
	local by: { [string]: { any } } = {}
	for _, r in M.rows(chat) do
		local g = M.group_of(r)
		by[g] = by[g] or {}
		table.insert(by[g], r)
	end
	local out: { any } = {}
	for _, g in GROUPS do
		local list = by[g]
		if list then
			local is_shut = closed[`{chat}:{g}`] == true
			table.insert(out, { kind = "group", group = g, n = #list, shut = is_shut, key = `group:{g}:{#list}:{is_shut}` })
			if not is_shut then
				for _, r in list do
					local said = `{r.state}|{r.title}|{r.line or ""}|{r.updated or 0}|{r.number or ""}|{r.mark or ""}|{r.model or ""}|{r.step or ""}|{r.chat or ""}`
					table.insert(out, { kind = "row", row = r, key = `{r.key}|{said}` })
				end
			end
		end
	end
	return out
end

local function GroupHead(chat: string, item: any): Node
	local key = `{chat}:{item.group}`
	return ui.button {
		id = `threads-group-{item.group}`,
		class = "threads-group",
		label = `{locale.s(`chat.threads.group.{item.group}`)} · {item.n}`,
		expanded = not item.shut,
		bg = "#00000000",
		px = ui.raw(6, "the mockup's group head"),
		pt = ui.raw(12, "the mockup's group head"),
		pb = ui.raw(6, "the mockup's group head"),
		gap = ui.raw(5, "the mockup's group head"),
		items = "center",
		text_size = ui.raw(12.5, "the mockup's group head"),
		color = "$text.secondary",
		hover = { color = "$text.primary" },
		on_click = function()
			local now = table.clone(shut:peek())
			now[key] = if item.shut then nil else true
			shut:set(now)
		end,
		ui.text { `{locale.s(`chat.threads.group.{item.group}`)} · {item.n}`, literal = true, wrap = false },
		ui.div { rotate = if item.shut then -90 else 0, Icon { name = "lucide-chevron-down", size = 12 } },
	}
end

local function List(ctx: Ctx): Node
	local chat = ctx.session
	local items = ui.computed(function(): { any }
		return items_of(chat, shut:get())
	end)
	return ui.column {
		id = "threads-list",
		w_full = true,
		grow = 1,
		min_h = 0,
		ui.row {
			w_full = true,
			h = ui.raw(36, "the mockup's list head"),
			shrink = 0,
			px = ui.raw(14, "the mockup's list head"),
			items = "center",
			border = 1,
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border.subtle",
			ui.text {
				function(): string
					return locale.s("chat.threads.count", { n = #M.rows(chat) })
				end,
				id = "threads-count",
				text_size = ui.raw(13, "the mockup's list head"),
				color = "$text.secondary",
				wrap = false,
			},
		},
		ui.presence(function(): string
			local entry = M.of(chat)
			return if #M.rows(chat) == 0 and not (entry and entry.plan) then "empty" else "list"
		end, function(which: string): Node
			if which == "empty" then
				return ui.text {
					locale.t("chat.threads.empty"),
					id = "threads-empty",
					wrap = true,
					px = ui.raw(16, "the mockup's empty list"),
					py = ui.raw(12, "the mockup's empty list"),
					text_size = ui.raw(12, "the mockup's empty list"),
					color = "$text.tertiary",
				}
			end
			return ui.scroll {
				w_full = true,
				grow = 1,
				min_h = 0,
				ui.column {
					w_full = true,
					px = ui.raw(8, "the mockup's list"),
					pt = ui.raw(6, "the mockup's list"),
					pb = ui.raw(20, "the mockup's list"),
					ui.each(items, function(item: any): Node
						if item.kind == "group" then
							return GroupHead(chat, item)
						end
						return RowView(ctx, item.row)
					end, {
						key = function(item: any): string
							return item.key
						end,
						w_full = true,
					}),
				},
			}
		end, { w_full = true, grow = 1, min_h = 0 }),
	}
end

local ThreadView = ui.component(function(props: { ctx: Ctx, thread: string }): Node
	local ctx, thread = props.ctx, props.thread
	local Lane = (require :: any)("@app/screens/chat/lane_tab")
	local slot = slot_of(thread)
	ui.on_cleanup(function()
		lanes.watch(slot, nil)
	end)
	local function row(): any
		for _, r in M.rows(ctx.session) do
			if r.chat == thread then
				return r
			end
		end
		return nil
	end
	local function title(): string
		local r = row()
		if r then
			return r.title
		end
		local c = lanes.child_row(thread)
		return if c and c.title then c.title else ""
	end
	local function st(): string
		local r = row()
		return if r then r.state else ""
	end
	return ui.column {
		id = "threads-thread",
		w_full = true,
		grow = 1,
		min_h = 0,
		ui.row {
			w_full = true,
			h = ui.raw(36, "the mockup's list head"),
			shrink = 0,
			px = ui.raw(6, "the mockup's thread head"),
			gap = "$space.1",
			items = "center",
			border = 1,
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border.subtle",
			ui.button {
				id = "threads-back",
				label = locale.t("chat.threads.back"),
				bg = "#00000000",
				h = 24,
				pl = ui.raw(2, "the mockup's back"),
				pr = ui.raw(6, "the mockup's back"),
				gap = ui.raw(4, "the mockup's back"),
				items = "center",
				rounded = "$radius.sm",
				text_size = ui.raw(12.5, "the mockup's back"),
				color = "$text.secondary",
				hover = { bg = "$hover", color = "$text.primary" },
				on_click = function()
					state.patch(ctx.key, ctx.tab, { thread = false })
				end,
				Icon { name = "alt-arrow-left", size = 14 },
				ui.text { locale.t("panel.tab.threads"), wrap = false },
			},
		},
		ui.row {
			w_full = true,
			shrink = 0,
			px = ui.raw(16, "the mockup's thread title"),
			pt = ui.raw(10, "the mockup's thread title"),
			pb = ui.raw(4, "the mockup's thread title"),
			gap = ui.raw(8, "the mockup's thread title"),
			items = "center",
			ui.presence(st, function(s: string): Node
				return if s == "" then ui.div {} else M.state_icon(s, 18)
			end, { shrink = 0 }),
			FadeText { title, id = "threads-thread-title", literal = true, min_w = 0, grow = 1, weight = "$font.weight.medium", text_size = ui.raw(14, "the mockup's thread title") },
			ui.presence(function(): string
				local r = row()
				return if r and r.mark then `{r.mark}\0{r.model or ""}` else ""
			end, function(which: string): Node
				local r = row()
				if which == "" or not r then
					return ui.div {}
				end
				return ui.row {
					gap = ui.raw(5, "the mockup's thread sub line"),
					items = "center",
					shrink = 0,
					mark_of(r, 12),
					if r.model then ui.text { r.model, literal = true, wrap = false, text_size = ui.raw(12, "the mockup's thread sub line"), color = "$text.secondary" } else nil,
				}
			end, { shrink = 0 }),
		},
		ui.show(function(): boolean
			return st() == "waitingOnYou"
		end, ui.row {
			id = "threads-waiting-banner",
			mx = ui.raw(16, "the mockup's banner"),
			my = ui.raw(6, "the mockup's banner"),
			px = ui.raw(8, "the mockup's banner"),
			py = ui.raw(4, "the mockup's banner"),
			gap = ui.raw(6, "the mockup's banner"),
			items = "center",
			shrink = 0,
			rounded = ui.raw(8, "the mockup's banner"),
			border = 1,
			border_color = "$warning",
			color = "$warning",
			Icon { name = "clock-circle", size = 12 },
			ui.text {
				function(): string
					local r = row()
					local line = if r and r.line then r.line else ""
					return if line ~= "" then locale.s("chat.threads.waiting", { line = line }) else locale.s("chat.threads.group.waiting")
				end,
				wrap = true,
				min_w = 0,
				text_size = ui.raw(12, "the mockup's banner"),
			},
		}),
		ui.column {
			w_full = true,
			grow = 1,
			min_h = 0,
			Lane.Tab { tab = { id = slot, kind = "side_chat", chat = thread, title = title() }, panel_key = ctx.key },
		},
	}
end)

M.Tab = ui.component(function(props: { tab: any, panel_key: string }): Node
	local ctx: Ctx = { session = props.tab.chat or "", key = props.panel_key, tab = props.tab.id }
	mount(ctx.session)
	return ui.presence(function(): string
		for _, t in state.of(ctx.key).tabs do
			if t.id == ctx.tab then
				return t.thread or ""
			end
		end
		return ""
	end, function(thread: string): Node
		if thread == "" then
			return List(ctx)
		end
		return ThreadView { ctx = ctx, thread = thread }
	end, { w_full = true, grow = 1, min_h = 0 })
end)

return M
