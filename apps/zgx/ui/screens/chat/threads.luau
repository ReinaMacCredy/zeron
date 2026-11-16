local ds = require("@ds")
local Icon = require("@ds/icon")
local Mark = require("@ds/mark")
local FadeText = require("@ds/fade_text")
local Menu = require("@ds/menu")
local ContextMenu = require("@ds/context_menu")
local IconButton = require("@ds/icon_button").IconButton
local ProgressBar = require("@ds/progress").ProgressBar
local Glass = require("@ds/glass")
local Transcript = require("@ds/transcript_threads")
local locale = require("@ds/locale")
local status = require("@ds/status")
local motion = require("@ds/motion")
local sessions = require("@account/sessions")
local lanes = require("@app/screens/chat/lanes")
local state = require("@app/screens/panel/state")

local M = {}

local published = ui.shared("chat.threads", {} :: { [string]: any })
local waiting = ui.shared("chat.waiting", {} :: { any })
local shut = ui.state("chat_threads_shut", {} :: { [string]: boolean })
local overview_open = ui.state("chat_threads_overview", {} :: { [string]: boolean })
local drafts = ui.state("chat_threads_drafts", {} :: { [string]: any })
local routes = ui.state("chat_threads_routes", nil :: any)

local ROLES = { "worker", "design", "validator", "research", "general" }

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

function M.question(chat: string?): string?
	if not chat then
		return nil
	end
	for _, it in waiting:get() do
		if it.chat == chat and it.ask then
			local q, a = it.ask.question, it.ask.approval
			if q and q.text and q.text ~= "" then
				return q.text
			elseif a then
				return a.command or a.detail or a.title
			end
		end
	end
	return nil
end

local function steps_of(chat: string?): ({ any }, any)
	local entry = M.of(chat)
	local plan = entry and entry.plan
	if not plan then
		return {}, nil
	end
	local by: { [string]: any } = {}
	for _, r in entry.rows or {} do
		if r.step then
			by[r.step] = r
		end
	end
	local out: { any } = {}
	for i, id in plan.order or {} do
		table.insert(out, { id = id, at = i, row = by[id] })
	end
	return out, plan
end

local function build_card(chat: string?): any
	local steps, plan = steps_of(chat)
	if not plan then
		return nil
	end
	local lines: { any } = {}
	local by_step: { [string]: any } = {}
	local by_thread: { [string]: string } = {}
	for _, s in steps do
		local r = s.row
		if r then
			local line = {
				id = s.id,
				number = r.number or s.at,
				title = r.step_title or r.title,
				mark = r.mark,
				model = r.model,
				harness = r.harness,
				state = r.state,
				chat = r.chat,
				line = r.line,
				attempt = r.attempt,
				earlier = r.earlier,
				background = r.background,
				quiet_since = r.quiet_since,
				activity = r.activity,
				activity_at = r.activity_at,
				turn_started_at = r.turn_started_at,
				turn = r.turn,
			}
			table.insert(lines, line)
			by_step[s.id] = line
			if r.chat then
				by_thread[r.chat] = s.id
			end
			for _, e in (if type(r.earlier) == "table" then r.earlier else {}) do
				if type(e) == "table" and e.id then
					by_thread[e.id] = s.id
				end
			end
		end
	end
	local progress = plan.progress or {}
	return {
		overview = plan.overview or "",
		pending = plan.pending == true,
		count = #steps,
		steps = lines,
		by_step = by_step,
		threads = by_thread,
		done = progress.done or 0,
		total = progress.total or #steps,
	}
end

local cards: { [string]: { entry: any, card: any } } = {}

function M.card(chat: string?): any
	local entry = M.of(chat)
	local hit = if chat then cards[chat] else nil
	if hit and hit.entry == entry then
		return hit.card
	end
	local card = build_card(chat)
	if chat then
		cards[chat] = if entry then { entry = entry, card = card } else nil
	end
	return card
end

function M.plan_running(chat: string?): boolean
	local c = M.card(chat)
	if not c or c.pending or c.count == 0 then
		return false
	end
	if c.done < c.total then
		return true
	end
	for _, s in c.steps do
		if s.state == "working" then
			return true
		end
	end
	return false
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

function M.approve(chat: string?)
	if chat then
		call("threads.approve", { chat = chat })
	end
end

local function trim(s: string?): string
	return string.match(s or "", "^%s*(.-)%s*$") or ""
end

local function pick_of(t: any): any
	return if type(t) == "table" and t.harness then { harness = t.harness, model = t.model, effort = t.effort } else nil
end

local function same_pick(a: any, b: any): boolean
	if a == nil or b == nil then
		return a == b
	end
	return a.harness == b.harness and a.model == b.model and a.effort == b.effort
end

local function live_row(chat: string, id: string?): any
	if id then
		for _, r in M.rows(chat) do
			if r.step == id then
				return r
			end
		end
	end
	return nil
end

local function new_draft(chat: string): any?
	local steps, plan = steps_of(chat)
	if not plan then
		return nil
	end
	local fixed = 0
	for _, s in steps do
		if not s.row or s.row.locked then
			fixed = s.at
		end
	end
	local list: { any } = {}
	local base: { [string]: any } = {}
	for _, s in steps do
		local r = s.row
		if r then
			base[s.id] = { title = r.step_title or r.title, spec = r.spec or "", role = r.role, target = pick_of(r.target) }
			table.insert(list, {
				key = s.id,
				id = s.id,
				title = r.step_title or r.title,
				spec = r.spec or "",
				role = r.role,
				target = pick_of(r.target),
				locked = r.locked == true,
				fixed = s.at <= fixed,
				state = r.state,
			})
		end
	end
	return { overview = plan.overview or "", base_overview = plan.overview or "", steps = list, base = base, order = table.clone(plan.order or {}), fixed = fixed, removed = {}, confirm = false, saving = false, added = 0 }
end

function M.draft(chat: string): any?
	return drafts:get()[chat]
end

local function change(chat: string, f: (d: any) -> ())
	local d = drafts:peek()[chat]
	if not d then
		return
	end
	local next_d = table.clone(d)
	next_d.steps = table.clone(d.steps)
	next_d.removed = table.clone(d.removed)
	f(next_d)
	local all = table.clone(drafts:peek())
	all[chat] = next_d
	drafts:set(all)
