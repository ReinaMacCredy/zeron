-- The Tasks page, trimmed as .maestro/topic/zgx-port/mock/tasks-inbox.html shows (crates/ui tasks.rs).

local ds = require("@ds")
local locale = require("@ds/locale")
local Lightbox = require("@ds/lightbox")
local store = require("@app/screens/tasks/store")
local card = require("@app/screens/tasks/card")
local motion = require("@ds/motion")

local M = {}

local LIST_W, BOARD_W, ROW_H, META_H, ROW_GAP, DONE_H, CHECK = 640, 1100, 44, 46, 2, 36, 18
local THUMB, THUMBS = 28, 3
local DRAWER, EASE_OUT = "cubicBezier(.32,.72,0,1)", "cubicBezier(0,0,.58,1)"
local MORPH, STAGGER, FADE, INCOMING_AT = 320, 22, 176, 96
local COLUMN_DELAY, COLUMN_STAGGER, HEAD_DELAY, HEAD_MS = 40, 30, 120, 256
local EXTRA_AT, EXTRA = 128, 192

local switched_at = -1e9

-- A part arriving with a view switch plays `enter` on its own element (read back mid-way if the switch turns around), starting from `enter`'s first values.
local function arrive(props: { [any]: any }, enter: { [string]: any }?): { [any]: any }
	if not enter or motion.reduced_now() or ui.now() - switched_at > 600 then
		return props
	end
	for k, v in enter do
		if type(v) == "table" then
			props[k] = v[1]
		end
	end
	props.on_mount = function(el: any)
		anime.animate(el, enter)
	end
	return props
end

M.view = ui.persist("tasks_view", "list")
M.done_open = ui.persist("tasks_done_open", false)
M.lit = ui.state("tasks_lit", nil :: string?)
M.scroll = ui.state("tasks_scroll", 0)
M.morphing = ui.state("tasks_morphing", false)
local ghosts = ui.state("tasks_ghosts", {} :: { any })
local row_refs: { [string]: any } = {}
local card_refs: { [string]: any } = {}
local function ref_in(refs: { [string]: any }, id: string): any
	local r = refs[id]
	if not r then
		r = ui.ref()
		refs[id] = r
	end
	return r
end
local function real_opacity(): number
	return if M.morphing:get() then 0 else 1
end

local title_of = store.title_of
M.title_of = title_of

local function sep(): Node
	return ui.text { "·", color = "$text.tertiary", shrink = 0 }
end

-- A row's meta line: project, due, linked thread and its word, Not sent, Chat deleted, Blocked by (tasks.rs `meta`).
function M.meta(t: store.Task, given: store.State?, ghost: boolean?): { Node }
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
			add(ui.text { title_of({ title = s.chat.title } :: any), literal = true, truncate = true, max_w = 220, shrink = 1, min_w = 0, id = if ghost then nil else `task-row-thread-{t.id}` })
			if s.reason == "not_sent" then
				add(ui.text { locale.s("tasks.not_sent"), color = "$warning", shrink = 0 })
			else
				add(ui.text { store.state_word(s), color = store.TONES[s.column], shrink = 0 })
			end
			local progress = store.progress_word(s)
			if progress then
				add(ui.text { progress, id = if ghost then nil else `task-row-progress-{t.id}`, literal = true, tabular = true, shrink = 0 })
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
			add(ui.row { id = if ghost then nil else `task-row-blocked-{t.id}`, gap = ui.raw(5, "gpui"), min_w = 0, items = "center", ui.text { locale.s(if open then "tasks.blocked_by" else "tasks.depends_on"), shrink = 0 }, table.unpack(titles) })
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

