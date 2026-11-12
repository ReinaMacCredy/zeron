-- The Tasks page, trimmed as .maestro/topic/zgx-port/mock/tasks-inbox.html shows (crates/ui tasks.rs).

local ds = require("@ds")
local locale = require("@ds/locale")
local Lightbox = require("@ds/lightbox")
local store = require("@app/screens/tasks/store")
local card = require("@app/screens/tasks/card")

local M = {}

local LIST_W, ROW_H, META_H, ROW_GAP, DONE_H, CHECK = 640, 44, 46, 2, 36, 18
local THUMB, THUMBS = 28, 3

M.view = ui.persist("tasks_view", "list")
M.done_open = ui.persist("tasks_done_open", false)
M.lit = ui.state("tasks_lit", nil :: string?)
M.scroll = ui.state("tasks_scroll", 0)

local title_of = store.title_of
M.title_of = title_of

local function sep(): Node
	return ui.text { "·", color = "$text.tertiary", shrink = 0 }
end

-- A row's meta line: project, due, linked thread and its word, Not sent, Chat deleted, Blocked by (tasks.rs `meta`).
function M.meta(t: store.Task, given: store.State?): { Node }
	local parts: { Node } = {}
	local function add(n: Node)
		if #parts > 0 then
			table.insert(parts, sep())
		end
		table.insert(parts, n)
	end
	local s = given or store.state(t)
	local project = store.project_name(t.project)
	if project then
		add(ui.text { project, literal = true, truncate = true, shrink = 1, min_w = 0 })
	end
	local due, late = store.due_label(t)
	if due then
		add(ui.text { due, literal = true, color = if late then "$danger" else nil, shrink = 0 })
	end
	if s.column ~= "done" then
		if s.chat then
			add(ui.text { title_of({ title = s.chat.title } :: any), literal = true, truncate = true, max_w = 220, shrink = 1, min_w = 0, id = `task-row-thread-{t.id}` })
			if s.reason == "not_sent" then
				add(ui.text { locale.s("tasks.not_sent"), color = "$warning", shrink = 0 })
			else
				add(ui.text { store.linked_word(s.chat), color = store.TONES[s.column], shrink = 0 })
			end
		elseif s.reason == "not_sent" then
			add(ui.text { locale.s("tasks.not_sent"), color = "$warning", shrink = 0 })
		elseif s.gone then
			add(ui.text { locale.s("tasks.chat_deleted"), shrink = 0 })
		end
		local open, titles = false, {}
		for _, id in t.depends or {} do
			local d = store.find(id)
			if d then
				open = open or not d.completed
				table.insert(titles, ui.rich { ui.span { title_of(d), literal = true, strike = d.completed ~= nil }, truncate = true, max_w = 180, color = if d.completed then nil else "$text.primary" })
			end
		end
		if #titles > 0 then
			add(ui.row { id = `task-row-blocked-{t.id}`, gap = ui.raw(5, "gpui"), min_w = 0, items = "center", ui.text { locale.s(if open then "tasks.blocked_by" else "tasks.depends_on"), shrink = 0 }, table.unpack(titles) })
		end
	end
	return parts
end


-- The check: a ring, filled with the accent once done (tasks.rs `check_visual`).
function M.Check(props: { task: store.Task, id: string }): Node
	local t = props.task
	local done = t.completed ~= nil
	return ui.button {
		id = props.id,
		role = "checkbox",
		checked = done,
		label = locale.s(if done then "tasks.mark_not_done" else "tasks.mark_done"),
		w = CHECK,
		h = CHECK,
		shrink = 0,
		p = 0,
		rounded = "$radius.full",
		items = "center",
		justify = "center",
		border = if done then 0 else 1,
		border_color = "$text.tertiary",
		bg = if done then "$accent" else "#00000000",
		hover = if done then nil else { border_color = "$accent", bg = "$zeron.tasks.wash" },
		cursor = "pointer",
		on_click = function()
			store.set_done(t.id, not done)
		end,
		if done then ds.Icon { name = "check", size = 12, color = ui.raw("#ffffff", "white on the accent, as gpui") } else nil,
	}
end