end

local function change_step(chat: string, key: string, f: (s: any) -> ())
	change(chat, function(d)
		for i, s in d.steps do
			if s.key == key then
				local next_s = table.clone(s)
				f(next_s)
				d.steps[i] = next_s
			end
		end
	end)
end

local function tab_of(chat: string): (string?, string?)
	local k = state.key(chat, nil)
	if not k then
		return nil, nil
	end
	for _, t in state.of(k).tabs do
		if t.kind == "threads" and t.chat == chat then
			return k, t.id
		end
	end
	return k, nil
end

function M.edit(chat: string?)
	if not chat or not M.of(chat) or not M.of(chat).plan then
		return
	end
	M.open(chat)
	local d = drafts:peek()[chat]
	if not d or (M.changes(d)) == 0 then
		local all = table.clone(drafts:peek())
		all[chat] = new_draft(chat)
		drafts:set(all)
	end
	local k, id = tab_of(chat)
	if k and id then
		state.patch(k, id, { editing = true, thread = false })
	end
end

local function close_editor(chat: string)
	local all = table.clone(drafts:peek())
	all[chat] = nil
	drafts:set(all)
	local k, id = tab_of(chat)
	if k and id then
		state.patch(k, id, { editing = false })
	end
end

local function step_band(s: any): string
	return if s.id == nil then "new" elseif s.fixed then "fixed" else "tail"
end

function M.changes(d: any): (number, boolean)
	local n = 0
	if trim(d.overview) ~= trim(d.base_overview) then
		n += 1
	end
	n += #d.removed
	local bad = false
	local was: { string } = {}
	for _, id in d.order do
		if d.base[id] and not table.find(d.removed, id) then
			table.insert(was, id)
		end
	end
	local now: { string } = {}
	for _, s in d.steps do
		local title = trim(s.title)
		if s.id then
			table.insert(now, s.id)
			local b = d.base[s.id]
			if title == "" then
				bad = true
			end
			if title ~= "" and title ~= b.title then
				n += 1
			end
			if trim(s.spec) ~= trim(b.spec) then
				n += 1
			end
			if s.role ~= b.role then
				n += 1
			end
			if not same_pick(s.target, b.target) then
				n += 1
			end
		elseif title ~= "" then
			n += 1
		end
	end
	if table.concat(was, "\0") ~= table.concat(now, "\0") then
		n += 1
	end
	return n, bad
end

function M.edit_args(chat: string, d: any): any
	local steps: { any } = {}
	local tail: { string } = {}
	for _, s in d.steps do
		local title, spec = trim(s.title), trim(s.spec)
		if s.id then
			local b = d.base[s.id]
			local moved = not same_pick(s.target, b.target)
			if title ~= b.title or spec ~= trim(b.spec) or s.role ~= b.role or moved then
				table.insert(steps, { id = s.id, title = title, spec = spec, role = s.role, target = if moved then s.target or false else nil })
			end
			if not s.fixed then
				table.insert(tail, s.id)
			end
		elseif title ~= "" then
			table.insert(steps, { title = title, spec = spec, role = s.role, target = s.target })
		end
	end
	local order: { string } = {}
	for i, id in d.order do
		if i <= d.fixed and not table.find(d.removed, id) then
			table.insert(order, id)
		end
	end
	for _, id in tail do
		table.insert(order, id)
	end
	local overview = trim(d.overview)
	return {
		chat = chat,
		overview = if overview ~= trim(d.base_overview) then overview else nil,
		steps = steps,
		remove = table.clone(d.removed),
		order = order,
	}
end

local function save(chat: string)
	local d = drafts:peek()[chat]
	if not d or d.saving then
		return
	end
	local n, bad = M.changes(d)
	if n == 0 or bad then
		return
	end
	change(chat, function(x)
		x.saving = true
	end)
	local function refused(err: any)
		change(chat, function(x)
			x.saving = false
		end)
		local why = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
		ds.toast.show { title = locale.s("chat.threads.edit.failed", { err = why }), tone = "danger" }
	end
	local p = M.host.call("threads.edit", M.edit_args(chat, d))
	if not p then
		refused("failed")
		return
	end
	p:after(function()
		close_editor(chat)
	end, refused)
end

local function leave(chat: string)
	local d = drafts:peek()[chat]
	if d and (M.changes(d)) > 0 then
		change(chat, function(x)
			x.confirm = true
		end)
		return
	end
	close_editor(chat)
end

local function move_step(chat: string, key: string, to_key: string)
	change(chat, function(d)
		local from, to = nil, nil
		for i, s in d.steps do
			if s.key == key then
				from = i
			elseif s.key == to_key then
				to = i
			end
		end
		if not from or not to or step_band(d.steps[from]) ~= step_band(d.steps[to]) or d.steps[from].fixed then
			return
		end
		local s = table.remove(d.steps, from)
		table.insert(d.steps, to, s)
	end)
end

local function nudge(chat: string, key: string, by: number)
	local d = drafts:peek()[chat]
	if not d then
		return
	end
	for i, s in d.steps do
		if s.key == key then
			local other = d.steps[i + by]
			if other then
				move_step(chat, key, other.key)
			end
			return
		end
	end
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
	if row.chat then
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
	if row.kind == "peer" then
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
	return ui.div {
		shrink = 0,
		tooltip = row.model,
		Mark { name = row.mark :: any, size = size, bare = true },
	}
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

local MOVE_MS = 220
local EASE = "cubicBezier(.2,.8,.2,1)"

function M.key_of(row: any): string
	return if row.step then `step:{row.step}` else row.key
end

local function line_of(r: any): string
	if r.state == "waitingOnYou" then
		return M.question(r.chat) or r.line or ""
	end
	local working = Transcript.working_line(r)
	if working then
		return working
	end
	local facts = if r.state ~= "working" and r.state ~= "planned" then Transcript.facts(r.turn) else nil
	local text = r.line or ""
	if facts then
		return if text ~= "" then `{facts} · {text}` else facts
	end
	return text
end

local function mark_kind(r: any): string
	return if Transcript.quiet(r) then "quiet" else r.state
end

