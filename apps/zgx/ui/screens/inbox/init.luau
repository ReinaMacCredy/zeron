-- The Inbox page (crates/ui inbox.rs); zgx has no ⌘I tray, so its banner opens the chats the sidebar Inbox marks.

local ds = require("@ds")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local store = require("@app/screens/tasks/store")
local tasks_page = require("@app/screens/tasks")
local card = require("@app/screens/tasks/card")

local M = {}

local PAGE_W, TILE_RADIUS, RUNS_EVERY = 1760, 16, 60000

local waiting = ui.shared("chat.waiting", {} :: { any })

-- What needs you, as the shipping app's tray counts it past its waits (shell.rs `render_inbox_page`): chats asking with no question to answer here, chats failed, newest first, then tasks Not sent or Ready to review.
function M.needs(): { any }
	local asked: { [string]: boolean } = {}
	for _, w in waiting:get() do
		asked[w.chat] = true
	end
	local out = {}
	for _, c in sessions.now:get().chats do
		if (c.status == "input" and not asked[c.id]) or c.status == "failed" then
			table.insert(out, { chat = c })
		end
	end
	table.sort(out, function(a: any, b: any): boolean
		return (a.chat.updated or 0) > (b.chat.updated or 0)
	end)
	for _, t in store.parts:get().open do
		local reason = (tasks_page.line(t).state :: store.State).reason
		if reason == "not_sent" or reason == "review" then
			table.insert(out, { task = t })
		end
	end
	return out
end
local needs = ui.computed(M.needs)

M.runs = ui.state("inbox_runs", nil :: any)
M.runs_failed = ui.state("inbox_runs_failed", false)

local function same(a: any, b: any): boolean
	if type(a) ~= "table" or type(b) ~= "table" then
		return a == b
	end
	for k, v in a do
		if not same(v, b[k]) then
			return false
		end
	end
	for k in b do
		if a[k] == nil then
			return false
		end
	end
	return true
end

local function refresh()
	local p = store.host.today({ since = store.day_start() })
	if not p then
		M.runs_failed:set(true)
		return
	end
	p:after(function(v: any)
		local got = v or { runs = {}, waits = {} }
		if not same(M.runs:peek(), got) then
			M.runs:set(got)
		end
		M.runs_failed:set(false)
	end, function()
		M.runs_failed:set(true)
	end)
end
M.refresh = refresh

local function title_of(id: string, fallback: string?): string
	local c = store.chat_of(id)
	return if c then c.title else fallback or locale.s("chat.completions.new_session")
end

-- Today's history (inbox.rs `render_history`): a chat's last run Failed (its last turn errored, seen or not) or Finished, Stopped, and the questions you answered, newest first.
function M.history(): { any }
	local r = M.runs:get()
	if not r then
		return {}
	end
	local items = {}
	local latest: { [string]: number } = {}
	for _, run in r.runs or {} do
		latest[run.chatId] = math.max(latest[run.chatId] or 0, run.startedAtMs or 0)
	end
	for _, run in r.runs or {} do
		local what = nil
		if run.status == "complete" then
			local c = store.chat_of(run.chatId)
			what = if c and c.errored and latest[run.chatId] == run.startedAtMs then "failed" else "finished"
		elseif run.status == "aborted" then
			what = "stopped"
		end
		if what then
			table.insert(items, { at = (run.startedAtMs or 0) + (run.durationMs or 0), what = what, chat = run.chatId, title = title_of(run.chatId, run.title) })
		end
	end
	for _, w in r.waits or {} do
		if w.endedAtMs then
			table.insert(items, { at = w.endedAtMs, what = "answered", chat = w.chatId, title = title_of(w.chatId) })
		end
	end
	table.sort(items, function(a: any, b: any): boolean
		return a.at > b.at
	end)
	return items
end

local WHAT = { failed = "inbox.what.failed", stopped = "inbox.what.stopped", finished = "inbox.what.finished", answered = "inbox.what.answered" }
local TONES = { failed = "$danger", stopped = "$danger", finished = "$text.secondary", answered = "$accent" }
local GLYPHS = { failed = "lucide-circle-alert", stopped = "lucide-circle-alert", finished = "check", answered = "lucide-message-circle-question" }

