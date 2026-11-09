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

local M = {}

local W, RADIUS, SAVE_MS, THUMB = 340, 24, 500, 56

M.current = ui.state("tasks_card", nil :: string?)
M.focus = ui.state("tasks_card_focus", "title")

function M.open(id: string, focus: string?)
	ui.batch(function()
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
local function saved(t: store.Task, key: string): (State<string>, () -> ())
	local value = ui.use_state((t :: any)[key] or "")
	local seq = 0
	local pending = false
	local function flush()
		if pending then
			pending = false
			local v = value:peek()
			if key ~= "title" or not string.match(v, "^%s*$") then
				store.update(t.id, { [key] = v })
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
	return value, typed :: any
end

local function Pick(props: { id: string, label: string, value: () -> string, options: () -> { any }, on_pick: (string) -> () }): Node
	local chosen = ui.use_state(props.value())
	ui.effect(function()
		local v = props.value()
		if chosen:peek() ~= v then
			chosen:set(v)
		end
	end)
	ui.effect(function()
		local v = chosen:get()
		if v ~= props.value() then
			props.on_pick(v)
		end
	end)
	return ds.Select { id = props.id, value = chosen, options = props.options, name = props.label, size = "sm" }
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
function M.picture(path: string, size: number, glyph: number): Node
	return ui.show(function(): boolean
		return not store.image_failed(path)
	end, ui.image {
		src = function(): string
			return store.image_src(path) or ""
		end,
		w = size,
		h = size,
		fit = "cover",
		aria_hidden = true,
		on_error = function()
			store.image_broke(path)
		end,
	}, ui.div { w = size, h = size, items = "center", justify = "center", ds.Icon { name = "gallery", size = glyph, color = "$text.tertiary" } })
end
function M.missing_tip(path: string): Node
	return ui.show(function(): boolean
		return store.image_failed(path)
	end, ds.Tooltip { function(): string
		return locale.s("tasks.image_missing")
	end })
end

local function Images(t: store.Task): Node
	local list = t.images or {}
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
	local row: { [any]: any } = { id = "task-card-images", gap = ui.raw(6, "gpui"), wrap = true }
	for i, path in list do
		local over = ui.use_state(false)
		table.insert(row, ui.div {
			relative = true,
			w = THUMB,
			h = THUMB,
			rounded = ui.raw(10, "gpui"),
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
			end, Lightbox.Thumb { group = `task-card-{t.id}`, index = i, items = items, radius = 10, w = THUMB, h = THUMB, M.picture(path, THUMB, 16) }, ds.Icon { name = "gallery", size = 16, color = "$text.tertiary" }),
			ui.show(function(): boolean
				return over:get()
			end, ui.button {
				id = `task-card-remove-image-{i}`,
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
					local next = table.clone(list)
					table.remove(next, i)
					store.update(t.id, { images = next })
				end,
				ds.Icon { name = "close", size = 10, color = ui.raw("#ffffff", "white on the accent, as gpui") },
			}),
		})
	end
	if #list < store.IMAGES_MAX then
		table.insert(row, ui.button {
			id = "task-card-add-image",
			label = locale.s("tasks.add_image"),
			w = THUMB,
			h = THUMB,
			p = 0,
			rounded = ui.raw(10, "gpui"),
			border = 1,
			border_style = "dotted",
			border_color = "$text.tertiary",
			bg = "#00000000",
			hover = { bg = "$hover" },
			color = "$text.secondary",
			items = "center",
			justify = "center",
			text_size = ui.raw(11, "gpui"),
			on_click = function()
				M.add_images(t)
			end,
			ui.text { locale.s("tasks.card.add_image_tile"), align = "center", wrap = true },
		})
	end
	return ui.row(row)
end

function M.add_images(t: store.Task)
	ui.open_files({
		title = locale.s("tasks.add_image"),
		multiple = true,
		filters = { { locale.s("tasks.card.images"), { "png", "jpg", "jpeg", "webp", "gif" } } },
	}):after(function(files: { any })
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
	end)