local function RowView(ctx: Ctx, now: () -> any, here: boolean): Node
	local hovered = ui.use_state(false)
	local menu_open = ui.use_state(false)
	local r0 = now()
	local opens = r0.chat ~= nil
	local reduced = motion.reduced_now()
	local fade_in = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, delay = 60, duration = 180 }
	local fade_out = { opacity = 0, duration = motion.FADE }
	local function list(): { any }
		return entries(ctx, now())
	end
	local title: { any } = { items = "baseline", gap = ui.raw(6, "the mockup's step number"), min_w = 0 }
	if r0.number then
		table.insert(title, ui.text {
			function(): string
				return tostring(now().number or "")
			end,
			color = "$text.tertiary",
			tabular = true,
			shrink = 0,
			wrap = false,
		})
	end
	table.insert(title, FadeText {
		function(): string
			return now().title or ""
		end,
		literal = true,
		min_w = 0,
		shrink = 1,
		color = function(): string
			local st = now().state
			return if st == "planned" then "$text.secondary" elseif st == "failed" then "$danger" else "$text.primary"
		end,
	})
	table.insert(title, ui.div {
		self = "center",
		shrink = 0,
		Transcript.Chips {
			chips = function(): { any }
				return Transcript.chips(now())
			end,
		},
	})
	local lines: { any } = {
		grow = 1,
		min_w = 0,
		gap = ui.raw(1, "the mockup's row"),
		ui.row(title),
		ui.presence(function(): string?
			local l = line_of(now())
			return if l ~= "" then l else nil
		end, function(l: string): Node
			return ui.text { l, literal = true, wrap = true, lines = 2, text_size = ui.raw(12, "the mockup's row"), color = "$text.secondary" }
		end, { w_full = true, min_w = 0, enter = fade_in, exit = fade_out }),
		ui.presence(function(): string?
			return tag_of(now())
		end, function(tag: string): Node
			return ui.text { tag, literal = true, wrap = false, truncate = true, text_size = ui.raw(11.5, "the mockup's row"), color = "$text.tertiary" }
		end, { w_full = true, min_w = 0 }),
	}
	local corner: { any } = {
		shrink = 0,
		items = "end",
		gap = ui.raw(6, "the mockup's row"),
		ui.show(
			function(): boolean
				local r = now()
				return r.state ~= "planned" and r.updated ~= nil
			end,
			ui.text {
				function(): string
					local at = now().updated
					return if at then status.ago(status.since(at)) else ""
				end,
				text_size = ui.raw(11.5, "the mockup's row"),
				color = "$text.tertiary",
				wrap = false,
				opacity = function(): number
					return if hovered:get() or menu_open:get() then 0 else 1
				end,
			}
		),
		ui.presence(function(): string?
			local r = now()
			return if r.mark then `{r.mark}\0{r.model or ""}` else nil
		end, function(): Node
			return mark_of(now(), 14) or ui.div {}
		end, { shrink = 0 }),
	}
	return ContextMenu {
		ring = "$radius.lg",
		w_full = true,
		items = function(): { any }?
			local l = list()
			return if #l > 0 then l else nil
		end,
		ui.row {
			id = if here then `threads-row-{dom_id(M.key_of(r0))}` else nil,
			class = "threads-row",
			role = if opens then "button" else nil,
			label = function(): string
				return now().title or ""
			end,
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
					open_row(ctx, now())
				end
				else nil,
			ui.presence(function(): string
				return mark_kind(now())
			end, function(kind: string): Node
				local st = now().state
				return ui.div {
					class = `threads-row-{st}`,
					mt = ui.raw(1, "the mockup's row"),
					if kind == "quiet" then status.Spinner { size = 18, ink = "$zeron.tone.warning.ink" } else M.state_icon(st, 18),
				}
			end, {
				shrink = 0,
				enter = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { from = 0, to = 1, duration = 160 }, scale = { 0.6, 1 }, duration = MOVE_MS, ease = EASE },
				exit = { opacity = 0, duration = if reduced then motion.FADE else 160 },
			}),
			ui.column(lines),
			ui.column(corner),
			ui.div {
				absolute = true,
				right = ui.raw(6, "the mockup's row"),
				top = ui.raw(6, "the mockup's row"),
				opacity = function(): number
					return if hovered:get() or menu_open:get() then 1 else 0
				end,
				ui.presence(function(): string?
					local l = list()
					if #l == 0 then
						return nil
					end
					local r = now()
					return `{r.state}\0{r.chat or ""}`
				end, function(): Node
					return more_menu(ctx, now(), list(), menu_open)
				end),
			},
		},
	}
end

local arrivals = 0
local function grouped(chat: string, mine: { [string]: { g: string, at: number } }): { [string]: { any } }
	local by: { [string]: { any } } = {}
	local seen: { [string]: boolean } = {}
	for i, r in M.rows(chat) do
		local g, k = M.group_of(r), M.key_of(r)
		seen[k] = true
		local p = mine[k]
		if not p or p.g ~= g then
			local at = 0
			if p then
				arrivals += 1
				at = arrivals
			end
			p = { g = g, at = at }
			mine[k] = p
		end
		by[g] = by[g] or {}
		table.insert(by[g], { r = r, at = p.at, i = i })
	end
	for k in mine do
		if not seen[k] then
			mine[k] = nil
		end
	end
	local out: { [string]: { any } } = {}
	for g, list in by do
		if g ~= "planned" then
			table.sort(list, function(a: any, b: any): boolean
				if a.at ~= b.at then
					return a.at > b.at
				end
				return a.i < b.i
			end)
		end
		local rows: { any } = {}
		for _, x in list do
			table.insert(rows, x.r)
		end
		out[g] = rows
	end
	return out
end

local function GroupHead(chat: string, group: string, n: () -> number, closed: () -> boolean): Node
	local key = `{chat}:{group}`
	local name = locale.s(`chat.threads.group.{group}`)
	return ui.button {
		id = `threads-group-{group}`,
		class = "threads-group",
		label = function(): string
			return `{name} · {n()}`
		end,
		expanded = function(): boolean
			return not closed()
		end,
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
			now[key] = if closed() then nil else true
			shut:set(now)
		end,
		ui.row {
			items = "center",
			gap = ui.raw(4, "the mockup's group head"),
			ui.text { `{name} ·`, literal = true, wrap = false },
			Transcript.Roll {
				id = `threads-group-{group}-count`,
				value = function(): string
					return tostring(n())
				end,
			},
		},
		ui.div {
			rotate = motion.tween(ui.computed(function(): number
				return if closed() then -90 else 0
			end), { duration = 200, ease = EASE }),
			Icon { name = "lucide-chevron-down", size = 12 },
		},
	}
