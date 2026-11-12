-- The task card, trimmed as the U26 mockup shows (crates/ui tasks/card.rs); nothing opens unless asked.

local ds = require("@ds")
local locale = require("@ds/locale")
local Menu = require("@ds/menu")
local Lightbox = require("@ds/lightbox")
local motion = require("@ds/motion")
local glass = require("@ds/glass")
local sessions = require("@account/sessions")
local store = require("@app/screens/tasks/store")
local route = require("@app/screens/shell/route")
local chat_store = require("@app/screens/chat/store")
local references = require("@ds/references")
local field_kit = require("@app/screens/tasks/field")
local add_project = require("@app/screens/shell/add_project")

local M = {}

local W, RADIUS, SAVE_MS, THUMB = 324, 24, 500, 56
local DRAWER, SHARP = "cubicBezier(.32,.72,0,1)", "cubicBezier(.4,0,1,1)"

M.current = ui.state("tasks_card", nil :: string?)
M.focus = ui.state("tasks_card_focus", "title")

function M.open(id: string, focus: string?)
	ui.batch(function()
		if M.current:peek() ~= id then
			M.run_in:set(nil)
			M.plan_first:set(false)
			M.hand_off:set("new")
		end
		M.focus:set(focus or "title")
		M.current:set(id)
	end)
end

local flushers: { () -> () } = {}
function M.close()
	for _, f in flushers do
		f()
	end
	M.current:set(nil)
end

local title_of = store.title_of

-- (A press anywhere outside the card closes it, as the shell's other floating cards; the card's own presses mark themselves first.)
local pressed_in = false
function M.press_outside()
	if pressed_in then
		pressed_in = false
		return
	end
	if M.current:peek() then
		M.close()
	end
end

-- A field that saves `key` as typed, a pause after the last key.
local function saved(t: store.Task, key: string): (State<string>, () -> (), string)
	local refs = `task:{t.id}:{key}`
	local value = ui.use_state(field_kit.shown(refs, (t :: any)[key] or ""))
	local seq = 0
	local pending = false
	local function flush()
		if pending then
			pending = false
			local v = value:peek()
			if key ~= "title" or not string.match(v, "^%s*$") then
				store.update(t.id, { [key] = references.encode(refs, v) })
			end
		end
	end
	local function typed(v: string)
		value:set(v)
		seq += 1
		pending = true
		local mine = seq
		ui.after(SAVE_MS, function()
			if mine == seq then
				flush()
			end
		end)
	end
	table.insert(flushers, flush)
	ui.on_cleanup(function()
		flush()
		local at = table.find(flushers, flush)
		if at then
			table.remove(flushers, at)
		end
	end)
	return value, typed :: any, refs
end

local function Pick(props: { id: string, label: string, icon: string, value: () -> string, options: () -> { any }, on_pick: (string) -> (), w: number, quiet: boolean? }): Node
	local open = ui.use_state(false)
	local function shown(): string
		local v = props.value()
		for _, o in props.options() do
			if o.value == v then
				return o.label
			end
		end
		return v
	end
	local trigger = ui.button {
		id = props.id,
		label = props.label,
		h = if props.quiet then 32 else 28,
		max_w = if props.quiet then 170 else nil,
		min_w = 0,
		pl = ui.raw(10, "gpui"),
		pr = ui.raw(8, "gpui"),
		gap = ui.raw(8, "gpui"),
		items = "center",
		rounded = if props.quiet then ui.raw(8, "gpui") else "$radius.full",
		bg = function(): string
			if open:get() then
				return "$zeron.tasks.chip_hover"
			end
			return if props.quiet then "#00000000" else "$zeron.tasks.chip"
		end,
		hover = { bg = "$zeron.tasks.chip_hover", color = "$text.primary" },
		color = if props.quiet then "$text.secondary" else "$text.primary",
		text_size = ui.raw(12.5, "gpui"),
		expanded = function(): boolean
			return open:get()
		end,
		on_click = function()
			open:set(not open:peek())
		end,
		ds.Icon { name = props.icon, size = 14, color = "$text.secondary" },
		ui.text { shown, literal = true, truncate = true, min_w = 0, shrink = 1 },
		ds.Icon { name = "alt-arrow-down", size = 14, color = function(): string
			return if open:get() then "$text.primary" else "$text.secondary"
		end },
	}
	return ui.row {
		min_w = 0,
		shrink = 1,
		ui.popover(Menu.panel {
			open = open,
			anchor = trigger,
			w = props.w,
			placement = { side = "bottom", align = "start", offset = 6 },
			build = function(): Node
				local rows = {}
				for _, o in props.options() do
					table.insert(rows, Menu.Row {
						id = `{props.id}-{o.value}`,
						icon = props.icon,
						label = o.label,
						checked = function(): boolean
							return props.value() == o.value
						end,
						on_click = function()
							open:set(false)
							props.on_pick(o.value)
						end,
					})
				end
				return Menu.Group(rows)
			end,
		}),
	}