end

-- The new chat composer's agent, as a new chat would start (nil: none known).
local function defaults(): { [string]: any }
	local d = chat_store.new_defaults
	return if d then d:get() else {}
end
local function new_chat_model(): any
	local d = defaults()
	local catalog = chat_store.catalog:get()
	for _, m in catalog.models or {} do
		if m.value == d.model then
			return m
		end
	end
	return (catalog.models or {})[1]
end

M.starting = ui.state("tasks_card_starting", nil :: string?)

-- Start a task in a new chat on the new chat composer's agent; refused, the button comes back.
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
	store.start(t.id, { model = m.value, effort = defaults().effort, access = defaults().access }, t.project, over, over)
end

local function NewChat(t: store.Task): Node
	local function starting(): boolean
		return M.starting:get() == t.id
	end
	local function start()
		M.start(t)
	end
	return ui.column {
		gap = ui.raw(8, "gpui"),
		ui.row {
			id = "task-card-agent",
			h = 30,
			px = ui.raw(10, "gpui"),
			rounded = ui.raw(10, "gpui"),
			bg = "$hover",
			items = "center",
			text_size = ui.raw(12.5, "gpui"),
			ui.text { function(): string
				local m = new_chat_model()
				return if m then m.label else locale.s("tasks.card.no_agent")
			end, color = "$text.primary", truncate = true },
		},
		ui.row {
			gap = ui.raw(8, "gpui"),
			items = "center",
			ds.Button {
				function(): string
					return locale.s(if starting() then "tasks.card.starting" else "tasks.card.start")
				end,
				id = "task-card-start",
				
				tone = "primary",
				size = "sm",
				disabled = function(): boolean
					return new_chat_model() == nil or starting()
				end,
				on_click = start,
			},
			ui.text { ui.shortcut_text("cmd+enter"), text_size = ui.raw(11.5, "gpui"), color = "$text.secondary" },
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
			gap = ui.raw(8, "gpui"),
			ds.Button {
				locale.t("tasks.card.send"),
				id = "task-card-send", tone = "primary", size = "sm", disabled = function(): boolean
				return picked:get() == nil
			end, on_click = function()
				send(true)
			end },
			ds.Button {
				locale.t("tasks.card.link_only"),
				id = "task-card-link", size = "sm", disabled = function(): boolean
				return picked:get() == nil
			end, on_click = function()
				send(false)
			end },
		},
	}
end

local hand_off = ui.persist("tasks_card_hand_off", "new")