end

local GroupSection = ui.component(function(props: { ctx: Ctx, group: string, rows: () -> { any } }): Node
	local ctx, group = props.ctx, props.group
	local chat = ctx.session
	local skey = `{chat}:{group}`
	local list = ui.computed(props.rows)
	local n = ui.computed(function(): number
		return #list:get()
	end)
	local closed = ui.computed(function(): boolean
		return shut:get()[skey] == true
	end)
	local reduced = motion.reduced_now()
	local built_at = ui.now()
	local opened_at = -1e9
	local body_on = ui.use_state(not closed:peek())
	local rows_ref = ui.ref()
	local was = closed:peek()
	ui.effect(function()
		local c = closed:get()
		if c == was then
			return
		end
		was = c
		if not c then
			opened_at = ui.now()
			if body_on:peek() then
				local els = rows_ref:elements()
				if #els > 0 and not motion.reduced_now() then
					anime.animate(els, {
						opacity = { 0, 1 },
						y = { -4, 0 },
						duration = 180,
						delay = function(_el: any, i: number): number
							return 35 * math.min(i, 8)
						end,
						ease = EASE,
					})
				end
			else
				body_on:set(true)
			end
		else
			local els = rows_ref:elements()
			if #els > 0 and not motion.reduced_now() then
				anime.animate(els, { opacity = 0, y = -4, duration = 180, ease = EASE })
			end
			ui.after(MOVE_MS, function()
				if closed:peek() then
					body_on:set(false)
				end
			end)
		end
	end)
	local alive = ui.use_state(n:peek() > 0)
	ui.effect(function()
		if n:get() > 0 then
			alive:set(true)
		else
			ui.after(MOVE_MS, function()
				if n:peek() == 0 then
					alive:set(false)
				end
			end)
		end
	end)
	local by_key = ui.computed(function(): { [string]: any }
		local out: { [string]: any } = {}
		for _, r in list:get() do
			out[M.key_of(r)] = r
		end
		return out
	end)
	local function item(r: any, i: number): Node
		local k, last = M.key_of(r), r
		local at = ui.now()
		local opening = at - opened_at < 100
		local arriving = not opening and at - built_at > 100
		local stagger = opening and not motion.reduced_now()
		return ui.column {
			ref = rows_ref,
			w_full = true,
			layout = if arriving and not reduced then "height" else nil,
			layout_clip = arriving,
			layout_enter = arriving and not reduced,
			layout_duration = MOVE_MS,
			layout_ease = EASE,
			opacity = if stagger then 0 else nil,
			on_mount = if stagger
				then function(el: any)
					anime.animate(el, { opacity = { 0, 1 }, y = { -4, 0 }, duration = 180, delay = 35 * math.min(i - 1, 8), ease = EASE })
				end
				else nil,
			ui.presence(function(): string
				return if by_key:get()[k] then "here" else "leaving"
			end, function(which: string): Node
				return RowView(ctx, function(): any
					local x = by_key:get()[k]
					if x then
						last = x
					end
					return last
				end, which == "here")
			end, { w_full = true }),
		}
	end
	local function body(): Node
		return ui.column {
			w_full = true,
			overflow = "hidden",
			layout = "height",
			layout_clip = true,
			layout_duration = if reduced then 0 else MOVE_MS,
			layout_ease = EASE,
			max_h = function(): number
				return if closed:get() then 0 else 1e6
			end,
			ui.show(
				body_on,
				ui.each(list, item, {
					key = function(r: any): string
						return M.key_of(r)
					end,
					w_full = true,
					animate = if reduced then nil else { duration = MOVE_MS, ease = EASE },
					enter = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, y = { 6, 0 }, duration = MOVE_MS, ease = EASE },
					exit = { opacity = 0, duration = motion.FADE },
				})
			),
		}
	end
	return ui.presence(function(): string?
		return if alive:get() then "on" else nil
	end, function(): Node
		local arriving = not reduced and ui.now() - built_at > 100
		return ui.column {
			id = `threads-section-{group}`,
			w_full = true,
			ui.column {
				w_full = true,
				overflow = "hidden",
				layout = "height",
				layout_clip = true,
				layout_enter = arriving,
				layout_duration = if reduced then 0 else MOVE_MS,
				layout_ease = EASE,
				max_h = function(): number
					return if n:get() > 0 then 1e6 else 0
				end,
				GroupHead(chat, group, function(): number
					return n:get()
				end, function(): boolean
					return closed:get()
				end),
			},
			body(),
		}
	end, { w_full = true })
end)