end

local function due_value(t: store.Task): string
	if t.due == nil then
		return ""
	elseif t.due == store.today() then
		return "today"
	elseif t.due == store.tomorrow() then
		return "tomorrow"
	end
	return t.due
end

-- A task's picture in its thumb; one that can't be drawn (gone, or not a picture) is the gallery glyph and says so (tasks/images.rs `strip`).
function M.picture(path: string, size: number, glyph: number, h: number?): Node
	return ui.show(function(): boolean
		return not store.image_failed(path)
	end, ui.image {
		src = function(): string
			return store.image_src(path) or ""
		end,
		w = size,
		h = h or size,
		fit = "cover",
		aria_hidden = true,
		on_error = function()
			store.image_broke(path)
		end,
	}, ui.div { w = size, h = h or size, items = "center", justify = "center", ds.Icon { name = "gallery", size = glyph, color = "$text.tertiary" } })
end
function M.missing_tip(path: string): Node
	return ui.show(function(): boolean
		return store.image_failed(path)
	end, ds.Tooltip { function(): string
		return locale.s("tasks.image_missing")
	end })
end

function M.thumbs(list: { string }, prefix: string, group: string, on_remove: (i: number) -> (), size: { w: number, h: number, radius: number }?): { Node }
	local tw, th, tr = if size then size.w else THUMB, if size then size.h else THUMB, if size then size.radius else 10
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
	local out: { Node } = {}
	for i, path in list do
		local over = ui.use_state(false)
		table.insert(out, ui.div {
			id = `{prefix}-image-{i}`,
			relative = true,
			w = tw,
			h = th,
			rounded = ui.raw(tr, "gpui"),
			overflow = "hidden",
			bg = "$hover",
			items = "center",
			justify = "center",
			on_hover = function(inside: boolean)
				over:set(inside)
			end,
			tooltip = M.missing_tip(path),
			ui.show(function(): boolean
				return store.image_src(path) ~= nil
			end, Lightbox.Thumb { group = group, index = i, items = items, radius = tr, w = tw, h = th, M.picture(path, tw, 16, th) }, ds.Icon { name = "gallery", size = 16, color = "$text.tertiary" }),
			ui.show(function(): boolean
				return over:get()
			end, ui.button {
				id = `{prefix}-remove-image-{i}`,
				label = locale.s("tasks.card.remove_image"),
				absolute = true,
				top = 3,
				right = 3,
				w = 18,
				h = 18,
				p = 0,
				rounded = "$radius.full",
				items = "center",
				justify = "center",
				bg = "$zeron.tasks.remove",
				hover = { bg = "$zeron.tasks.remove_hover" },
				on_click = function()
					on_remove(i)
				end,
				ds.Icon { name = "close", size = 10, color = ui.raw("#ffffff", "white on the accent, as gpui") },
			}),
		})
	end
	return out
end

local function Images(t: store.Task): Node
	local list = t.images or {}
	local row: { [any]: any } = { id = "task-card-images", gap = ui.raw(8, "gpui"), wrap = true, pt = ui.raw(6, "gpui") }
	for _, thumb in M.thumbs(list, "task-card", `task-card-{t.id}`, function(i: number)
		local next = table.clone(list)
		table.remove(next, i)
		store.update(t.id, { images = next })
	end, { w = 72, h = 52, radius = 9 }) do
		table.insert(row, thumb)
	end
	return ui.row(row)
end

