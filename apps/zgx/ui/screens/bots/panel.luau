local ds = require("@ds")
local Menu = require("@app/screens/bots/menu")
local locale = require("@ds/locale")
local motion = require("@ds/motion")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local store = require("@app/screens/bots/store")
local avatar = require("@app/screens/bots/avatar")

local M = {}

local HOVER_MS, ARRIVE_MS, COMPACT_MS = 110, 200, 160
local TILE_W, TILE_AVATAR, ROW_H, ROW_AVATAR = 72, 46, 56, 38
local STATUS_H = 18
local EASE = "cubicBezier(.2, .8, .2, 1)"

local asks = ui.shared("chat.asks", {} :: { [string]: string })

local moved = ui.state("bots_panel_moved", nil :: string?)
local editing = ui.state("bots_panel_editing", nil :: string?)
local edit_open = ui.state("bots_panel_edit_open", false)
local edit_name = ui.state("bots_panel_edit_name", "")
-- The height Compact view took from the session rows while Properties is open:
-- kept under them until the menu closes, so the menu's button stays put.
local slack = ui.state("bots_panel_slack", 0)

local function fill(bg: string, shown: any, radius: any): Node
	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		rounded = radius,
		bg = bg,
		opacity = ui.tween(shown, { duration = math.max(motion.ms(HOVER_MS), 1), ease = "outQuad" }),
		pointer_events = "none",
	}
end

local function arrive(id: string): (any) -> ()
	return function(el: any)
		if moved:peek() ~= id then
			return
		end
		moved:set(nil)
		local ms = motion.ms(ARRIVE_MS)
		if ms > 0 then
			anime.animate(el, { y = { 6, 0 }, opacity = { 0, 1 }, duration = ms, ease = EASE })
		end
	end
end

function M.edit(bot: store.Bot)
	edit_name:set(bot.name)
	editing:set(bot.id)
	edit_open:set(true)
end

function M.menu(bot: store.Bot): { Menu.Entry }
	local function leave()
		local open = store.open:peek()
		if open and open.id == bot.id then
			store.gallery()
		end
	end
	return {
		{
			label = ui.t(if bot.pinned then "bots.menu.unpin" else "bots.menu.pin"),
			icon = "pin",
			on_select = function()
				moved:set(bot.id)
				store.pin(bot.id, not bot.pinned)
			end,
		},
		{ separator = true },
		{
			label = ui.t("bots.menu.edit"),
			icon = "pen",
			on_select = function()
				M.edit(bot)
			end,
		},
		{
			label = ui.t(if bot.unread then "bots.menu.mark_read" else "bots.menu.mark_unread"),
			icon = "unread-square",
			on_select = function()
				store.mark_unread(bot.id, not bot.unread)
			end,
		},
		{ separator = true },
		{
			label = ui.t("bots.menu.archive"),
			icon = "archive-minimalistic",
			on_select = function()
				leave()
				store.archive(bot.id, true)
			end,
		},
		{
			label = ui.t("bots.menu.delete"),
			icon = "trash-bin-minimalistic",
			danger = true,
			confirm = ui.t("bots.menu.delete_confirm"),
			on_select = function()
				leave()
				store.delete(bot.id)
			end,
		},
	}
end

local function selected(id: string): () -> number
	return function(): number
		local open = store.open:get()
		return if open and open.id == id then 1 else 0
	end
end

local function chat_of(bot: store.Bot): sessions.Chat?
	for _, c in sessions.now:get().chats do
		if c.id == bot.chat then
			return c
		end
	end
	return nil
end

local STATUS_WORD: { [string]: string } = {
	working = "bots.status.working",
	waiting = "bots.status.waiting",
	approval = "bots.status.approval",
	input = "bots.status.input",
	done = "bots.status.done",
}

local function line_of(bot: store.Bot): string
	local c = chat_of(bot)
	if not c then
		return ""
	end
	if c.status == "working" then
		return locale.s("bots.status.working")
	end
	return c.preview or ""
end