local function PlanBar(chat: string): Node
	local function plan(): any
		local e = M.of(chat)
		return e and e.plan
	end
	return ui.presence(function(): string
		local p = plan()
		return if not p then "" elseif p.pending then "pending" else "running"
	end, function(which: string): Node
		if which == "" then
			return ui.div {}
		end
		local pending = which == "pending"
		local function shown(): boolean
			return pending or overview_open:get()[chat] == true
		end
		local head: { any } = {
			w_full = true,
			items = "center",
			gap = ui.raw(6, "the mockup's plan bar"),
			ui.text { locale.t("chat.threads.plan.title"), weight = "$font.weight.semibold", wrap = false },
			Transcript.Roll {
				id = "threads-plan-progress",
				value = function(): string
					local p = plan()
					return if p and p.progress then `{p.progress.done}/{p.progress.total}` else ""
				end,
				font = "$font.mono",
				text_size = ui.raw(12, "the mockup's plan bar"),
				color = "$text.tertiary",
			},
			ui.div { grow = 1 },
		}
		if not pending then
			table.insert(head, ds.Button {
				ui.text {
					function(): string
						return locale.s(if overview_open:get()[chat] then "chat.threads.plan.hide" else "chat.threads.plan.overview")
					end,
					wrap = false,
				},
				label = locale.t("chat.threads.plan.overview"),
				id = "threads-plan-overview",
				tone = "ghost",
				size = "xs",
				expanded = shown,
				on_click = function()
					local now = table.clone(overview_open:peek())
					now[chat] = if now[chat] then nil else true
					overview_open:set(now)
				end,
			})
		end
		table.insert(head, ds.Button {
			locale.t("chat.threads.plan.edit"),
			id = "threads-plan-edit",
			tone = "ghost",
			size = "xs",
			on_click = function()
				M.edit(chat)
			end,
		})
		if pending then
			table.insert(head, ds.Button {
				locale.t("ds.pills.approve"),
				id = "threads-plan-approve",
				tone = "accent",
				size = "xs",
				on_click = function()
					M.approve(chat)
				end,
			})
		end
		return ui.column {
			id = "threads-plan",
			class = if pending then "threads-plan-pending" else "threads-plan-running",
			w_full = true,
			shrink = 0,
			mb = ui.raw(4, "the mockup's plan bar"),
			px = ui.raw(12, "the mockup's plan bar"),
			py = ui.raw(10, "the mockup's plan bar"),
			gap = ui.raw(6, "the mockup's plan bar"),
			rounded = ui.raw(10, "the mockup's plan bar"),
			border = 1,
			border_color = "$border.subtle",
			text_size = ui.raw(13, "the mockup's plan bar"),
			ui.row(head),
			if pending
				then ui.text { locale.t("chat.threads.plan.waiting"), id = "threads-plan-waiting", wrap = true, text_size = ui.raw(12.5, "the mockup's plan bar"), color = "$warning" }
				else ui.div {
					w_full = true,
					shrink = 0,
					ProgressBar {
						size = "thin",
						w_full = true,
						label = locale.t("chat.threads.plan.title"),
						value = function(): number?
							local p = plan()
							local total = if p and p.progress then p.progress.total else 0
							return if total > 0 then p.progress.done / total else 0
						end,
					},
				},
			ui.show(
				function(): boolean
					local p = plan()
					return shown() and p ~= nil and (p.overview or "") ~= ""
				end,
				ui.text {
					function(): string
						local p = plan()
						return if p then p.overview or "" else ""
					end,
					id = "threads-plan-overview-text",
					literal = true,
					wrap = true,
					text_size = ui.raw(12.5, "the mockup's plan bar"),
					color = "$text.secondary",
				}
			),
		}
	end, { w_full = true, shrink = 0 })
end

local function List(ctx: Ctx): Node
	local chat = ctx.session
	local placed: { [string]: { g: string, at: number } } = {}
	local groups = ui.computed(function(): { [string]: { any } }
		return grouped(chat, placed)
	end)
	local sections: { [any]: any } = { w_full = true }
	for _, g in GROUPS do
		table.insert(sections, GroupSection {
			ctx = ctx,
			group = g,
			rows = function(): { any }
				return groups:get()[g] or {}
			end,
		})
	end
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
					PlanBar(chat),
					ui.column(sections),
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
					local line = M.question(thread) or (if r and r.line then r.line else "")
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
			Lane.Tab { tab = { id = slot, kind = "side_chat", chat = thread, title = title(), placeholder = "chat.threads.placeholder" }, panel_key = ctx.key },
		},
	}
end)

local dragging = ui.state("chat_threads_dragging", nil :: string?)

local function load_routes()
	if routes:peek() ~= nil then
		return
	end
	routes:set(false)
	local p = M.host.call("settings.engine", { method = "GetRoutingSettings", params = {} })
	if p then
		p:after(function(v: any)
			routes:set(if type(v) == "table" then v else false)
		end, function() end)
	end
end

local function model_word(harness: string?, model: string?): string?
	if not model then
		return nil
	end
	return (require("@app/screens/chat/store") :: any).model_label(harness, model) or model
end

local function route_word(role: string?): string?
	local r = routes:get()
	local x = if role and type(r) == "table" and r.roles then r.roles[role] else nil
	if not x then
		return nil
	elseif x.kind == "target" then
		local name = Transcript.harness_name(x.harness)
		local model = model_word(x.harness, x.model)
		return if model then `{name} {model}` else name
	elseif x.kind == "sameAsParent" then
		return locale.s("chat.threads.edit.same_as_lead")
	end
	return nil
end

local function caps(key: string): Node
	return ui.text {
		string.upper(locale.s(key)),
		pt = ui.raw(12, "the mockup's editor label"),
		pb = ui.raw(6, "the mockup's editor label"),
		text_size = ui.raw(11, "the mockup's editor label"),
		weight = "$font.weight.medium",
		color = "$text.tertiary",
		wrap = false,
	}
end

local function default_model(chat: string, s: any): string?
	if not s then
		return nil
	end
	local d = drafts:get()[chat]
	local b = if d and s.id then d.base[s.id] else nil
	local r = live_row(chat, s.id)
	if b and b.role == s.role and r and r.routed then
		return r.routed.model
	end
	local routed = routes:get()
	local x = if s.role and type(routed) == "table" and routed.roles then routed.roles[s.role] else nil
	if x and x.kind == "target" then
		return if x.model then `{x.harness}/{x.model}` else nil
	end
	local entry = M.of(chat)
	local agent = entry and entry.plan and entry.plan.agent
	return agent and agent.model
end