function M.Thumbs(props: { task: store.Task, size: number?, group: string }): Node?
	local list = props.task.images or {}
	if #list == 0 then
		return nil
	end
	local size = props.size or THUMB
	local function items(): { any }
		local out = {}
		for i, path in list do
			local src = store.image_src(path)
			if src then
				table.insert(out, { src = src, name = string.match(path, "[^/]+$") or path, path = path, key = i })
			end
		end
		return out
	end
	local row: { [any]: any } = { gap = ui.raw(4, "gpui"), shrink = 0, items = "center" }
	for i, path in list do
		if i > THUMBS then
			break
		end
		table.insert(row, ui.div {
			id = `task-thumb-{props.task.id}-{i}`,
			w = size,
			h = size,
			rounded = ui.raw(6, "gpui"),
			overflow = "hidden",
			bg = "$hover",
			items = "center",
			justify = "center",
			tooltip = card.missing_tip(path),
			ui.show(function(): boolean
				return store.image_src(path) ~= nil
			end, Lightbox.Thumb {
				group = props.group,
				index = i,
				items = items,
				radius = 6,
				w = size,
				h = size,
				card.picture(path, size, 14),
			}, ds.Icon { name = "gallery", size = 14, color = "$text.tertiary" }),
		})
	end
	return ui.row(row)
end

local function NewTag(): Node
	return ui.text {
		locale.s("tasks.new"),
		text_size = ui.raw(10, "gpui"),
		color = "$accent",
		bg = "$zeron.tasks.wash",
		rounded = ui.raw(4, "gpui"),
		px = ui.raw(5, "gpui"),
		ml = ui.raw(6, "gpui"),
		shrink = 0,
	}
end

local function open_card(id: string)
	card.open(id)
end

-- A list row (tasks.rs `row_content`): the check, the title (struck once done), "New" on a capture, the meta line, the thumbs.
local function Row(t: store.Task, s: store.State): Node
	local meta = M.meta(t, s)
	local done = t.completed ~= nil
	return ui.row {
		id = `task-row-{t.id}`,
		role = "listitem",
		label = title_of(t),
		w_full = true,
		min_h = ROW_H,
		px = ui.raw(12, "gpui"),
		py = ui.raw(6, "gpui"),
		gap = ui.raw(12, "gpui"),
		items = "center",
		rounded = ui.raw(12, "gpui"),
		cursor = "pointer",
		bg = function(): string
			if card.current:get() == t.id then
				return "$zeron.tasks.selected"
			end
			return if M.lit:get() == t.id then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		on_click = function()
			M.lit:set(t.id)
			open_card(t.id)
		end,
		M.Check { task = t, id = `task-check-{t.id}` },
		ui.column {
			grow = 1,
			min_w = 0,
			ui.row {
				items = "center",
				min_w = 0,
				ui.rich { ui.span { title_of(t), literal = true, strike = done }, truncate = true, text_size = ui.raw(13.5, "gpui"), line_height = ui.raw(18, "gpui"), color = if done then "$text.secondary" else "$text.primary", min_w = 0, shrink = 1 },
				if store.is_new(t.id) then NewTag() else nil,
			},
			if #meta > 0 then ui.row { gap = ui.raw(6, "gpui"), items = "center", min_w = 0, text_size = ui.raw(12, "gpui"), line_height = ui.raw(16, "gpui"), color = "$text.secondary", wrap = false, overflow = "hidden", table.unpack(meta) } else nil,
		},
		M.Thumbs { task = t, group = `task-row-{t.id}` },
	}
end

-- A task's line: it, its state, and what its meta line reads, so a row is built again when its thread, project or blockers change (plain fields, no nodes).
type Line = { kind: string, task: store.Task?, key: string, n: number?, state: store.State?, said: string?, meta: boolean? }
-- (Kept by task: a sessions publish makes again only the lines whose chat or project changed.)
local memo: { [string]: any } = {}
function M.line(t: store.Task): Line
	local c = store.chat_of(t.chat)
	local project = store.project_name(t.project)
	local m = memo[t.id]
	if m and m.t == t and m.c == (c ~= nil) and m.status == (c and c.status) and m.title == (c and c.title) and m.project == project and #(t.depends or {}) == 0 and #(t.steps or {}) == 0 then
		return m.line
	end
	local l = M.line_of(t, project)
	memo[t.id] = { t = t, c = c ~= nil, status = c and c.status, title = c and c.title, project = project, line = l }
	return l