local function HandOff(t: store.Task): Node
	return ui.column {
		gap = ui.raw(8, "gpui"),
		ui.text { locale.s("tasks.card.hand_it_to"), text_size = ui.raw(11.5, "gpui"), weight = "$font.weight.semibold", color = "$text.secondary" },
		ds.Segmented { id = "task-card-hand-off", value = hand_off, label = locale.t("tasks.card.hand_it_to"), options = { { value = "new", label = locale.t("tasks.card.new_chat") }, { value = "thread", label = locale.t("tasks.card.thread") } } },
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
		gap = ui.raw(8, "gpui"),
		if c then ds.Button {
				locale.t("tasks.card.open_chat"),
				id = "task-card-open-chat", size = "sm", on_click = function()
			M.close()
			route.open_session(c.id)
		end } else nil,
		ds.Button {
				locale.t("tasks.mark_not_done"),
				id = "task-card-reopen", size = "sm", tone = "primary", on_click = function()
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
	local title, type_title = saved(t, "title")
	local notes, type_notes = saved(t, "notes")
	local menu = ui.use_state(false)
	local title_ref, notes_ref = ui.ref(), ui.ref()
	ui.after(0, function()
		local el = (if M.focus:peek() == "notes" then notes_ref else title_ref):peek()
		if el then
			el:focus()
		end
	end)
	local function live(): store.Task
		return store.find(id) or t
	end
	local more = ui.button {
		id = "task-card-menu",
		label = locale.s("tasks.card.more"),
		w = 28,
		h = 28,
		p = 0,
		rounded = ui.raw(8, "gpui"),
		bg = "#00000000",
		hover = { bg = "$hover" },
		items = "center",
		justify = "center",
		expanded = function(): boolean
			return menu:get()
		end,
		on_click = function()
			menu:set(not menu:peek())
		end,
		ds.Icon { name = "more-horizontal", size = 16 },
	}
	return ui.column {
		w_full = true,
		gap = ui.raw(12, "gpui"),
		ui.row {
			justify = "end",
			gap = ui.raw(4, "gpui"),
			ui.popover(Menu.panel {
				open = menu,
				anchor = more,
				w = 200,
				placement = { side = "bottom", align = "end", offset = 4 },
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
			ui.button {
				id = "task-card-close",
				label = locale.s("tasks.card.close"),
				w = 28,
				h = 28,
				p = 0,
				rounded = ui.raw(8, "gpui"),
				bg = "#00000000",
				hover = { bg = "$hover" },
				items = "center",
				justify = "center",
				on_click = M.close,
				ds.Icon { name = "close", size = 14 },
			},
		},
		ui.input {
			id = "task-card-title",
			ref = title_ref,
			label = locale.s("tasks.card.title"),
			placeholder = locale.s("tasks.card.title"),
			placeholder_color = "$text.tertiary",
			value = function(): string
				return title:get()
			end,
			on_change = type_title,
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
			w_full = true,
			h = 24,
			px = 0,
			border = 0,
			bg = "#00000000",
			focus = { border = 0, outline = 0 },
			text_size = ui.raw(17, "gpui"),
			weight = "$font.weight.semibold",
		},
		ui.textarea {
			id = "task-card-notes",
			ref = notes_ref,
			label = locale.s("tasks.card.notes"),
			placeholder = locale.s("tasks.card.notes"),
			placeholder_color = "$text.tertiary",
			value = function(): string
				return notes:get()
			end,
			on_change = type_notes,
			w_full = true,
			min_h = 60,
			px = 0,
			border = 0,
			bg = "#00000000",
			focus = { border = 0, outline = 0 },
			text_size = ui.raw(13, "gpui"),
		},
		ui.row {
			gap = ui.raw(6, "gpui"),
			wrap = true,
			Pick {
				id = "task-card-due",
				label = locale.s("tasks.card.due"),
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
		},
		ui.presence(function(): string
			return table.concat(live().images or {}, "\n")
		end, function(): Node
			return Images(live())
		end, { w_full = true }),
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
	return ui.show(function(): boolean
		return M.current:get() ~= nil
	end, ui.div {
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
		ui.column(glass.style({
			id = "task-card",
			role = "dialog",
			label = function(): string
				local t = store.find(M.current:get())
				return if t then title_of(t) else locale.s("tasks.card.title")
			end,
			absolute = true,
			top = 16,
			right = 16,
			bottom = 16,
			w = W,
			p = ui.raw(18, "gpui"),
			rounded = ui.raw(RADIUS, "gpui"),
			overflow = "hidden",
			on_key = function(e: any): boolean
				if e.key == "Escape" then
					M.close()
					return true
				elseif e.key == "Enter" and (e.meta or e.cmd) then
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
			enter = motion.transition({ opacity = { 0, 1 }, x = { 28, 0 }, duration = 200 }),
			exit = motion.transition({ opacity = 0, x = 20, duration = 140 }),
			ui.scroll {
				w_full = true,
				grow = 1,
				-- (The engine may answer a create before its watch brings the task: the body is built again as it comes.)
				ui.presence(function(): string
					local id = M.current:get() or ""
					return if store.find(id) then id else `{id}?`
				end, function(key: string): Node
					return Body((string.gsub(key, "%?$", "")))
				end, { w_full = true }),
			},
		}, "popup")),
	})
end)

return M