local function role_menu(chat: string, key: string, now: () -> any, pick: { model: State<string?>, effort: State<string?> }): Node
	local open = ui.use_state(false)
	local first = now() or {}
	local locked = first.locked == true
	local dk = dom_id(key)
	local function ran(): any
		return live_row(chat, first.id) or first
	end
	local function started(): boolean
		return locked or ran().locked == true
	end
	local function said(): string
		return if started() then model_word(ran().harness, ran().model) or "" else ""
	end
	local anchor = ui.button {
		id = `threads-edit-role-{dk}`,
		label = function(): string
			local s = now()
			return locale.s("chat.threads.edit.role", { title = if s then s.title else "" })
		end,
		disabled = started,
		bg = "#00000000",
		h = 22,
		px = ui.raw(4, "the mockup's role button"),
		gap = ui.raw(5, "the mockup's role button"),
		items = "center",
		rounded = "$radius.sm",
		text_size = ui.raw(12, "the mockup's role button"),
		color = "$text.secondary",
		hover = if locked then {} else { bg = "$hover", color = "$text.primary" },
		cursor = if locked then "default" else "pointer",
		on_click = function()
			if not started() then
				open:set(not open:peek())
			end
		end,
		ui.show(
			started,
			Mark {
				name = function(): any
					return ran().mark
				end :: any,
				size = 12,
				bare = true,
			}
		),
		ui.text {
			function(): string
				local s = now()
				return if s and s.role then s.role else locale.s("chat.threads.edit.no_role")
			end,
			wrap = false,
		},
		ui.show(function(): boolean
			return said() ~= ""
		end, ui.text {
			function(): string
				return `· {said()}`
			end,
			literal = true,
			wrap = false,
			color = "$text.tertiary",
		}),
		ui.show(function(): boolean
			return not started()
		end, Icon { name = "lucide-chevron-down", size = 12 }),
	}
	local rows: { any } = {}
	for _, role in ROLES do
		table.insert(rows, Menu.Row {
			id = `threads-edit-role-{dk}-{role}`,
			label = role,
			role = "menuitemradio",
			checked = function(): boolean
				local s = now()
				return s ~= nil and s.role == role
			end,
			trail = {
				ui.text {
					function(): string
						return route_word(role) or ""
					end,
					literal = true,
					wrap = false,
					text_size = ui.raw(11.5, "the mockup's role menu"),
					color = "$text.tertiary",
				},
			},
			on_click = function()
				open:set(false)
				change_step(chat, key, function(s)
					s.role = role
				end)
			end,
		})
	end
	local default_row = Menu.Row {
		id = `threads-edit-model-{dk}-default`,
		label = locale.s("chat.threads.edit.default_model"),
		role = "menuitemradio",
		checked = function(): boolean
			local s = now()
			return s ~= nil and s.target == nil
		end,
		trail = {
			ui.text {
				function(): string
					local v = default_model(chat, now())
					return if v then model_word(nil, v) or "" else ""
				end,
				literal = true,
				wrap = false,
				text_size = ui.raw(11.5, "the mockup's role menu"),
				color = "$text.tertiary",
			},
		},
		on_click = function()
			open:set(false)
			ui.batch(function()
				pick.model:set(nil)
				pick.effort:set(nil)
			end)
			change_step(chat, key, function(s)
				s.target = nil
			end)
		end,
	}
	return ui.menu(Menu.panel {
		open = open,
		anchor = anchor,
		placement = { side = "bottom", align = "start", offset = 4 },
		w = 240,
		Menu.Group(rows),
		Menu.Rule(),
		Menu.Group { default_row },
	})
end

local function model_chip(chat: string, key: string, now: () -> any, pick: { model: State<string?>, effort: State<string?> }): Node
	local store = require("@app/screens/chat/store") :: any
	local function shown(): string?
		return pick.model:get() or default_model(chat, now())
	end
	local primed = false
	ui.effect(function()
		local m, e = pick.model:get(), pick.effort:get()
		if not primed then
			primed = true
			return
		end
		local v = m or (if e then default_model(chat, now()) else nil)
		local chosen = if v then { harness = string.match(v, "^([^/]+)/") or v, model = v, effort = e } else nil
		local s = now()
		if s and not same_pick(s.target, chosen) then
			change_step(chat, key, function(x)
				x.target = chosen
			end)
		end
	end)
	return ds.ModelPicker {
		model = pick.model,
		effort = pick.effort,
		harnesses = function(): { any }
			local c: any = store.catalog:get()
			return c.tabs or c.harnesses or {}
		end,
		models = function(): { any }
			return store.catalog:get().models or {}
		end,
		default_model = function(): string?
			local s = now()
			return if s and s.target and not s.target.model then nil else default_model(chat, s)
		end,
		model_label = function(): string?
			local v = shown()
			if v then
				return string.match(v, "^[^/]+/(.+)$") or v
			end
			local s = now()
			return if s and s.target then Transcript.harness_name(s.target.harness) else nil
		end,
		looking = function(): boolean
			return store.catalog:get().complete ~= true
		end,
		label = locale.s("chat.threads.edit.model", { title = (now() or {}).title or "" }),
		tone = "chip",
		placement = { side = "bottom", align = "start", offset = 4 },
	}
end