function M.add_files(id: string, files: { any })
	local t0 = store.find(id)
	if not t0 then
		return
	end
	local t = t0 :: store.Task
	do
		local next = table.clone(t.images or {})
		local left = #files
		if left == 0 then
			return
		end
		local function one_done()
			left -= 1
			if left == 0 then
				if #next > store.IMAGES_MAX then
					ds.toast.show({ title = locale.s("tasks.card.images_max", { n = store.IMAGES_MAX }), tone = "warning" })
				end
				while #next > store.IMAGES_MAX do
					table.remove(next)
				end
				store.update(t.id, { images = next })
			end
		end
		for _, f in files do
			local p = store.host.upload({ path = f.path })
			if p then
				p:after(function(path: string)
					table.insert(next, path)
					one_done()
				end, function()
					ds.toast.show({ title = locale.s("tasks.card.image_failed", { name = f.name or f.path }), tone = "danger" })
					one_done()
				end)
			else
				one_done()
			end
		end
	end
end

function M.add_images(t: store.Task)
	ui.open_files({
		title = locale.s("tasks.add_image"),
		multiple = true,
		filters = { { locale.s("tasks.card.images"), { "png", "jpg", "jpeg", "webp", "gif" } } },
	}):after(function(files: { any })
		M.add_files(t.id, files)
	end)
end

-- The agent a task starts on: the card's own pick (gpui's detached `Pickers::new_for_tasks`), first the new chat composer's, never written back to it.
local function defaults(): { [string]: any }
	local d = chat_store.new_defaults
	return if d then d:get() else {}
end
local picked_model = ui.state("tasks_card_model", nil :: string?)
local picked_effort = ui.state("tasks_card_effort", nil :: string?)
local function new_chat_model(): any
	return chat_store.new_chat_model(picked_model:get())
end

M.starting = ui.state("tasks_card_starting", nil :: string?)
M.plan_first = ui.state("tasks_card_plan_first", false)
M.run_in = ui.state("tasks_card_run_in", nil :: string?)

local function open_when_landed(chat: string)
	local tries = 0
	local function look()
		for _, c in sessions.now:peek().chats or {} do
			if c.id == chat then
				M.close()
				route.open_session(chat)
				return
			end
		end
		tries += 1
		if tries < 100 then
			ui.after(100, look)
		end
	end
	look()
end

function M.start(t: store.Task)
	local m = new_chat_model()
	if not m or M.starting:peek() == t.id then
		return
	end
	M.starting:set(t.id)
	local function over()
		if M.starting:peek() == t.id then
			M.starting:set(nil)
		end
	end
	local effort = if picked_model:peek() == m.value then picked_effort:peek() else defaults().effort
	local project = M.run_in:peek()
	if project == nil then
		project = t.project
	end
	store.start(t.id, { model = m.value, effort = effort, access = defaults().access }, if project == "" then nil else project, M.plan_first:peek(), function(chat: any)
		over()
		if type(chat) == "string" then
			open_when_landed(chat)
		end
	end, over)
end

local function Pill(props: { [string]: any }): Node
	props.h = 28
	props.min_w = 28
	props.px = ui.raw(9, "gpui")
	props.rounded = "$radius.full"
	props.bg = "$zeron.tasks.chip"
	props.hover = { bg = "$zeron.tasks.chip_hover", color = "$text.primary" }
	props.items = "center"
	props.justify = "center"
	props.gap = ui.raw(6, "gpui")
	props.color = "$text.secondary"
	props.text_size = ui.raw(12.5, "gpui")
	return ui.button(props)
end

local function RunIn(t: store.Task): Node
	if M.run_in:peek() == nil then
		M.run_in:set(t.project or "")
	end
	return ui.row {
		id = "task-card-run-in",
		h = 32,
		gap = ui.raw(2, "gpui"),
		items = "center",
		text_size = ui.raw(12.5, "gpui"),
		color = "$text.secondary",
		ui.text { locale.s("tasks.card.run_in"), pl = ui.raw(2, "gpui") },
		Pick {
			id = "task-card-run-in-select",
			label = locale.s("tasks.card.run_in_name"),
			icon = "folder",
			w = 220,
			quiet = true,
			value = function(): string
				return M.run_in:get() or ""
			end,
			options = function(): { any }
				local now = sessions.now:get()
				local out = { { value = "", label = locale.s("tasks.no_project") } }
				for _, p in now.projects or {} do
					if p.device == nil or now.here == nil or p.device == now.here then
						table.insert(out, { value = p.id, label = p.name })
					end
				end
				return out
			end,
			on_pick = function(v: string)
				M.run_in:set(v)
			end,
		},
		ui.button {
			id = "task-card-folder",
			label = locale.s("tasks.card.folder"),
			px = ui.raw(8, "gpui"),
			py = ui.raw(4, "gpui"),
			rounded = ui.raw(8, "gpui"),
			bg = "#00000000",
			hover = { bg = "$hover", color = "$text.primary" },
			color = "$text.secondary",
			on_click = function()
				add_project.open()
			end,
			ui.text { locale.s("tasks.card.folder") },
		},
	}