end

function M.line_of(t: store.Task, project: string?): Line
	local s = store.state(t)
	local c = s.chat
	local blockers = ""
	if not t.completed then
		for _, id in t.depends or {} do
			local d = store.find(id)
			if d then
				blockers ..= `{if d.completed then "+" else "-"}{d.title}\n`
			end
		end
	end
	local open = s.column ~= "done"
	local meta = project ~= nil or t.due ~= nil or (open and (c ~= nil or s.reason == "not_sent" or s.gone or blockers ~= ""))
	local slim = { column = s.column, reason = s.reason, gone = s.gone, chat = if c then { id = c.id, title = c.title, status = c.status } else nil }
	return { kind = "task", task = t, key = t.id, state = slim, said = `{project or ""}|{blockers}`, meta = meta }
end

-- What the list shows: open rows, the Done toggle, and done rows while open.
local lines = ui.computed(function(): { Line }
	local p = store.parts:get()
	local out: { Line } = {}
	for _, t in p.open do
		table.insert(out, M.line(t))
	end
	if #p.done > 0 then
		table.insert(out, { kind = "done", key = "done", n = #p.done })
		if M.done_open:get() then
			for _, t in p.done do
				table.insert(out, M.line(t))
			end
		end
	end
	return out
end)
M.lines = lines

local function DoneToggle(n: number): Node
	return ui.button {
		id = "tasks-done-toggle",
		label = locale.s("tasks.done_count", { n = n }),
		expanded = function(): boolean
			return M.done_open:get()
		end,
		h = DONE_H,
		px = ui.raw(12, "gpui"),
		gap = ui.raw(6, "gpui"),
		items = "center",
		bg = "#00000000",
		hover = { bg = "#00000000", color = "$text.primary" },
		color = "$text.secondary",
		text_size = ui.raw(12.5, "gpui"),
		on_click = function()
			M.done_open:set(not M.done_open:peek())
		end,
		ds.Icon { name = function(): string
			return if M.done_open:get() then "alt-arrow-down" else "alt-arrow-right"
		end, size = 12 },
		ui.text { locale.s("tasks.done") },
		ui.text { tostring(n), color = "$text.tertiary" },
	}
end

local list_ref = ui.ref()

-- ↑ ↓ walk the rows, Enter opens one's card, Space ticks it (Zeron's list is mouse-first; this is the port's keyboard way); the lit row is kept by its task, not its place.
function M.keys(e: any): boolean
	if e.editing or e.cmd or e.ctrl or e.meta or e.alt then
		return false
	end
	local k = e.key
	local l = lines:peek()
	local rows: { number } = {}
	local at = 0
	for i, x in l do
		if x.kind == "task" then
			table.insert(rows, i)
			if x.key == M.lit:peek() then
				at = #rows
			end
		end
	end
	if #rows == 0 then
		return false
	end
	if k == "ArrowDown" or k == "ArrowUp" then
		local to = if k == "ArrowDown" then math.min(#rows, at + 1) else math.max(1, at - 1)
		M.lit:set(l[rows[to]].key)
		local el = list_ref:peek()
		if el then
			el:scroll_to_row(rows[to])
		end
		return true
	end
	local t = if at > 0 then l[rows[at]].task else nil
	if not t then
		return false
	end
	if k == "Enter" then
		open_card(t.id)
		return true
	elseif k == "Space" or k == " " then
		store.set_done(t.id, t.completed == nil)
		return true
	end
	return false
end

-- The add row (tasks.rs `render_add_row`): Enter saves at the top of To do, Tab saves and opens the card on its notes, Escape clears.
function M.AddRow(props: { id: string, placeholder: string, due: (() -> string?)?, on_tab: ((string) -> ())? }): Node
	local text = ui.use_state("")
	local function save(then_open: boolean)
		local title = text:peek()
		if string.match(title, "^%s*$") then
			return false
		end
		text:set("")
		store.create({ title = title, due = if props.due then props.due() else nil }, function(id: string)
			if then_open then
				card.open(id, "notes")
			end
		end)
		return true
	end
	return ui.row {
		w_full = true,
		min_h = ROW_H,
		px = ui.raw(12, "gpui"),
		py = ui.raw(6, "gpui"),
		gap = ui.raw(12, "gpui"),
		items = "center",
		rounded = ui.raw(12, "gpui"),
		ui.div { w = CHECK, h = CHECK, rounded = "$radius.full", border = 1, border_style = "dotted", border_color = "$text.tertiary", shrink = 0 },
		ui.input {
			id = props.id,
			label = props.placeholder,
			placeholder = props.placeholder,
			placeholder_color = "$text.tertiary",
			value = function(): string
				return text:get()
			end,
			on_change = function(v: string)
				text:set(v)
			end,
			on_key = function(e: any): boolean
				if e.key == "Enter" then
					save(false)
					return true
				elseif e.key == "Tab" and not e.shift then
					return save(true)
				elseif e.key == "Escape" and text:peek() ~= "" then
					text:set("")
					return true
				end
				return false
			end,
			grow = 1,
			h = 20,
			px = 0,
			border = 0,
			bg = "#00000000",
			focus = { border = 0, outline = 0 },
			text_size = ui.raw(13.5, "gpui"),
		},
	}
end

local function Header(): Node
	return ui.row {
		w_full = true,
		px = ui.raw(12, "gpui"),
		pb = ui.raw(16, "gpui"),
		items = "end",
		justify = "between",
		ui.column {
			gap = ui.raw(4, "gpui"),
			ui.text { locale.s("tasks.title"), role = "heading", text_size = ui.raw(24, "gpui"), line_height = ui.raw(30, "gpui"), weight = "$font.weight.semibold", color = "$text.primary" },
			ui.text {
				id = "tasks-subtitle",
				function(): string
					local n = #store.parts:get().open
					return if n == 0 then locale.s("tasks.nothing") else locale.s("tasks.to_do_count", { n = n })
				end,
				text_size = ui.raw(13, "gpui"),
				color = "$text.secondary",
			},
		},
		ds.Segmented {
			id = "tasks-view",
			value = M.view,
			label = locale.t("tasks.view"),
			options = { { value = "list", label = locale.t("tasks.list") }, { value = "board", label = locale.t("tasks.kanban") } },
		},
	}
end

local function List(): Node
	return ui.list {
		id = "tasks-list",
		ref = list_ref,
		role = "list",
		label = locale.s("tasks.title"),
		w_full = true,
		grow = 1,
		offset = M.scroll,
		scrollbar = true,
		data = lines,
		key = function(l: Line): string
			return l.key
		end,
		heights = function(): { number }
			local out: { number } = {}
			for i, l in lines:get() do
				out[i] = if l.kind == "done" then DONE_H else (if l.meta then META_H else ROW_H) + ROW_GAP
			end
			return out
		end,
		row = function(l: Line): Node
			if l.kind == "done" then
				return DoneToggle(l.n :: number)
			end
			return ui.div { w_full = true, pb = ui.raw(ROW_GAP, "gpui ROW_GAP"), Row(l.task :: store.Task, l.state :: store.State) }
		end,
	}
end

-- A board card (tasks.rs `render_kanban_card`): the title to 3 lines, New, thumbs, the project or "No project".
local function BoardCard(l: Line): Node
	local t, s = l.task :: store.Task, l.state :: store.State
	return ui.button {
		id = `task-kanban-{t.id}`,
		label = title_of(t),
		w_full = true,
		p = ui.raw(10, "gpui"),
		gap = ui.raw(8, "gpui"),
		direction = "column",
		items = "stretch",
		rounded = ui.raw(12, "gpui"),
		bg = "$zeron.tasks.card",
		border = 1,
		border_color = function(): string
			return if card.current:get() == t.id then "$accent" else "$border"
		end,
		hover = { border_color = "$border.strong" },
		on_click = function()
			open_card(t.id)
		end,
		ui.row { items = "start", min_w = 0, ui.text { title_of(t), literal = true, wrap = true, lines = 3, text_size = ui.raw(13, "gpui"), line_height = ui.raw(18, "gpui"), grow = 1, min_w = 0, color = "$text.primary" }, if store.is_new(t.id) then NewTag() else nil },
		M.Thumbs { task = t, group = `task-kanban-{t.id}` },
		ui.row {
			justify = "between",
			gap = ui.raw(6, "gpui"),
			text_size = ui.raw(11.5, "gpui"),
			color = "$text.secondary",
			ui.text { store.project_name(t.project) or locale.s("tasks.no_project"), literal = true, truncate = true, min_w = 0, shrink = 1 },
			if s.chat then ui.text { store.linked_word(s.chat), color = store.TONES[s.column], shrink = 0 } else nil,
		},
	}
end

-- Every task in its column, split in one pass.
local columns = ui.computed(function(): { [string]: { Line } }
	local out: { [string]: { Line } } = { todo = {}, working = {}, needs = {}, done = {} }
	local p = store.parts:get()
	for _, list in { p.open, p.done } do
		for _, t in list do
			local l = M.line(t)
			table.insert(out[(l.state :: store.State).column], l)
		end
	end
	return out
end)

local function Board(): Node
	local cols: { [any]: any } = { id = "tasks-board", w_full = true, grow = 1, gap = ui.raw(12, "gpui"), items = "stretch", px = ui.raw(12, "gpui") }
	for _, col in store.COLUMNS do
		local tasks = ui.computed(function(): { Line }
			return columns:get()[col]
		end)
		table.insert(cols, ui.column {
			id = `tasks-column-{col}`,
			role = "group",
			label = locale.s(`tasks.column.{col}`),
			grow = 1,
			basis = 0,
			min_w = 180,
			rounded = ui.raw(14, "gpui"),
			bg = "$hover",
			p = ui.raw(10, "gpui"),
			gap = ui.raw(8, "gpui"),
			ui.row {
				gap = ui.raw(6, "gpui"),
				items = "center",
				px = ui.raw(4, "gpui"),
				ui.div { w = 7, h = 7, rounded = "$radius.full", bg = store.TONES[col] },
				ui.text { locale.s(`tasks.column.{col}`), weight = "$font.weight.semibold", text_size = ui.raw(13, "gpui") },
				ui.text { function(): string
					return tostring(#tasks:get())
				end, color = "$text.secondary", text_size = ui.raw(13, "gpui") },
			},
			ui.list {
				w_full = true,
				grow = 1,
				data = tasks,
				measured = true,
				height = 80,
				gap = ui.raw(8, "gpui"),
				key = function(l: Line): string
					return l.key
				end,
				row = function(l: Line): Node
					return ui.div { w_full = true, pb = ui.raw(8, "gpui"), BoardCard(l) }
				end,
			},
		})
	end
	return ui.row(cols)
end

M.Page = ui.component(function(): Node
	ui.on_mount(function()
		store.saw_captured(false)
	end)
	ui.on_cleanup(function()
		store.saw_captured(true)
	end)
	return ui.div {
		id = "tasks-page",
		w_full = true,
		h_full = true,
		relative = true,
		focusable = true,
		label = locale.s("tasks.title"),
		on_key = function(e: any): boolean
			if e.key == "Escape" and card.current:peek() then
				card.close()
				return true
			end
			if e.editing or card.current:peek() then
				return false
			end
			if M.view:peek() == "list" then
				return M.keys(e)
			end
			return false
		end,
		ui.column {
			w_full = true,
			h_full = true,
			pt = ui.raw(28, "gpui"),
			items = "center",
			ui.column {
				w_full = true,
				max_w = function(): number
					return if M.view:get() == "list" then LIST_W else 1100
				end,
				h_full = true,
				px = ui.raw(24, "gpui"),
				Header(),
				ui.presence(function(): string
					return M.view:get()
				end, function(v: string): Node
					if v == "board" then
						return Board()
					end
					return ui.column {
						w_full = true,
						grow = 1,
						min_h = 0,
						M.AddRow { id = "tasks-add", placeholder = locale.s("tasks.add_placeholder") },
						List(),
					}
				end, { w_full = true, grow = 1, min_h = 0 }),
			},
		},
		card.Card {},
		Lightbox.Host(),
	}
end)

return M