local StepRow = ui.component(function(props: { chat: string, key: string, number: () -> number }): Node
	local chat, key = props.chat, props.key
	local dk = dom_id(key)
	local function find(d: any): any
		if d then
			for _, s in d.steps do
				if s.key == key then
					return s
				end
			end
		end
		return nil
	end
	local function now(): any
		return find(drafts:get()[chat])
	end
	local first = find(drafts:peek()[chat]) or {}
	local title = ui.use_state(first.title or "")
	local spec = ui.use_state(first.spec or "")
	ui.effect(function()
		local v = title:get()
		local s = find(drafts:peek()[chat])
		if s and s.title ~= v then
			change_step(chat, key, function(x)
				x.title = v
			end)
		end
	end)
	ui.effect(function()
		local v = spec:get()
		local s = find(drafts:peek()[chat])
		if s and s.spec ~= v then
			change_step(chat, key, function(x)
				x.spec = v
			end)
		end
	end)
	local locked = first.locked == true
	local movable = not locked and not first.fixed
	local pick = {
		model = ui.use_state(if first.target then first.target.model else nil),
		effort = ui.use_state(if first.target then first.target.effort else nil),
	}
	local hovered = ui.use_state(false)
	local function open(): boolean
		local s = now()
		return s ~= nil and s.open == true
	end
	local function name(): string
		local s = now()
		return if s and trim(s.title) ~= "" then trim(s.title) else locale.s("chat.threads.edit.title_placeholder")
	end
	local lead: Node
	if locked then
		lead = ui.div { w = 16, shrink = 0, items = "center", tooltip = locale.s("chat.threads.edit.started"), M.state_icon(first.state or "working", 14) }
	elseif movable then
		lead = ui.div {
			id = `threads-edit-grip-{dk}`,
			class = "threads-edit-grip",
			label = locale.t("chat.threads.edit.drag"),
			w = 16,
			h = 24,
			shrink = 0,
			items = "center",
			justify = "center",
			color = function(): string
				return if hovered:get() or dragging:get() == key then "$text.secondary" else "$text.tertiary"
			end,
			cursor = function(): string
				return if dragging:get() == key then "grabbing" else "grab"
			end,
			on_drag = function(): any
				dragging:set(key)
				return { plan_step = key, plan_chat = chat }
			end,
			drag_preview = function(): Node
				local card: { [any]: any } = {
					w = 280,
					h = 36,
					px = "$space.3",
					gap = "$space.2",
					items = "center",
					rounded = "$radius.lg",
					color = "$text.primary",
					Icon { name = "grip", size = 14 },
					ui.text { name(), literal = true, grow = 1, wrap = false, truncate = true },
				}
				Glass.style(card, "menu")
				return ui.row(card)
			end,
			on_drag_end = function()
				dragging:set(nil)
			end,
			Icon { name = "grip", size = 14 },
		}
	else
		lead = ui.div { w = 16, shrink = 0 }
	end
	return ui.column {
		id = `threads-edit-step-{dk}`,
		class = if locked then "threads-edit-step threads-edit-step-locked" else "threads-edit-step",
		w_full = true,
		px = ui.raw(6, "the mockup's editor row"),
		py = ui.raw(6, "the mockup's editor row"),
		gap = ui.raw(4, "the mockup's editor row"),
		rounded = ui.raw(10, "the mockup's editor row"),
		bg = function(): string
			return if open() then "$hover" else "#00000000"
		end,
		opacity = function(): number
			return if dragging:get() == key then 0.35 else 1
		end,
		on_hover = function(inside: boolean)
			hovered:set(inside)
		end,
		drop_when = function(payload: any): boolean
			return movable and type(payload) == "table" and payload.plan_chat == chat and payload.plan_step ~= nil and payload.plan_step ~= key
		end,
		on_drag_over = function(payload: any, e: any)
			local d = drafts:peek()[chat]
			if not d then
				return
			end
			local from, here = nil, nil
			for i, s in d.steps do
				if s.key == payload.plan_step then
					from = i
				elseif s.key == key then
					here = i
				end
			end
			if not from or not here then
				return
			end
			local past_middle = e.local_y > e.h / 2
			if (from < here and past_middle) or (from > here and not past_middle) then
				move_step(chat, payload.plan_step, key)
			end
		end,
		on_drop = function(_payload: any) end,
		on_key = function(e: any): boolean?
			if movable and e.alt and (e.key == "ArrowUp" or e.key == "ArrowDown") then
				nudge(chat, key, if e.key == "ArrowUp" then -1 else 1)
				return true
			end
			return nil
		end,
		ui.row {
			w_full = true,
			items = "center",
			gap = ui.raw(6, "the mockup's editor row"),
			lead,
			ui.text {
				function(): string
					return tostring(props.number())
				end,
				w = 14,
				shrink = 0,
				tabular = true,
				text_size = ui.raw(12, "the mockup's editor row"),
				color = "$text.tertiary",
				wrap = false,
			},
			ui.div {
				grow = 1,
				min_w = 0,
				ds.TextField {
					value = title,
					size = "sm",
					w_full = true,
					name = function(): string
						return locale.s("chat.threads.edit.title_name", { n = props.number() })
					end,
					placeholder = locale.t("chat.threads.edit.title_placeholder"),
					autofocus = first.id == nil,
					error = function(): string?
						local s = now()
						return if s and s.id and trim(s.title) == "" then locale.s("chat.threads.edit.bad") else nil
					end,
				},
			},
			ui.div {
				shrink = 0,
				rotate = function(): number
					return if open() then 180 else 0
				end,
				IconButton {
					id = `threads-edit-spec-{dk}`,
					icon = "lucide-chevron-down",
					label = function(): string
						return locale.s("chat.threads.edit.spec_toggle", { n = props.number() })
					end :: any,
					size = "sm",
					tone = "quiet",
					on_click = function()
						change_step(chat, key, function(s)
							s.open = not s.open
						end)
					end,
				},
			},
			if locked
				then ui.div { w = 28, shrink = 0 }
				else IconButton {
					id = `threads-edit-remove-{dk}`,
					icon = "close",
					label = function(): string
						return locale.s("chat.threads.edit.remove", { n = props.number() })
					end :: any,
					size = "sm",
					tone = "quiet",
					on_click = function()
						change(chat, function(d)
							for i, s in d.steps do
								if s.key == key then
									table.remove(d.steps, i)
									if s.id then
										table.insert(d.removed, s.id)
									end
									break
								end
							end
						end)
					end,
				},
		},
		ui.row {
			pl = ui.raw(42, "the mockup's editor row"),
			items = "center",
			role_menu(chat, key, now, pick),
			if locked
				then nil
				else ui.show(function(): boolean
					local s = now()
					local r = if s then live_row(chat, s.id) else nil
					return not (r and r.locked == true)
				end, model_chip(chat, key, now, pick)),
		},
		ui.show(
			open,
			ui.column {
				w_full = true,
				pl = ui.raw(42, "the mockup's editor row"),
				pr = ui.raw(6, "the mockup's editor row"),
				gap = ui.raw(6, "the mockup's editor row"),
				ds.TextArea {
					value = spec,
					w_full = true,
					rows = 2,
					max_rows = 10,
					size = "sm",
					name = function(): string
						return locale.s("chat.threads.edit.spec_name", { n = props.number() })
					end,
					placeholder = locale.t("chat.threads.edit.spec_placeholder"),
				},
				if locked
					then ui.text { locale.t("chat.threads.edit.locked"), id = `threads-edit-locked-{dk}`, wrap = true, text_size = ui.raw(11.5, "the mockup's editor row"), color = "$text.tertiary" }
					else nil,
			}
		),
	}
end)