function M.Thumbs(props: { task: store.Task, size: number?, group: string, ghost: boolean? }): Node?
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
		local glyph = ds.Icon { name = "gallery", size = 14, color = "$text.tertiary" }
		table.insert(row, ui.div {
			id = if props.ghost then nil else `task-thumb-{props.task.id}-{i}`,
			w = size,
			h = size,
			rounded = ui.raw(6, "gpui"),
			overflow = "hidden",
			bg = "$hover",
			items = "center",
			justify = "center",
			tooltip = if props.ghost then nil else card.missing_tip(path),
			ui.show(function(): boolean
				return store.image_src(path) ~= nil
			end, if props.ghost then card.picture(path, size, 14) else Lightbox.Thumb {
				group = props.group,
				index = i,
				items = items,
				radius = 6,
				w = size,
				h = size,
				card.picture(path, size, 14),
			}, glyph),
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

local function CheckLook(t: store.Task): Node
	local done = t.completed ~= nil
	return ui.div {
		w = CHECK,
		h = CHECK,
		shrink = 0,
		rounded = "$radius.full",
		items = "center",
		justify = "center",
		border = if done then 0 else 1,
		border_color = "$text.tertiary",
		bg = if done then "$accent" else "#00000000",
		if done then ds.Icon { name = "check", size = 12, color = ui.raw("#ffffff", "white on the accent, as gpui") } else nil,
	}
end

-- A list row (tasks.rs `row_content`): the check, the title (struck once done), "New" on a capture, the meta line, the thumbs. A ghost's has no ids, roles or handlers.
local function Row(t: store.Task, s: store.State, ghost: boolean?): Node
	local meta = M.meta(t, s, ghost)
	local done = t.completed ~= nil
	return ui.row {
		id = if ghost then nil else `task-row-{t.id}`,
		role = if ghost then nil else "listitem",
		label = if ghost then nil else title_of(t),
		ref = if ghost then nil else ref_in(row_refs, t.id),
		opacity = if ghost then nil else real_opacity,
		w_full = true,
		h_full = ghost,
		min_h = ROW_H,
		px = ui.raw(12, "gpui"),
		py = ui.raw(6, "gpui"),
		gap = ui.raw(12, "gpui"),
		items = "center",
		rounded = ui.raw(12, "gpui"),
		cursor = if ghost then nil else "pointer",
		bg = function(): string
			if card.current:get() == t.id then
				return "$zeron.tasks.selected"
			end
			return if M.lit:get() == t.id then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = if ghost then nil else { bg = "$hover" },
		on_click = if ghost
			then nil
			else function()
				M.lit:set(t.id)
				open_card(t.id)
			end,
		if ghost then CheckLook(t) else M.Check { task = t, id = `task-check-{t.id}` },
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
		M.Thumbs { task = t, group = `task-row-{t.id}`, ghost = ghost },
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
	local slim = { column = s.column, reason = s.reason, step = s.step, progress = s.progress, gone = s.gone, chat = if c then { id = c.id, title = c.title, status = c.status } else nil }
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

local done_ref, add_ref = ui.ref(), ui.ref()
local function DoneToggle(n: number, ghost: boolean?, enter: { [string]: any }?): Node
	return (if ghost then ui.row else ui.button)(arrive({
		id = if ghost then nil else "tasks-done-toggle",
		ref = if ghost then nil else done_ref,
		label = if ghost then nil else locale.s("tasks.done_count", { n = n }),
		expanded = if ghost
			then nil
			else function(): boolean
				return M.done_open:get()
			end,
		h = DONE_H,
		px = ui.raw(12, "gpui"),
		gap = ui.raw(6, "gpui"),
		items = "center",
		bg = "#00000000",
		hover = if ghost then nil else { bg = "#00000000", color = "$text.primary" },
		color = "$text.secondary",
		text_size = ui.raw(12.5, "gpui"),
		on_click = if ghost
			then nil
			else function()
				M.done_open:set(not M.done_open:peek())
			end,
		ds.Icon { name = function(): string
			return if M.done_open:get() then "alt-arrow-down" else "alt-arrow-right"
		end, size = 12 },
		ui.text { locale.s("tasks.done") },
		ui.text { tostring(n), color = "$text.tertiary" },
	}, enter))
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
function M.AddRow(props: { id: string, placeholder: string, due: (() -> string?)?, on_tab: ((string) -> ())?, ref: any?, enter: { [string]: any }? }): Node
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
	return ui.row(arrive({
		ref = props.ref,
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
	}, props.enter))
end


local function width_of(v: string): number
	return if v == "list" then LIST_W else BOARD_W
end

local header_ref, body_ref = ui.ref(), ui.ref()
local column_refs: { any }, head_refs: { any } = {}, {}
for i in store.COLUMNS do
	column_refs[i], head_refs[i] = ui.ref(), ui.ref()
end

local seen_view = M.view:peek()
ui.effect(function()
	local v = M.view:get()
	if v == seen_view then
		return
	end
	seen_view = v
	switched_at = ui.now()
	local el = header_ref:peek()
	if el then
		anime.animate(el, { maxWidth = width_of(v), duration = if motion.reduced_now() then 0 else MORPH, ease = DRAWER })
	end
end)

local function Header(): Node
	return ui.column {
		ref = header_ref,
		w_full = true,
		max_w = width_of(M.view:peek()),
		px = ui.raw(24, "gpui"),
		ui.row {
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
				value = {
					get = function(_): string
						return M.view:get()
					end,
					peek = function(_): string
						return M.view:peek()
					end,
					set = function(_, v: string)
						M.switch(v)
					end,
				} :: any,
				label = locale.t("tasks.view"),
				options = { { value = "list", label = locale.t("tasks.list") }, { value = "board", label = locale.t("tasks.kanban") } },
			},
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
				return DoneToggle(l.n :: number, false, { opacity = { 0, 1 }, duration = EXTRA, delay = EXTRA_AT, ease = "linear" })
			end
			return ui.div { w_full = true, pb = ui.raw(ROW_GAP, "gpui ROW_GAP"), Row(l.task :: store.Task, l.state :: store.State) }
		end,
	}
end

-- A board card (tasks.rs `render_kanban_card`): the title to 3 lines, New, thumbs, the project or "No project". A ghost's has no ids or handlers.
local function BoardCard(l: Line, ghost: boolean?): Node
	local t, s = l.task :: store.Task, l.state :: store.State
	return (if ghost then ui.column else ui.button) {
		id = if ghost then nil else `task-kanban-{t.id}`,
		label = if ghost then nil else title_of(t),
		ref = if ghost then nil else ref_in(card_refs, t.id),
		opacity = if ghost then nil else real_opacity,
		w_full = true,
		h_full = ghost,
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
		hover = if ghost then nil else { border_color = "$border.strong" },
		on_click = if ghost
			then nil
			else function()
				open_card(t.id)
			end,
		ui.row { items = "start", min_w = 0, ui.text { title_of(t), literal = true, wrap = true, lines = 3, text_size = ui.raw(13, "gpui"), line_height = ui.raw(18, "gpui"), grow = 1, min_w = 0, color = "$text.primary" }, if store.is_new(t.id) then NewTag() else nil },
		M.Thumbs { task = t, group = `task-kanban-{t.id}`, ghost = ghost },
		ui.row {
			justify = "between",
			gap = ui.raw(6, "gpui"),
			text_size = ui.raw(11.5, "gpui"),
			color = "$text.secondary",
			ui.text { store.project_name(t.project) or locale.s("tasks.no_project"), literal = true, truncate = true, min_w = 0, shrink = 1 },
			if s.chat then ui.row {
				gap = ui.raw(6, "gpui"),
				shrink = 0,
				ui.text { store.state_word(s), color = store.TONES[s.column], shrink = 0 },
				if store.progress_word(s) then ui.text { store.progress_word(s) :: string, id = if ghost then nil else `task-kanban-progress-{t.id}`, literal = true, tabular = true, shrink = 0 } else nil,
			} else nil,
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

local function ColumnHead(col: string, count: () -> number, ref: any?, enter: { [string]: any }?): Node
	return ui.row(arrive({
		ref = ref,
		w_full = true,
		gap = ui.raw(6, "gpui"),
		items = "center",
		px = ui.raw(4, "gpui"),
		ui.div { w = 7, h = 7, rounded = "$radius.full", bg = store.TONES[col] },
		ui.text { locale.s(`tasks.column.{col}`), weight = "$font.weight.semibold", text_size = ui.raw(13, "gpui") },
		ui.text { function(): string
			return tostring(count())
		end, color = "$text.secondary", text_size = ui.raw(13, "gpui") },
	}, enter))
end

local function Board(): Node
	local cols: { [any]: any } = { id = "tasks-board", w_full = true, grow = 1, gap = ui.raw(12, "gpui"), items = "stretch", px = ui.raw(12, "gpui") }
	for i, col in store.COLUMNS do
		local tasks = ui.computed(function(): { Line }
			return columns:get()[col]
		end)
		local step = COLUMN_STAGGER * (i - 1)
		table.insert(cols, ui.column(arrive({
			id = `tasks-column-{col}`,
			ref = column_refs[i],
			role = "group",
			label = locale.s(`tasks.column.{col}`),
			grow = 1,
			basis = 0,
			min_w = 180,
			rounded = ui.raw(14, "gpui"),
			bg = "$hover",
			p = ui.raw(10, "gpui"),
			gap = ui.raw(8, "gpui"),
			ColumnHead(col, function(): number
				return #tasks:get()
			end, head_refs[i], { opacity = { 0, 1 }, y = { -6, 0 }, duration = HEAD_MS, delay = HEAD_DELAY + step, ease = DRAWER }),
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
		}, { opacity = { 0, 1 }, y = { 8, 0 }, duration = MORPH, delay = COLUMN_DELAY + step, ease = DRAWER })))
	end
	return ui.row(cols)
end

-- The List/Kanban switch (crates/ui tasks.rs `render_morph`): both views' real tasks hide, and one ghost per task moves from the box it showed in to the one it lands in, the row and the card crossfading inside it.
type Rect = { x: number, y: number, w: number, h: number }
type Ghost = { kind: string, key: string, from: Rect?, to: Rect?, line: Line?, index: number?, to_kanban: boolean?, moving: boolean?, ms: number?, delay: number?, ease: string?, n: number?, col: string?, row_at: number?, card_at: number?, at: number? }

local gen = 0
local ghost_els: { [string]: { box: any, row: any?, card: any? } } = {}

-- Where an element's opacity stands now (mid-fade), so a switch back mid-way starts from what shows.
local function opacity_of(el: any): number?
	if not el then
		return nil
	end
	local ok, v = pcall(anime.utils.get, el, "opacity")
	return if ok then tonumber(v) else nil
end

local function in_body(el: any, origin: any): Rect?
	local r = if el then ui.window_rect(el) else nil
	if not r or r.width <= 0 or r.height <= 0 then
		return nil
	end
	local x, y = r.x - origin.x, r.y - origin.y
	if x + r.width <= 0 or y + r.height <= 0 or x >= origin.width or y >= origin.height then
		return nil
	end
	return { x = x, y = y, w = r.width, h = r.height }
end

local function boxes(refs: { [string]: any }, origin: any, ghosted: boolean): { [string]: Rect }
	local out: { [string]: Rect } = {}
	for id, ref in refs do
		local g = ghost_els[id]
		local r = if ghosted and g then in_body(g.box, origin) else nil
		out[id] = r or in_body(ref:peek(), origin)
	end
	return out
end

local function morph_end(tasks: number): number
	local last_task, last_column = math.max(tasks - 1, 0) * STAGGER, (#store.COLUMNS - 1) * COLUMN_STAGGER
	return math.max(last_task + INCOMING_AT + FADE, last_task + MORPH, COLUMN_DELAY + last_column + MORPH, HEAD_DELAY + last_column + HEAD_MS, EXTRA_AT + EXTRA)
end

local function settle(g: number)
	if g ~= gen then
		return
	end
	table.clear(ghost_els)
	ui.batch(function()
		ghosts:set({})
		M.morphing:set(false)
	end)
end

local function in_order(): { Line }
	local p = store.parts:peek()
	local out: { Line } = {}
	for _, list in { p.open, p.done } do
		for _, t in list do
			table.insert(out, M.line(t))
		end
	end
	return out
end

function M.switch(v: string)
	local was = M.view:peek()
	if v == was then
		return
	end
	gen += 1
	local g = gen
	local body = body_ref:peek()
	local origin = if body then ui.window_rect(body) else nil
	if motion.reduced_now() or not origin then
		table.clear(ghost_els)
		ui.batch(function()
			ghosts:set({})
			M.morphing:set(false)
			M.view:set(v)
		end)
		return
	end
	local to_kanban = v == "board"
	local from = boxes(if was == "list" then row_refs else card_refs, origin, true)
	local seen: { [string]: { row: number?, card: number? } } = {}
	for id, els in ghost_els do
		seen[id] = { row = opacity_of(els.row), card = opacity_of(els.card) }
	end
	table.clear(ghost_els)
	local extras: { Ghost } = {}
	local function extra(e: Ghost, r: Rect?)
		if r then
			e.from = r
			table.insert(extras, e)
		end
		return r
	end
	if to_kanban then
		extra({ kind = "add", key = `{g}-add`, ms = EXTRA, delay = 0, ease = "linear", at = opacity_of(add_ref:peek()) }, in_body(add_ref:peek(), origin))
		extra({ kind = "done", key = `{g}-done`, n = #store.parts:peek().done, ms = EXTRA, delay = 0, ease = "linear", at = opacity_of(done_ref:peek()) }, in_body(done_ref:peek(), origin))
	else
		local cols = columns:peek()
		for i, col in store.COLUMNS do
			local step = COLUMN_STAGGER * (i - 1)
			extra({ kind = "column", key = `{g}-column-{i}`, ms = MORPH, delay = step, ease = DRAWER, at = opacity_of(column_refs[i]:peek()) }, in_body(column_refs[i]:peek(), origin))
			extra({ kind = "head", key = `{g}-head-{i}`, col = col, n = #cols[col], ms = HEAD_MS, delay = step, ease = DRAWER, at = opacity_of(head_refs[i]:peek()) }, in_body(head_refs[i]:peek(), origin))
		end
	end
	local order = in_order()
	local first = table.clone(extras)
	for _, l in order do
		local f = from[l.key]
		if f then
			local at = seen[l.key] or {}
			table.insert(first, { kind = "task", key = `{g}-held-{l.key}`, line = l, from = f, to_kanban = to_kanban, row_at = at.row, card_at = at.card })
		end
	end
	switched_at = ui.now()
	ui.batch(function()
		M.morphing:set(true)
		ghosts:set(first)
		M.view:set(v)
	end)
	local tries = 0
	local function land()
		if g ~= gen then
			return
		end
		local to = boxes(if to_kanban then card_refs else row_refs, origin, false)
		local p = store.parts:peek()
		local expected = #p.open > 0 or (#p.done > 0 and (to_kanban or M.done_open:peek()))
		if next(to) == nil and expected and tries < 10 then
			tries += 1
			ui.after(16, land)
			return
		end
		local all = table.clone(extras)
		local n = 0
		for _, l in order do
			local f, t = from[l.key], to[l.key]
			if f or t then
				local at = seen[l.key] or {}
				table.insert(all, { kind = "task", key = `{g}-{l.key}`, line = l, from = f, to = t, index = n, to_kanban = to_kanban, moving = true, row_at = at.row, card_at = at.card })
				n += 1
			end
		end
		ghosts:set(all)
		ui.after(morph_end(n), function()
			settle(g)
		end)
	end
	ui.after(0, land)
end

local function AddLook(): Node
	return ui.row {
		w_full = true,
		h_full = true,
		px = ui.raw(12, "gpui"),
		gap = ui.raw(12, "gpui"),
		items = "center",
		ui.div { w = CHECK, h = CHECK, rounded = "$radius.full", border = 1, border_style = "dotted", border_color = "$text.tertiary", shrink = 0 },
		ui.text { locale.s("tasks.add_placeholder"), color = "$text.tertiary", text_size = ui.raw(13.5, "gpui"), truncate = true, min_w = 0 },
	}
end

local function GhostView(e: Ghost): Node
	local r = (e.from or e.to) :: Rect
	local place = { absolute = true, left = r.x, top = r.y, w = r.w, h = r.h }
	if e.kind ~= "task" then
		local look = if e.kind == "add"
			then AddLook()
			elseif e.kind == "done" then DoneToggle(e.n :: number, true)
			elseif e.kind == "head" then ColumnHead(e.col :: string, function(): number
				return e.n :: number
			end)
			else ui.div { w_full = true, h_full = true, rounded = ui.raw(14, "gpui"), bg = "$hover" }
		place.on_mount = function(el: any)
			anime.animate(el, { opacity = { e.at or 1, 0 }, duration = e.ms, delay = e.delay, ease = e.ease })
		end
		place.opacity = e.at
		table.insert(place, look)
		return ui.div(place)
	end
	local l = e.line :: Line
	local t = l.task :: store.Task
	local delay = (e.index or 0) * STAGGER
	local els = { box = nil :: any, row = nil :: any, card = nil :: any }
	ghost_els[t.id] = els
	local function look(row: boolean): Node?
		local leaving = row == e.to_kanban
		local at = (if row then e.row_at else e.card_at) or (if leaving then 1 else 0)
		if (leaving and not e.from) or (not leaving and not e.to and at <= 0) then
			return nil
		end
		return ui.div {
			absolute = true,
			left = 0,
			top = 0,
			right = 0,
			bottom = 0,
			opacity = at,
			on_mount = function(el: any)
				if row then
					els.row = el
				else
					els.card = el
				end
				if not e.moving then
					return
				end
				if leaving or not e.to then
					anime.animate(el, { opacity = { at, 0 }, duration = FADE, delay = delay, ease = EASE_OUT })
				else
					anime.animate(el, { opacity = { at, 1 }, duration = FADE, delay = delay + INCOMING_AT, ease = EASE_OUT })
				end
			end,
			if row then Row(t, l.state :: store.State, true) else BoardCard(l, true),
		}
	end
	place.rounded = ui.raw(10, "gpui")
	place.overflow = "hidden"
	place.on_mount = function(el: any)
		els.box = el
		local from, to = e.from, e.to
		if e.moving and from and to then
			anime.animate(el, { left = { from.x, to.x }, top = { from.y, to.y }, w = { from.w, to.w }, h = { from.h, to.h }, duration = MORPH, delay = delay, ease = DRAWER })
		end
	end
	table.insert(place, look(true))
	table.insert(place, look(false))
	return ui.div(place)
end

local function Ghosts(): Node
	return ui.div {
		absolute = true,
		left = 0,
		top = 0,
		right = 0,
		bottom = 0,
		pointer_events = "none",
		overflow = "hidden",
		ui.div {
			relative = true,
			w_full = true,
			h_full = true,
			ui.each(ghosts, function(e: Ghost): Node
				return GhostView(e)
			end, {
				key = function(e: Ghost): string
					return e.key
				end,
			}),
		},
	}
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
			Header(),
			ui.column {
				ref = body_ref,
				w_full = true,
				grow = 1,
				min_h = 0,
				relative = true,
				items = "center",
				ui.column {
					w_full = true,
					max_w = function(): number
						return width_of(M.view:get())
					end,
					h_full = true,
					px = ui.raw(24, "gpui"),
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
							M.AddRow { id = "tasks-add", placeholder = locale.s("tasks.add_placeholder"), ref = add_ref, enter = { opacity = { 0, 1 }, duration = EXTRA, delay = EXTRA_AT, ease = "linear" } },
							List(),
						}
					end, { w_full = true, grow = 1, min_h = 0 }),
				},
				Ghosts(),
			},
		},
		card.Card {},
		Lightbox.Host(),
	}
end)

return M