local function Tile(props: { [any]: any }): Node
	local p = table.clone(props)
	p.rounded = ui.raw(TILE_RADIUS, "gpui")
	p.bg = "$hover"
	p.w_full = true
	return ui.column(p)
end

local function Heading(word: string, n: () -> number, extra: Node?): Node
	return ui.row {
		w_full = true,
		justify = "between",
		items = "center",
		pb = ui.raw(8, "gpui"),
		ui.row { gap = ui.raw(6, "gpui"), items = "baseline", ui.text { locale.s(word), role = "heading", weight = "$font.weight.semibold", text_size = ui.raw(14, "gpui") }, ui.text { function(): string
			return tostring(n())
		end, color = "$text.secondary", text_size = ui.raw(13, "gpui") } },
		extra,
	}
end

local function Banner(): Node
	return ui.show(function(): boolean
		return #needs:get() > 0
	end, Tile {
		id = "inbox-banner",
		px = ui.raw(16, "gpui"),
		py = ui.raw(12, "gpui"),
		gap = ui.raw(10, "gpui"),
		direction = "row",
		items = "center",
		ds.Icon { name = "bell", size = 16, color = "$text.secondary" },
		ui.text { function(): string
			return locale.s("inbox.needs", { n = #needs:get() })
		end, grow = 1, truncate = true, text_size = ui.raw(13.5, "gpui"), color = "$text.secondary" },
		ui.button {
			id = "inbox-open",
			label = locale.s("inbox.open"),
			h = 30,
			px = ui.raw(12, "gpui"),
			rounded = ui.raw(8, "gpui"),
			bg = "$zeron.tasks.wash",
			hover = { bg = "$zeron.tasks.wash_hover" },
			color = "$accent",
			weight = "$font.weight.semibold",
			text_size = ui.raw(13, "gpui"),
			on_click = function()
				local first = needs:peek()[1]
				if first and first.chat then
					route.open_session(first.chat.id)
				elseif first then
					card.open(first.task.id)
				end
			end,
			ui.text { locale.s("inbox.open") },
		},
	})
end

local function History(): Node
	return Tile {
		id = "inbox-history",
		px = ui.raw(20, "gpui"),
		py = ui.raw(18, "gpui"),
		Heading("inbox.today", function(): number
			return #M.history()
		end),
		ui.show(function(): boolean
			return #M.history() == 0
		end, ui.text { function(): string
			if M.runs_failed:get() then
				return locale.s("inbox.runs_failed")
			end
			return locale.s(if M.runs:get() == nil then "inbox.history_loading" else "inbox.history_empty")
		end, color = "$text.secondary", text_size = ui.raw(12.5, "gpui") }),
		ui.each(ui.computed(function(): { any }
			return M.history()
		end), function(item: any): Node
			return ui.button {
				label = `{locale.s(WHAT[item.what])} {item.title}`,
				w_full = true,
				h = 32,
				px = 0,
				gap = ui.raw(10, "gpui"),
				items = "center",
				justify = "start",
				bg = "#00000000",
				hover = { bg = "#00000000", color = "$text.primary" },
				on_click = function()
					route.open_session(item.chat)
				end,
				ui.row { w = 120, gap = ui.raw(6, "gpui"), items = "center", shrink = 0, color = TONES[item.what], text_size = ui.raw(12.5, "gpui"), ds.Icon { name = GLYPHS[item.what], size = 14 }, ui.text { locale.s(WHAT[item.what]) } },
				ui.text { item.title, literal = true, truncate = true, min_w = 0, text_size = ui.raw(13, "gpui"), color = "$text.primary" },
			}
		end),
	}
end

-- Today's tasks (inbox.rs `render_tasks`): check, title, In {chat}, Overdue or Today; a row opens its card; the add row makes one due today.
type Line = { task: store.Task, key: string, meta: string?, late: boolean }
local function line(t: store.Task): Line
	local meta: string? = nil
	local late = false
	if not t.completed then
		local c = store.chat_of(t.chat)
		if c then
			meta = locale.s("inbox.in_chat", { title = c.title })
		elseif t.due and t.due < store.work_day() then
			meta, late = locale.s("inbox.overdue"), true
		elseif t.due then
			meta = locale.s("inbox.due_today")
		end
	end
	return { task = t, key = t.id, meta = meta, late = late }
end

local function TaskRow(l: Line): Node
	local t, meta = l.task, l.meta
	local done = t.completed ~= nil
	return ui.row {
		id = `inbox-task-{t.id}`,
		role = "listitem",
		label = tasks_page.title_of(t),
		w_full = true,
		min_h = 40,
		py = ui.raw(4, "gpui"),
		gap = ui.raw(12, "gpui"),
		items = "center",
		rounded = ui.raw(10, "gpui"),
		cursor = "pointer",
		hover = { bg = "$hover" },
		on_click = function()
			card.open(t.id)
		end,
		tasks_page.Check { task = t, id = `inbox-task-check-{t.id}` },
		ui.column {
			grow = 1,
			min_w = 0,
			ui.rich { ui.span { tasks_page.title_of(t), literal = true, strike = done }, truncate = true, text_size = ui.raw(13.5, "gpui"), color = if done then "$text.secondary" else "$text.primary" },
			if meta then ui.text { meta, literal = true, truncate = true, text_size = ui.raw(12, "gpui"), color = if l.late then "$danger" else "$text.secondary" } else nil,
		},
	}
end

local function Tasks(): Node
	local list = ui.computed(function(): { Line }
		local out: { Line } = {}
		for _, t in store.today_tasks() do
			table.insert(out, line(t))
		end
		return out
	end)
	return Tile {
		id = "inbox-tasks",
		px = ui.raw(20, "gpui"),
		py = ui.raw(18, "gpui"),
		Heading("inbox.todays_tasks", function(): number
			local n = 0
			for _, l in list:get() do
				if not l.task.completed then
					n += 1
				end
			end
			return n
		end, ui.button {
			id = "inbox-all-tasks",
			label = locale.s("inbox.all_tasks"),
			p = 0,
			bg = "#00000000",
			hover = { bg = "#00000000", underline = true },
			color = "$accent",
			text_size = ui.raw(13, "gpui"),
			on_click = function()
				route.plugin_page("zeron", "tasks")
			end,
			ui.text { locale.s("inbox.all_tasks") },
		}),
		ui.column {
			role = "list",
			w_full = true,
			ui.each(list, TaskRow),
		},
		tasks_page.AddRow { id = "inbox-add", placeholder = locale.s("inbox.add_placeholder"), due = function(): string
			return store.work_day()
		end },
	}
end

M.Page = ui.component(function(): Node
	refresh()
	local live = true
	local function again()
		ui.after(RUNS_EVERY, function()
			if live then
				refresh()
				again()
			end
		end)
	end
	again()
	ui.on_cleanup(function()
		live = false
	end)
	return ui.div {
		id = "inbox-root",
		w_full = true,
		h_full = true,
		relative = true,
		on_key = function(e: any): boolean
			if e.key == "Escape" and card.current:peek() then
				card.close()
				return true
			end
			return false
		end,
		ui.scroll {
			id = "inbox-page",
			w_full = true,
			h_full = true,
			items = "center",
			ui.column {
				w_full = true,
				max_w = PAGE_W,
				px = ui.raw(40, "gpui"),
				pt = ui.raw(28, "gpui"),
				pb = ui.raw(48, "gpui"),
				gap = ui.raw(28, "gpui"),
				ui.row {
					gap = ui.raw(12, "gpui"),
					items = "baseline",
					wrap = true,
					ui.text { locale.s("inbox.title"), role = "heading", text_size = ui.raw(30, "gpui"), line_height = ui.raw(36, "gpui"), weight = "$font.weight.medium" },
					ui.text { id = "inbox-greeting", locale.s("inbox.greeting"), text_size = ui.raw(15, "gpui"), color = "$text.secondary", wrap = true },
				},
				Banner(),
				ui.row {
					w_full = true,
					gap = ui.raw(16, "gpui"),
					items = "start",
					wrap = true,
					ui.column { grow = 1, basis = 300, gap = ui.raw(16, "gpui"), History(), Tasks() },
				},
			},
		},
		card.Card {},
	}
end)

return M