local Tile = ui.component(function(props: { row: any, index: () -> number }): Node
	local row, index = props.row, props.index
	local bot = row:peek() :: store.Bot
	local over = ui.use_state(false)
	local menu_open = ui.use_state(false)
	local on = ui.computed(selected(bot.id))
	local wash = ui.computed(function(): number
		return if over:get() or menu_open:get() or on:get() == 1 then 1 else 0
	end)
	local name = function(): string
		return (row:get() :: store.Bot).name
	end
	local ref = ui.ref()
	return Menu.Context {
		items = function(): { Menu.Entry }
			return M.menu(row:peek())
		end,
		label = ui.t("bots.menu.label"),
		focus = ref,
		open = menu_open,
		ui.button {
			id = `bot-tile-{bot.id}`,
			ref = ref,
			class = "bot-tile",
			label = name,
			tooltip = ui.row {
				px = ui.raw(9, "the mockup's tip"),
				py = ui.raw(5, "the mockup's tip"),
				gap = ui.raw(8, "the mockup's tip"),
				rounded = ui.raw(8, "the mockup's tip"),
				bg = ui.raw("#161615", "the mockup's tip, dark in both themes"),
				text_size = "$font.size.xs",
				ui.text { name, literal = true, color = ui.raw("#ffffff", "on the dark tip") },
				ui.show(function(): boolean
					local i = index()
					return i >= 1 and i <= 9
				end, ui.text {
					function(): string
						return ui.shortcut_text(`alt+shift+{math.clamp(index(), 1, 9)}`)
					end,
					literal = true,
					color = ui.raw("#ffffff99", "on the dark tip"),
				}),
			},
			tooltip_delay = 400,
			on_mount = arrive(bot.id),
			relative = true,
			w = TILE_W,
			pt = ui.raw(7, "the mockup's tile"),
			pb = ui.raw(6, "the mockup's tile"),
			px = 0,
			direction = "column",
			items = "center",
			gap = ui.raw(6, "the mockup's tile"),
			rounded = ui.raw(10, "the mockup's tile"),
			bg = "#00000000",
			hover = { bg = "#00000000" },
			active = { bg = "$pressed" },
			cursor = "pointer",
			on_hover = function(inside: boolean)
				over:set(inside)
			end,
			on_click = function()
				store.go(row:peek())
			end,
			fill("$hover", wash, ui.raw(10, "the mockup's tile")),
			avatar.Avatar {
				size = TILE_AVATAR,
				look = function(): avatar.Look
					return (row:get() :: store.Bot).avatar
				end,
			},
			ui.text {
				name,
				literal = true,
				truncate = true,
				max_w = ui.raw(62, "the mockup's tile name"),
				text_size = ui.raw(11.5, "the mockup's tile name"),
				color = function(): string
					return if on:get() == 1 then "$text.primary" else "$text.secondary"
				end,
			},
		},
	}
end)