end

local function PlanFirst(): Node
	return ui.row {
		id = "task-card-plan-first-row",
		gap = ui.raw(10, "gpui"),
		items = "start",
		ui.column {
			grow = 1,
			min_w = 0,
			gap = ui.raw(2, "gpui"),
			ui.text { locale.s("tasks.card.plan_first"), text_size = ui.raw(12.5, "gpui"), color = "$text.primary" },
			ui.text { locale.s("tasks.card.plan_first_help"), text_size = ui.raw(12, "gpui"), color = "$text.secondary", wrap = true },
		},
		ui.div { id = "task-card-plan-first", ds.Switch { value = M.plan_first, label = locale.s("tasks.card.plan_first") } },
	}
end

local function NewChat(t: store.Task): Node
	local function starting(): boolean
		return M.starting:get() == t.id
	end
	local function start()
		M.start(t)
	end
	ui.effect(function()
		if picked_model:get() ~= nil then
			return
		end
		local m = new_chat_model()
		if m and (m.value == defaults().model or chat_store.catalog:get().complete == true) then
			ui.batch(function()
				picked_model:set(m.value)
				picked_effort:set(defaults().effort)
			end)
		end
	end)
	return ui.column {
		gap = ui.raw(8, "gpui"),
		ui.column {
			gap = ui.raw(2, "gpui"),
			ui.row {
				id = "task-card-agent",
				ml = ui.raw(-6, "gpui"),
				min_w = 0,
				h = 32,
				items = "center",
				ds.ModelPicker {
					model = picked_model,
					effort = picked_effort,
					harnesses = function(): { any }
						local c: any = chat_store.catalog:get()
						return c.tabs or c.harnesses or {}
					end,
					models = function(): { any }
						return chat_store.catalog:get().models or {}
					end,
					looking = function(): boolean
						return chat_store.catalog:get().complete ~= true
					end,
					placement = { side = "bottom", align = "start", offset = 6 },
				},
			},
			RunIn(t),
		},
		PlanFirst(),
		ui.row {
			pt = ui.raw(6, "gpui"),
			justify = "end",
			gap = ui.raw(6, "gpui"),
			ui.button {
				id = "task-card-start",
				label = function(): string
					return locale.s(if starting() then "tasks.card.starting" else "tasks.card.start")
				end,
				min_h = 30,
				px = ui.raw(14, "gpui"),
				py = ui.raw(5, "gpui"),
				gap = ui.raw(6, "gpui"),
				rounded = "$radius.full",
				items = "center",
				bg = "$text.primary",
				color = "$background",
				hover = { opacity = 0.9 },
				text_size = ui.raw(12.5, "gpui"),
				weight = "$font.weight.medium",
				disabled = function(): boolean
					return new_chat_model() == nil or starting()
				end,
				opacity = function(): number
					return if new_chat_model() == nil or starting() then 0.4 else 1
				end,
				on_click = start,
				ui.text { function(): string
					return locale.s(if starting() then "tasks.card.starting" else "tasks.card.start")
				end },
				ui.text { ui.shortcut_text("cmd+enter"), text_size = ui.raw(11, "gpui"), opacity = 0.5 },
			},
		},
	}
end