local function Foot(chat: string): Node
	local function counts(): (number, boolean)
		local d = drafts:get()[chat]
		if not d then
			return 0, false
		end
		return M.changes(d)
	end
	return ui.presence(function(): string
		local d = drafts:get()[chat]
		return if d and d.confirm then "confirm" else "edit"
	end, function(which: string): Node
		local row: { any } = {
			id = "threads-edit-foot",
			w_full = true,
			h = ui.raw(48, "the mockup's editor foot"),
			shrink = 0,
			px = ui.raw(14, "the mockup's editor foot"),
			gap = ui.raw(6, "the mockup's editor foot"),
			items = "center",
			border = 1,
			border_sides = { 1, 0, 0, 0 },
			border_color = "$border.subtle",
			text_size = ui.raw(12.5, "the mockup's editor foot"),
		}
		if which == "confirm" then
			table.insert(row, ui.text {
				function(): string
					return locale.s("chat.threads.edit.discard_ask", { n = (counts()) })
				end,
				id = "threads-edit-discard-ask",
				grow = 1,
				wrap = false,
				color = "$warning",
			})
			table.insert(row, ds.Button {
				locale.t("chat.threads.edit.keep"),
				id = "threads-edit-keep",
				tone = "secondary",
				size = "sm",
				on_click = function()
					change(chat, function(d)
						d.confirm = false
					end)
				end,
			})
			table.insert(row, ds.Button {
				locale.t("chat.threads.edit.discard"),
				id = "threads-edit-discard",
				tone = "ghost",
				size = "sm",
				color = "$danger",
				on_click = function()
					close_editor(chat)
				end,
			})
			return ui.row(row)
		end
		table.insert(row, ui.text {
			function(): string
				local n, bad = counts()
				return if bad then locale.s("chat.threads.edit.bad") elseif n > 0 then locale.s("chat.threads.edit.count", { n = n }) else locale.s("chat.threads.edit.none")
			end,
			id = "threads-edit-count",
			grow = 1,
			wrap = false,
			color = "$text.secondary",
		})
		table.insert(row, ds.Button {
			locale.t("chat.threads.edit.cancel"),
			id = "threads-edit-cancel",
			tone = "ghost",
			size = "sm",
			on_click = function()
				leave(chat)
			end,
		})
		table.insert(row, ds.Button {
			locale.t("chat.threads.edit.save"),
			id = "threads-edit-save",
			size = "sm",
			disabled = function(): boolean
				local n, bad = counts()
				return n == 0 or bad
			end,
			busy = function(): boolean
				local d = drafts:get()[chat]
				return d ~= nil and d.saving == true
			end,
			on_click = function()
				save(chat)
			end,
		})
		return ui.row(row)
	end, { w_full = true, shrink = 0 })
end

local Editor = ui.component(function(props: { chat: string }): Node
	local chat = props.chat
	if not drafts:peek()[chat] then
		local all = table.clone(drafts:peek())
		all[chat] = new_draft(chat)
		drafts:set(all)
	end
	load_routes()
	local d0 = drafts:peek()[chat]
	local overview = ui.use_state(if d0 then d0.overview else "")
	ui.effect(function()
		local v = overview:get()
		local d = drafts:peek()[chat]
		if d and d.overview ~= v then
			change(chat, function(x)
				x.overview = v
			end)
		end
	end)
	local keys = ui.computed(function(): { string }
		local out: { string } = {}
		local d = drafts:get()[chat]
		for _, s in (if d then d.steps else {}) do
			table.insert(out, s.key)
		end
		return out
	end)
	local function number_of(key: string): () -> number
		return function(): number
			return table.find(keys:get(), key) or 0
		end
	end
	return ui.column {
		id = "threads-edit",
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
				id = "threads-edit-back",
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
					leave(chat)
				end,
				Icon { name = "alt-arrow-left", size = 14 },
				ui.text { locale.t("panel.tab.threads"), wrap = false },
			},
			ui.text { locale.t("chat.threads.edit.head"), grow = 1, weight = "$font.weight.semibold", text_size = ui.raw(13.5, "the mockup's editor head"), wrap = false },
			IconButton {
				id = "threads-edit-close",
				icon = "close",
				label = locale.s("chat.threads.edit.close"),
				size = "sm",
				tone = "quiet",
				on_click = function()
					leave(chat)
				end,
			},
		},
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.column {
				w_full = true,
				px = ui.raw(12, "the mockup's editor"),
				pb = ui.raw(16, "the mockup's editor"),
				caps("chat.threads.edit.overview"),
				ds.TextArea { value = overview, w_full = true, rows = 2, max_rows = 8, size = "sm", name = locale.t("chat.threads.edit.overview") },
				caps("chat.threads.edit.steps"),
				ui.each(keys, function(key: string): Node
					return StepRow { chat = chat, key = key, number = number_of(key) }
				end, {
					key = function(key: string): string
						return key
					end,
					w_full = true,
					gap = ui.raw(2, "the mockup's editor rows"),
				}),
				ui.button {
					id = "threads-edit-add",
					label = locale.t("chat.threads.edit.add"),
					mt = ui.raw(8, "the mockup's add step"),
					w_full = true,
					h = ui.raw(36, "the mockup's add step"),
					px = ui.raw(12, "the mockup's add step"),
					gap = ui.raw(6, "the mockup's add step"),
					items = "center",
					rounded = ui.raw(10, "the mockup's add step"),
					border = 1,
					border_color = "$border.subtle",
					bg = "#00000000",
					color = "$text.secondary",
					hover = { bg = "$hover", color = "$text.primary" },
					on_click = function()
						change(chat, function(d)
							d.added += 1
							table.insert(d.steps, { key = `new-{d.added}`, title = "", spec = "", role = "worker", locked = false, fixed = false, open = false })
						end)
					end,
					Icon { name = "plus", size = 14 },
					ui.text { locale.t("chat.threads.edit.add"), wrap = false },
				},
			},
		},
		Foot(chat),
	}
end)

M.Tab = ui.component(function(props: { tab: any, panel_key: string }): Node
	local ctx: Ctx = { session = props.tab.chat or "", key = props.panel_key, tab = props.tab.id }
	mount(ctx.session)
	return ui.presence(function(): string
		for _, t in state.of(ctx.key).tabs do
			if t.id == ctx.tab then
				local entry = if t.editing then M.of(ctx.session) else nil
				return if entry and entry.plan then "\0edit" else t.thread or ""
			end
		end
		return ""
	end, function(thread: string): Node
		if thread == "" then
			return List(ctx)
		elseif thread == "\0edit" then
			return Editor { chat = ctx.session }
		end
		return ThreadView { ctx = ctx, thread = thread }
	end, { w_full = true, grow = 1, min_h = 0 })
end)

return M