local Row = ui.component(function(props: { row: any }): Node
	local row = props.row
	local bot = row:peek() :: store.Bot
	local over = ui.use_state(false)
	local menu_open = ui.use_state(false)
	local on = ui.computed(selected(bot.id))
	local wash = ui.computed(function(): number
		return if (over:get() or menu_open:get()) and on:get() == 0 then 1 else 0
	end)
	local name = function(): string
		return (row:get() :: store.Bot).name
	end
	local ref = ui.ref()
	return Menu.Context {
		items = function(): { Menu.Entry }
			return M.menu(row:peek())
		end,
		label = ui.t("bots.menu.label"),
		focus = ref,
		open = menu_open,
		w_full = true,
		ui.button {
			id = `bot-row-{bot.id}`,
			ref = ref,
			class = "bot-row",
			label = name,
			on_mount = arrive(bot.id),
			relative = true,
			w_full = true,
			h = ROW_H,
			px = ui.raw(8, "the mockup's row"),
			gap = ui.raw(10, "the mockup's row"),
			items = "center",
			justify = "start",
			rounded = ui.raw(9, "the mockup's row"),
			bg = "#00000000",
			hover = { bg = "#00000000" },
			active = { bg = "$pressed" },
			cursor = "pointer",
			on_hover = function(inside: boolean)
				over:set(inside)
			end,
			on_click = function()
				store.go(row:peek())
			end,
			fill("$zeron.selected.flat.bg", on, ui.raw(9, "the mockup's row")),
			fill("$hover", wash, ui.raw(9, "the mockup's row")),
			avatar.Avatar {
				size = ROW_AVATAR,
				look = function(): avatar.Look
					return (row:get() :: store.Bot).avatar
				end,
			},
			ui.column {
				grow = 1,
				min_w = 0,
				gap = ui.raw(1, "the mockup's row"),
				ui.text { name, literal = true, truncate = true, text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
				ui.text {
					function(): string
						return line_of(row:get())
					end,
					literal = true,
					truncate = true,
					text_size = ui.raw(12.5, "the mockup's row line"),
					color = "$text.secondary",
				},
			},
			ui.show(function(): boolean
				return (row:get() :: store.Bot).unread
			end, ui.div {
				w = 8,
				h = 8,
				shrink = 0,
				self = "start",
				mt = ui.raw(14, "the mockup's dot"),
				rounded = "$radius.full",
				bg = "$info",
			}),
		},
	}
end)

type Kid = { id: string, title: string, status: string?, updated: number, project: string? }
type Folder = { key: string, name: string, kids: { Kid } }

local function kids_of(bot: store.Bot): { Kid }
	local children = sessions.now:get().children or {}
	local out: { Kid } = {}
	local function walk(parent: string, depth: number)
		for _, k in children[parent] or {} do
			table.insert(out, k :: any)
			if depth < 4 then
				walk(k.id, depth + 1)
			end
		end
	end
	walk(bot.chat, 0)
	return out
end

local function status_of(k: Kid): string
	local asked = asks:get()[k.id]
	if asked == "approval" then
		return "approval"
	elseif asked == "question" then
		return "input"
	end
	return k.status or "done"
end

local RANK: { [string]: number } = { approval = 1, input = 2, working = 3, waiting = 4, done = 5 }

local function sorted(kids: { Kid }): { Kid }
	local by = store.sort:get()
	table.sort(kids, function(a: Kid, b: Kid): boolean
		if by == "status" then
			local ra, rb = RANK[status_of(a)] or 5, RANK[status_of(b)] or 5
			if ra ~= rb then
				return ra < rb
			end
		end
		if a.updated ~= b.updated then
			return a.updated > b.updated
		end
		return a.id < b.id
	end)
	return kids
end

local function folders_of(bot: store.Bot): ({ Folder }, { Kid })
	local now = sessions.now:get()
	local names: { [string]: string } = {}
	for _, p in now.projects do
		names[p.id] = p.name
	end
	local home = chat_of(bot)
	local folders: { Folder } = {}
	local by: { [string]: Folder } = {}
	local done: { Kid } = {}
	for _, k in sorted(kids_of(bot)) do
		if status_of(k) == "done" then
			table.insert(done, k)
			continue
		end
		local project = k.project or (home and home.project) or ""
		local folder = by[project]
		if not folder then
			folder = { key = project, name = names[project] or locale.s("bots.sessions.no_folder"), kids = {} }
			by[project] = folder
			table.insert(folders, folder)
		end
		table.insert(folder.kids, k)
	end
	table.sort(folders, function(a: Folder, b: Folder): boolean
		return a.name < b.name
	end)
	return folders, done
end

local function Spinner(): Node
	return ui.div {
		w = 14,
		h = 14,
		mt = ui.raw(2, "the mockup's session row"),
		shrink = 0,
		color = "$text.secondary",
		on_mount = function(el: any)
			if motion.ms(1) > 0 then
				anime.animate(el, { rotate = { 0, 360 }, duration = 2400, ease = "linear", loop = true })
			end
		end,
		ds.Icon { name = "lucide-circle-dashed", size = 14 },
	}
end

local function Sub(props: { [any]: any }): Node
	return ui.row {
		id = props.id,
		w_full = true,
		h = ui.raw(30, "the mockup's group row"),
		px = ui.raw(8, "the mockup's group row"),
		gap = ui.raw(8, "the mockup's group row"),
		items = "center",
		color = "$text.secondary",
		text_size = "$font.size.sm",
		if props.icon then ds.Icon { name = props.icon, size = 15 } else nil,
		ui.text { props.name, literal = true, truncate = true, min_w = 0 },
		ui.text { props.count, literal = true, color = "$text.tertiary", shrink = 0 },
	}
end

local function SessionRow(k: Kid): Node
	local status = status_of(k)
	return ui.button {
		id = `bot-session-{k.id}`,
		class = "bot-session",
		label = k.title,
		w_full = true,
		px = ui.raw(8, "the mockup's session row"),
		py = ui.raw(6, "the mockup's session row"),
		gap = ui.raw(8, "the mockup's session row"),
		items = "start",
		justify = "start",
		rounded = ui.raw(7, "the mockup's session row"),
		bg = function(): string
			local c = route.current:get()
			return if c.name == "session" and c.session == k.id then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		cursor = "pointer",
		on_click = function()
			route.open_session(k.id)
		end,
		if status == "working"
			then Spinner()
			else ui.div { w = 14, h = 14, mt = ui.raw(2, "the mockup's session row"), shrink = 0, color = "$text.secondary", ds.Icon { name = "chevron-right", size = 14 } },
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text { k.title, literal = true, truncate = true, text_size = "$font.size.sm", color = "$text.primary" },
			ui.div {
				overflow = "hidden",
				h = ui.tween(
					ui.computed(function(): number
						return if store.compact:get() then 0 else STATUS_H
					end),
					{ duration = math.max(motion.ms(COMPACT_MS), 1), ease = "outQuad" }
				),
				opacity = ui.tween(
					ui.computed(function(): number
						return if store.compact:get() then 0 else 1
					end),
					{ duration = math.max(motion.ms(COMPACT_MS), 1), ease = "outQuad" }
				),
				ui.text { ui.t(STATUS_WORD[status] or "bots.status.done"), mt = ui.raw(2, "the mockup's session row"), text_size = "$font.size.xs", color = "$text.secondary" },
			},
		},
	}