-- The threads a task can go to, as gpui's `thread_matches`: this project's first, six at most.
local function thread_matches(chats: { any }, query: string, device: string?, project: string?, projects: { [string]: string }): { any }
	local q = string.lower(string.match(query or "", "^%s*(.-)%s*$") or "")
	local own, other = {}, {}
	for _, c in chats do
		if device == nil or c.device == device then
			local title = string.match(string.gsub(c.title or "", "%s+", " "), "^%s*(.-)%s*$") or ""
			if title == "" then
				title = ui.t("chat.completions.new_session"):get()
			end
			if q == "" or string.find(string.lower(title), q, 1, true) or string.sub(string.lower(c.id), 1, #q) == q then
				table.insert(if c.project == project then own else other, { c = c, title = title })
			end
		end
	end
	for _, x in other do
		table.insert(own, x)
	end
	local out: { any } = {}
	for i, x in own do
		if i > 6 then
			break
		end
		local c = x.c
		table.insert(out, {
			label = x.title,
			detail = if c.project then projects[c.project] else nil,
			thread = { id = c.id, mark = c.mark or c.agent, icon = c.agent_icon, status = c.status },
		})
	end
	return out
end

local function Thread(t: store.Task): Node
	local query = ui.use_state("")
	local picked = ui.use_state(nil :: any)
	local function matches(): { any }
		local now = sessions.now:get()
		local names: { [string]: string } = {}
		for _, p in now.projects or {} do
			names[p.id] = p.name
		end
		local q = string.gsub(query:get(), "^@", "")
		return thread_matches(now.chats, q, now.here, t.project, names)
	end
	local function send(yes: boolean)
		local p = picked:peek()
		if not p then
			return
		end
		store.assign(t.id, p.thread.id, yes)
	end
	return ui.column {
		gap = ui.raw(8, "gpui"),
		ui.show(function(): boolean
			return picked:get() == nil
		end, ui.column {
			gap = ui.raw(4, "gpui"),
			ui.input {
				id = "task-card-assign",
				label = locale.s("tasks.card.assign"),
				placeholder = locale.s("tasks.card.assign_placeholder"),
				placeholder_color = "$text.tertiary",
				value = function(): string
					return query:get()
				end,
				on_change = function(v: string)
					query:set(v)
				end,
				on_key = function(e: any): boolean
					if e.key == "Enter" then
						local first = matches()[1]
						if first then
							picked:set(first)
						end
						return true
					end
					return false
				end,
				w_full = true,
				h = 30,
				px = ui.raw(10, "gpui"),
				rounded = ui.raw(10, "gpui"),
				border = 0,
				bg = "$hover",
				text_size = ui.raw(12.5, "gpui"),
			},
			ui.column {
				id = "task-card-matches",
				ui.each(ui.computed(function(): { any }
					return if query:get() == "" then {} else matches()
				end), function(m: any): Node
					return ui.button {
						id = `task-card-match-{m.thread.id}`,
						label = m.label,
						w_full = true,
						h = 28,
						px = ui.raw(8, "gpui"),
						rounded = ui.raw(8, "gpui"),
						bg = "#00000000",
						hover = { bg = "$hover" },
						justify = "start",
						text_size = ui.raw(12.5, "gpui"),
						on_click = function()
							picked:set(m)
						end,
						ui.text { m.label, literal = true, truncate = true },
					}
				end),
			},
			ui.text { locale.s("tasks.card.assign_hint"), text_size = ui.raw(11.5, "gpui"), color = "$text.secondary" },
		}, ui.row {
			id = "task-card-assign-picked",
			h = 30,
			px = ui.raw(10, "gpui"),
			gap = ui.raw(6, "gpui"),
			rounded = ui.raw(10, "gpui"),
			bg = "$hover",
			items = "center",
			ui.text { function(): string
				local p = picked:get()
				return if p then p.label else ""
			end, literal = true, truncate = true, grow = 1, min_w = 0, text_size = ui.raw(12.5, "gpui") },
			ui.button {
				id = "task-card-assign-clear",
				label = locale.s("tasks.card.clear"),
				p = ui.raw(2, "gpui"),
				bg = "#00000000",
				on_click = function()
					picked:set(nil)
				end,
				ds.Icon { name = "close", size = 12 },
			},
		}),
		ui.row {
			pt = ui.raw(6, "gpui"),
			justify = "end",
			gap = ui.raw(6, "gpui"),
			ds.Button {
				locale.t("tasks.card.link_only"),
				id = "task-card-link", size = "sm", disabled = function(): boolean
				return picked:get() == nil
			end, on_click = function()
				send(false)
			end },
			ds.Button {
				locale.t("tasks.card.send"),
				id = "task-card-send", tone = "primary", size = "sm", disabled = function(): boolean
				return picked:get() == nil
			end, on_click = function()
				send(true)
			end },
		},
	}
end

M.hand_off = ui.state("tasks_card_hand_off", "new")
local hand_off = M.hand_off

local function HandOff(t: store.Task): Node
	return ui.column {
		gap = ui.raw(8, "gpui"),
		ui.text { locale.s("tasks.card.hand_it_to"), text_size = ui.raw(12.5, "gpui"), color = "$text.secondary" },
		ds.Segmented { id = "task-card-hand-off", fill = true, value = hand_off, label = locale.t("tasks.card.hand_it_to"), options = { { value = "new", label = locale.t("tasks.card.new_chat") }, { value = "thread", label = locale.t("tasks.card.thread") } } },
		ui.presence(function(): string
			return hand_off:get()
		end, function(v: string): Node
			return if v == "thread" then Thread(t) else NewChat(t)
		end, { w_full = true }),
	}
end

local function Linked(t: store.Task): Node
	local c = store.state(t).chat
	local function now(): store.State
		return store.state(store.find(t.id) or t)
	end
	return ui.column {
		id = "task-card-linked",
		gap = ui.raw(8, "gpui"),
		p = ui.raw(10, "gpui"),
		rounded = ui.raw(12, "gpui"),
		bg = "$hover",
		ui.text { function(): string
			local l = now().chat
			return if l then l.title else locale.s("tasks.chat_deleted")
		end, literal = true, weight = "$font.weight.semibold", truncate = true },
		if c then ui.text { id = "task-card-status", function(): string
			return store.linked_word(now().chat)
		end, color = function(): string
			return store.TONES[now().column]
		end, text_size = ui.raw(12, "gpui") } else nil,
		if t.send_error then ui.row {
			gap = ui.raw(8, "gpui"),
			items = "center",
			ui.text { locale.s("tasks.card.not_sent", { error = t.send_error }), literal = true, color = "$warning", wrap = true, grow = 1, min_w = 0, text_size = ui.raw(12, "gpui") },
			ds.Button {
				locale.t("tasks.card.retry"),
				id = "task-card-retry-send", size = "sm", tone = "primary", on_click = function()
				if t.chat then
					store.assign(t.id, t.chat, true)
				end
			end },
		} else nil,
		ui.row {
			gap = ui.raw(8, "gpui"),
			if c then ds.Button {
				locale.t("tasks.card.open_thread"),
				id = "task-card-open-thread", size = "sm", on_click = function()
				M.close()
				route.open_session(c.id)
			end } else nil,
			ds.Button {
				locale.t("tasks.card.unassign"),
				id = "task-card-unassign", size = "sm", tone = "ghost", on_click = function()
				store.assign(t.id, "", false)
			end },
		},
	}
end

local function Done(t: store.Task): Node
	local c = store.chat_of(t.chat)
	return ui.row {
		justify = "end",
		gap = ui.raw(6, "gpui"),
		if c then ds.Button {
				locale.t("tasks.card.open_chat"),
				id = "task-card-open-chat", size = "sm", tone = "primary", on_click = function()
			M.close()
			route.open_session(c.id)
		end } else nil,
		ds.Button {
				locale.t("tasks.mark_not_done"),
				id = "task-card-reopen", size = "sm", on_click = function()
			store.set_done(t.id, false)
		end },
	}
end

local function Body(id: string): Node
	local t0 = store.find(id)
	if not t0 then
		return ui.div {}
	end
	local t = t0 :: store.Task
	local title, type_title, title_refs = saved(t, "title")
	local notes, type_notes, notes_refs = saved(t, "notes")
	local menu = ui.use_state(false)
	local title_ref, notes_ref = ui.ref(), ui.ref()
	ui.after(0, function()
		local el = (if M.focus:peek() == "notes" then notes_ref else title_ref):peek()
		if el then
			el:focus()
		end
	end)
	ui.effect(function()
		type_title(title:get())
	end)
	ui.effect(function()
		type_notes(notes:get())
	end)
	local function live(): store.Task
		return store.find(id) or t
	end
	local function on_files(files: { any })
		M.add_files(id, files)
	end
	local title_field = field_kit.use {
		key = "tasks_card_title",
		refs = function(): string
			return title_refs
		end,
		text = title,
		field = title_ref,
		line = 22,
		on_files = on_files,
	}
	local notes_field = field_kit.use {
		key = "tasks_card_notes",
		refs = function(): string
			return notes_refs
		end,
		text = notes,
		field = notes_ref,
		line = 20,
		on_files = on_files,
	}
	local function icon_button(props: { [string]: any }, icon: string): Node
		props.w = 24
		props.h = 24
		props.p = 0
		props.shrink = 0
		props.rounded = ui.raw(6, "gpui")
		props.bg = "#00000000"
		props.hover = { bg = "$hover" }
		props.items = "center"
		props.justify = "center"
		table.insert(props, ds.Icon { name = icon, size = 14, color = "$text.secondary" })
		return ui.button(props)
	end
	local more = icon_button({
		id = "task-card-menu",
		label = locale.s("tasks.card.more"),
		expanded = function(): boolean
			return menu:get()
		end,
		on_click = function()
			menu:set(not menu:peek())
		end,
	}, "more-horizontal")
	return ui.column {
		w_full = true,
		gap = ui.raw(14, "gpui"),
		ui.column {
			w_full = true,
			gap = ui.raw(4, "gpui"),
			ui.row {
				w_full = true,
				items = "start",
				gap = ui.raw(4, "gpui"),
				ui.div {
					grow = 1,
					min_w = 0,
					title_field.textarea {
						id = "task-card-title",
						label = locale.s("tasks.card.title"),
						placeholder = locale.s("tasks.card.title"),
						placeholder_color = "$text.tertiary",
						on_key = function(e: any): boolean
							if e.key == "Enter" and not (e.meta or e.cmd or e.ctrl) then
								local el = notes_ref:peek()
								if el then
									el:focus()
								end
								return true
							end
							return false
						end,
						wrap = false,
						w_full = true,
						h = 22,
						px = 0,
						py = 0,
						border = 0,
						bg = "#00000000",
						focus = { border = 0, outline = 0 },
						text_size = ui.raw(15, "gpui"),
						weight = "$font.weight.medium",
					},
				},
				ui.popover(Menu.panel {
					open = menu,
					anchor = more,
					w = 170,
					placement = { side = "bottom", align = "end", offset = 6 },
					build = function(): Node
						return Menu.Group {
							Menu.Row { id = "task-card-toggle-done", icon = "check-circle", label = locale.s(if live().completed then "tasks.mark_not_done" else "tasks.mark_done"), on_click = function()
								menu:set(false)
								store.set_done(id, live().completed == nil)
							end },
							Menu.Row { id = "task-card-delete", icon = "trash-bin-minimalistic", label = locale.s("tasks.card.delete"), danger = true, on_click = function()
								menu:set(false)
								store.delete(id)
								M.current:set(nil)
							end },
						}
					end,
				}),
				icon_button({ id = "task-card-close", label = locale.s("tasks.card.close"), on_click = M.close }, "close"),
			},
			notes_field.textarea {
				id = "task-card-notes",
				label = locale.s("tasks.card.notes"),
				placeholder = locale.s("tasks.card.notes"),
				placeholder_color = "$text.tertiary",
				wrap = true,
				w_full = true,
				grow = { min = 20, max = 220 },
				px = 0,
				py = 0,
				border = 0,
				bg = "#00000000",
				focus = { border = 0, outline = 0 },
				text_size = ui.raw(13, "gpui"),
			},
			ui.presence(function(): string?
				local list = live().images or {}
				return if #list > 0 then table.concat(list, "\n") else nil
			end, function(): Node
				return Images(live())
			end, { w_full = true }),
		},
		ui.row {
			id = "task-card-chips",
			gap = ui.raw(6, "gpui"),
			wrap = true,
			items = "center",
			Pick {
				id = "task-card-due",
				label = locale.s("tasks.card.due"),
				icon = "calendar",
				w = 180,
				value = function(): string
					return due_value(live())
				end,
				options = function(): { any }
					local out = { { value = "", label = locale.s("tasks.card.no_date") }, { value = "today", label = locale.s("tasks.card.today") }, { value = "tomorrow", label = locale.s("tasks.card.tomorrow") } }
					local v = due_value(live())
					if v ~= "" and v ~= "today" and v ~= "tomorrow" then
						table.insert(out, { value = v, label = (store.due_label(live())) or v })
					end
					return out
				end,
				on_pick = function(v: string)
					local day = if v == "today" then store.today() elseif v == "tomorrow" then store.tomorrow() else v
					store.update(id, { due = day })
				end,
			},
			Pick {
				id = "task-card-project",
				label = locale.s("tasks.card.project"),
				icon = "folder",
				w = 220,
				value = function(): string
					return live().project or ""
				end,
				options = function(): { any }
					local out = { { value = "", label = locale.s("tasks.no_project") } }
					for _, p in sessions.now:get().projects do
						table.insert(out, { value = p.id, label = p.name })
					end
					return out
				end,
				on_pick = function(v: string)
					store.update(id, { project = v })
				end,
			},
			ui.show(function(): boolean
				return #(live().images or {}) < store.IMAGES_MAX
			end, Pill {
				id = "task-card-add-image",
				label = locale.s("tasks.card.add_image"),
				tooltip = ds.Tooltip { locale.s("tasks.card.add_image") },
				on_click = function()
					M.add_images(live())
				end,
				ds.Icon { name = "gallery", size = 14, color = "$text.secondary" },
			}),
		},
		ui.presence(function(): string
			local l = live()
			if l.completed then
				return "done"
			end
			return if l.chat or l.send_error then `linked:{l.chat or ""}:{if store.state(l).chat then "" else "gone"}` else "hand"
		end, function(kind: string): Node
			if kind == "done" then
				return Done(live())
			elseif kind == "hand" then
				return HandOff(live())
			end
			return Linked(live())
		end, { w_full = true }),
	}
end

M.Card = ui.component(function(): Node
	local reduced = motion.reduced_now()
	local room = ui.use_state(0)
	local content_h = ui.use_state(0)
	local function measure(into: State<number>): (size: any) -> ()
		return function(size: any)
			if math.abs(into:peek() - size.h) >= 0.5 then
				into:set(size.h)
			end
		end
	end
	return ui.presence(function(): string?
		return if M.current:get() ~= nil then "open" else nil
	end, function(): Node
		return ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			ui.div {
				id = "task-card-backdrop",
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				on_click = M.close,
			},
			ui.column {
				absolute = true,
				top = 16,
				right = 16,
				bottom = 16,
				w = W,
				pointer_events = "none",
				on_resize = measure(room),
				ui.column {
					id = "task-card",
					role = "dialog",
					label = function(): string
						local t = store.find(M.current:get())
						return if t then title_of(t) else locale.s("tasks.card.title")
					end,
					w_full = true,
					p = ui.raw(16, "gpui"),
					rounded = ui.raw(RADIUS, "gpui"),
					bg = "$zeron.tasks.panel",
					border = 1,
					border_color = "$zeron.tasks.panel_edge",
					overflow = "hidden",
					on_key = function(e: any): boolean
						if e.key == "Escape" then
							M.close()
							return true
						elseif e.key == "Enter" and (e.meta or e.cmd or e.ctrl) then
							local t = store.find(M.current:peek())
							if t and not t.completed and not t.chat and not t.send_error and hand_off:peek() == "new" then
								M.start(t)
								return true
							end
						end
						return false
					end,
					on_press = function()
						pressed_in = true
						ui.after(0, function()
							pressed_in = false
						end)
					end,
					on_file_drop = function(files: { any })
						local id = M.current:peek()
						if id then
							M.add_files(id, files)
						end
					end,
					ui.scroll {
						w_full = true,
						h = function(): number
							return math.min(content_h:get(), math.max(room:get() - 34, 0))
						end,
						ui.presence(function(): string
							local id = M.current:get() or ""
							return if store.find(id) then id else `{id}?`
						end, function(key: string): Node
							return ui.column {
								w_full = true,
								on_resize = measure(content_h),
								Body((string.gsub(key, "%?$", ""))),
							}
						end, {
							w_full = true,
							mode = "wait",
							initial = true,
							enter = if reduced then nil else { opacity = { 0, 1 }, y = { 6, 0 }, duration = 180, ease = DRAWER },
							exit = if reduced then nil else { opacity = 0, y = -4, duration = 90, ease = "linear" },
						}),
					},
				},
			},
		}
	end, {
		enter = if reduced then nil else { opacity = { 0, 1 }, x = { 28, 0 }, duration = 300, ease = DRAWER },
		exit = if reduced then nil else { opacity = 0, x = 20, duration = 220, ease = SHARP },
	})
end)

return M