end

local function Head(props: { [any]: any }): Node
	return ui.row {
		id = props.id,
		w_full = true,
		h = ui.raw(34, "the mockup's section head"),
		mt = ui.raw(10, "the mockup's section head"),
		px = ui.raw(8, "the mockup's section head"),
		gap = ui.raw(6, "the mockup's section head"),
		items = "center",
		shrink = 0,
		ui.text { props.title, text_size = ui.raw(13.5, "the mockup's section head"), weight = "$font.weight.semibold", color = "$text.primary" },
		if props.count then ui.text { props.count, literal = true, text_size = ui.raw(13.5, "the mockup's section head"), color = "$text.tertiary" } else nil,
		ui.div { grow = 1 },
		props.action,
	}
end

local function Needs(bot: () -> store.Bot?): Node
	local waiting = ui.computed(function(): { Kid }
		local b = bot()
		if not b then
			return {}
		end
		local out: { Kid } = {}
		local asked = asks:get()
		local home = chat_of(b)
		if home and (asked[home.id] or home.status == "approval" or home.status == "input") then
			table.insert(out, { id = home.id, title = home.preview or home.title, status = home.status, updated = home.updated })
		end
		for _, k in kids_of(b) do
			local s = status_of(k)
			if s == "approval" or s == "input" then
				table.insert(out, k)
			end
		end
		return out
	end)
	return ui.show(
		function(): boolean
			return #waiting:get() > 0
		end,
		ui.column {
			id = "bots-needs",
			w_full = true,
			Head {
				title = ui.t("bots.needs.title"),
				count = function(): string
					return tostring(#waiting:get())
				end,
			},
			Sub {
				name = ui.t("bots.needs.decisions"),
				count = function(): string
					return tostring(#waiting:get())
				end,
			},
			ui.each(waiting, function(k: Kid): Node
				return ui.button {
					id = `bot-decision-{k.id}`,
					class = "bot-decision",
					label = k.title,
					w_full = true,
					h = ui.raw(30, "the mockup's group row"),
					px = ui.raw(8, "the mockup's group row"),
					gap = ui.raw(8, "the mockup's group row"),
					items = "center",
					justify = "start",
					rounded = ui.raw(7, "the mockup's group row"),
					bg = "#00000000",
					hover = { bg = "$hover" },
					active = { bg = "$pressed" },
					cursor = "pointer",
					color = "$text.primary",
					on_click = function()
						route.open_session(k.id)
					end,
					ds.Icon { name = "lucide-circle-help", size = 16 },
					ui.text { k.title, literal = true, truncate = true, min_w = 0, text_size = "$font.size.sm" },
				}
			end, {
				key = function(k: Kid): string
					return k.id
				end,
			}),
		}
	)
end

local function Properties(folders: () -> { string }, rows: () -> number): Node
	local open = ui.use_state(false)
	ui.effect(function()
		if not open:get() then
			slack:set(0)
		end
	end)
	local function sort(by: string, icon: string, word: string): Menu.Entry
		return {
			id = `bots-sort-{by}`,
			label = ui.t(word),
			icon = icon,
			checked = function(): boolean
				return store.sort:get() == by
			end,
			on_select = function()
				store.sort:set(by)
			end,
		}
	end
	return Menu.Popup {
		open = open,
		label = ui.t("bots.properties.title"),
		title = ui.t("bots.properties.title"),
		width = 250,
		placement = { side = "bottom", align = "start", offset = 4 },
		items = function(): { Menu.Entry }
			return {
				{
					id = "bots-properties-sort",
					label = ui.t("bots.properties.sort"),
					icon = "sort-vertical",
					value = function(): string
						return locale.s(if store.sort:get() == "status" then "bots.properties.status" else "bots.properties.updated")
					end,
					submenu = {
						width = 210,
						entries = function(): { Menu.Entry }
							return { sort("status", "lucide-circle-dashed", "bots.properties.status"), sort("updated", "clock-circle", "bots.properties.updated") }
						end,
					},
				},
				{
					id = "bots-properties-group",
					label = ui.t("bots.properties.group"),
					icon = "widget",
					value = function(): string
						return locale.s("bots.properties.folders")
					end,
					submenu = {
						width = 220,
						title = ui.t("bots.properties.group_by"),
						entries = function(): { Menu.Entry }
							local out: { Menu.Entry } = {}
							for _, name in folders() do
								table.insert(out, { label = name, icon = "folder", disabled = true })
							end
							if #out == 0 then
								table.insert(out, { label = ui.t("bots.properties.no_folders"), icon = "folder", disabled = true })
							end
							return out
						end,
					},
				},
				{ separator = true },
				{
					id = "bots-properties-compact",
					label = ui.t("bots.properties.compact"),
					icon = "compact-rows",
					keep_open = true,
					switch = function(): boolean
						return store.compact:get()
					end,
					on_select = function()
						local on = not store.compact:peek()
						store.compact:set(on)
						slack:set(if on then rows() * STATUS_H else 0)
					end,
				},
			}
		end,
		anchor = ds.IconButton {
			id = "bots-properties",
			icon = "sort-vertical",
			label = ui.t("bots.properties.title"),
			size = "sm",
			tone = "quiet",
			lit = open,
			on_click = ui.toggle(open),
		},
	}
end

local function Sessions(bot: () -> store.Bot?): Node
	local shown = ui.computed(function(): { folders: { Folder }, done: { Kid } }
		local b = bot()
		if not b then
			return { folders = {}, done = {} }
		end
		local folders, done = folders_of(b)
		return { folders = folders, done = done }
	end)
	local function group(icon: string, name: () -> string, kids: () -> { Kid }, id: string?): Node
		return ui.column {
			w_full = true,
			Sub {
				id = id,
				icon = icon,
				name = name,
				count = function(): string
					return tostring(#kids())
				end,
			},
			ui.each(ui.computed(kids), function(k: Kid): Node
				return SessionRow(k)
			end, {
				key = function(k: Kid): string
					return `{k.id}:{status_of(k)}:{k.title}`
				end,
			}),
		}
	end
	return ui.column {
		id = "bots-sessions",
		w_full = true,
		Head {
			id = "bots-sessions-head",
			title = ui.t("bots.sessions.title"),
			action = Properties(function(): { string }
				local names = {}
				for _, f in shown:peek().folders do
					table.insert(names, f.name)
				end
				return names
			end, function(): number
				local s = shown:peek()
				local n = #s.done
				for _, f in s.folders do
					n += #f.kids
				end
				return n
			end),
		},
		ui.show(function(): boolean
			local s = shown:get()
			return #s.folders == 0 and #s.done == 0
		end, ui.text {
			id = "bots-sessions-empty",
			ui.t("bots.sessions.empty"),
			px = ui.raw(8, "the mockup's group row"),
			py = "$space.2",
			text_size = "$font.size.sm",
			color = "$text.tertiary",
		}),
		ui.each(ui.computed(function(): { Folder }
			return shown:get().folders
		end), function(_f: Folder, _i: number, row: any): Node
			return group("folder", function(): string
				return (row:get() :: Folder).name
			end, function(): { Kid }
				return (row:get() :: Folder).kids
			end)
		end, {
			key = function(f: Folder): string
				return f.key
			end,
		}),
		ui.show(function(): boolean
			local s = shown:get()
			return #s.folders > 0 or #s.done > 0
		end, group("flag", function(): string
			return locale.s("bots.sessions.done")
		end, function(): { Kid }
			return shown:get().done
		end, "bots-sessions-done")),
		ui.div { w_full = true, shrink = 0, h = ui.tween(slack, { duration = math.max(motion.ms(COMPACT_MS), 1), ease = "outQuad" }) },
	}
end

local function EditDialog(): Node
	local function done()
		local id = editing:peek()
		local name = string.match(edit_name:peek(), "^%s*(.-)%s*$") or ""
		edit_open:set(false)
		editing:set(nil)
		if id and name ~= "" then
			store.rename(id, string.sub(name, 1, 80))
		end
	end
	return ds.Dialog {
		open = edit_open,
		title = ui.t("bots.edit.title") :: any,
		size = "sm",
		confirm = { label = ui.t("common.save") :: any, on_click = done },
		ds.TextField { value = edit_name, label = ui.t("bots.edit.name"), select_on_focus = true, w_full = true },
	}
end

local function Header(): Node
	local adding = ui.use_state(false)
	return ui.row {
		id = "bots-panel-head",
		w_full = true,
		h = ui.raw(44, "the mockup's panel head"),
		pl = ui.raw(14, "the mockup's panel head"),
		pr = ui.raw(10, "the mockup's panel head"),
		items = "center",
		shrink = 0,
		ui.text { ui.t("bots.title"), role = "heading", text_size = ui.raw(14.5, "the mockup's panel head"), weight = "$font.weight.semibold", color = "$text.primary" },
		ui.div { grow = 1 },
		Menu.Popup {
			open = adding,
			label = ui.t("bots.add.label"),
			items = function(): { Menu.Entry }
				return { { id = "bots-add-create", label = ui.t("bots.add.create"), icon = "bot", on_select = M.create } }
			end,
			anchor = ds.IconButton {
				id = "bots-add",
				icon = "plus",
				label = ui.t("bots.add.label"),
				size = "sm",
				tone = "quiet",
				lit = adding,
				on_click = ui.toggle(adding),
			},
		},
		ds.IconButton { id = "bots-more", icon = "more-horizontal", label = ui.t("bots.gallery.open"), size = "sm", tone = "quiet", on_click = store.gallery },
	}
end

M.create = function()
	store.gallery()
end

for i = 1, 9 do
	ui.command {
		id = `bots.open_{i}`,
		title = ui.t("bots.open_n", { i = i }):get(),
		menu = "Go",
		key = `alt+shift+{i}`,
		run = function()
			local bot = store.pinned()[i]
			if bot then
				store.go(bot)
			end
		end,
	}
end

M.Panel = ui.component(function(): Node
	local pinned = ui.computed(store.pinned)
	local rest = ui.computed(store.unpinned)
	local function open(): store.Bot?
		return store.open:get()
	end
	local function place(id: string): number
		for i, b in pinned:get() do
			if b.id == id then
				return i
			end
		end
		return 0
	end
	local by_id = {
		key = function(b: store.Bot): string
			return b.id
		end,
	}
	return ui.column {
		id = "bots-panel",
		w_full = true,
		h_full = true,
		Header(),
		ui.scroll {
			id = "bots-panel-scroll",
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.column {
				w_full = true,
				px = ui.raw(8, "the mockup's panel inset"),
				pb = "$space.4",
				ui.show(function(): boolean
					return #pinned:get() > 0
				end, ui.row {
					id = "bots-pins",
					w_full = true,
					justify = "center",
					flex_wrap = true,
					gap = ui.raw(2, "the mockup's tiles"),
					pt = ui.raw(2, "the mockup's tiles"),
					pb = ui.raw(8, "the mockup's tiles"),
					ui.each(pinned, function(b: store.Bot, _i: number, row: any): Node
						return Tile {
							row = row,
							index = function(): number
								return place(b.id)
							end,
						}
					end, by_id),
				}),
				ui.each(rest, function(_b: store.Bot, _i: number, row: any): Node
					return Row { row = row }
				end, by_id),
				ui.show(function(): boolean
					return store.list:get().state == "ready" and #pinned:get() == 0 and #rest:get() == 0
				end, ui.text {
					id = "bots-panel-empty",
					ui.t("bots.panel.empty"),
					px = ui.raw(8, "the mockup's group row"),
					py = "$space.3",
					wrap = true,
					text_size = "$font.size.sm",
					color = "$text.tertiary",
				}),
				ui.show(function(): boolean
					return open() ~= nil
				end, ui.column {
					w_full = true,
					Needs(open),
					Sessions(open),
				}),
			},
		},
		EditDialog(),
	}
end)

return M
