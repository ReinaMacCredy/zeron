-- The Skills page (crates/ui/src/shell/skills.rs): every agent's skills as cards, one agent's at a time,
-- and a skill's dialog: its SKILL.md, on or off for `$`, Open, Reveal, Copy Markdown, Uninstall and Try now.
-- zgx keeps its chats' sidebar here: the agents are a row of filters over the cards, not a sidebar of their own.

local ds = require("@ds")
local locale = require("@ds/locale")
local Menu = require("@ds/menu")
local Markdown = require("@ds/markdown")
local motion = require("@ds/motion")
local glass = require("@ds/glass")
local store = require("@app/screens/skills/store")
local route = require("@app/screens/shell/route")
local shared = require("@app/screens/chat/shared")
local chat_store = require("@app/screens/chat/store")
local references = require("@ds/references")

local M = {}

local PAGE_W, PAD_X, TWO_COLUMNS = 880, 28, 560

-- The skill whose dialog is open (its path).
M.open = ui.state("skills_dialog", nil :: string?)
local confirm = ui.state("skills_confirm_uninstall", false)
local menu = ui.state("skills_menu", false)
-- SKILL.md as read: { path, text?, error? }.
local doc = ui.state("skills_doc", nil :: any)

local function ink(): any
	local kind = ui.theme.kind()
	return ui.raw(if kind == "light" or kind == "high-contrast-light" then "#7f22fe" else "#c4b4ff", "gpui's skill tile ink")
end

local function Tile(size: number, round: boolean?): Node
	return ui.div {
		w = size,
		h = size,
		shrink = 0,
		rounded = if round then "$radius.full" else ui.raw(size * 0.3, "gpui's skill tile"),
		bg = "$zeron.secondary.bg",
		border = 1,
		border_color = "$border.subtle",
		items = "center",
		justify = "center",
		ds.Icon { name = "magic-stick", size = math.floor(size * 0.47 + 0.5), color = ink },
	}
end

local function mark_of(id: string, size: number): Node
	for _, h in store.list:peek().harnesses do
		if h.id == id then
			return ds.AgentMark { label = h.name, seed = h.id, mark = if h.icon then nil else h.mark, icon = h.icon, size = size, bare = true }
		end
	end
	return ds.Icon { name = "magic-stick", size = size }
end

function M.close()
	ui.batch(function()
		menu:set(false)
		confirm:set(false)
		M.open:set(nil)
	end)
end

function M.show(path: string)
	ui.batch(function()
		confirm:set(false)
		menu:set(false)
		M.open:set(path)
		doc:set({ path = path })
	end)
	local e = store.find(path)
	if not e or not e.on_disk then
		return
	end
	local ok, p = pcall(store.host.read, path)
	local function fail(why: string)
		if M.open:peek() == path then
			doc:set({ path = path, error = why })
		end
	end
	if not ok or p == nil then
		fail(locale.s("skills.old_host"))
		return
	end
	p:after(function(text: any)
		if M.open:peek() == path then
			doc:set({ path = path, text = tostring(text) })
		end
	end, function(err: any)
		fail(if err and err.message then tostring(err.message) else "failed")
	end)
end

-- A new chat on an agent that has it (the one shown, if it does), the skill first in its draft.
function M.try(e: store.Entry)
	local f = store.filter:peek()
	local allowed = if f and table.find(e.harnesses, f) then { f } else e.harnesses
	local defaults = chat_store.new_defaults
	local model = if defaults then defaults:peek().model else nil
	local now: string? = nil
	for _, m in chat_store.catalog:peek().models or {} do
		if model ~= nil and m.value == model then
			now = m.harness
		end
	end
	local agent = if now and table.find(allowed, now) then now else allowed[1]
	local link = (agent and e.links[agent]) or e.link
	M.close()
	route.new_chat()
	if agent then
		shared.want_agent:set(agent)
	end
	local draft = shared.draft_peek("").text
	local shown = references.add(shared.refs_key(""), { kind = "skill", name = e.name, link = link })
	shared.set_draft_of("", "text", `{shown} {draft}`)
	ui.after(0, function()
		shared.focus:set(shared.focus:peek() + 1)
	end)
end

local function act(what: string, e: store.Entry)
	menu:set(false)
	if what == "copy" then
		local d = doc:peek()
		if d and d.path == e.path and d.text then
			ui.clipboard.write(d.text)
			ds.toast.show { title = locale.s("skills.copied"), tone = "success" }
		end
		return
	end
	local ok, p = pcall(if what == "open" then store.host.open else store.host.reveal, e.path)
	if ok and p then
		p:after(function() end, function(err: any)
			ds.toast.show { title = if err and err.message then tostring(err.message) else locale.s("skills.couldnt_open"), tone = "danger" }
		end)
	end
end

local function uninstall(e: store.Entry)
	local ok, p = pcall(store.host.trash, e.paths)
	local function failed(why: string)
		ds.toast.show { title = locale.s("skills.couldnt_trash", { why = why }), tone = "danger" }
	end
	if not ok or p == nil then
		failed(locale.s("skills.old_host"))
		return
	end
	p:after(function()
		store.set_enabled(e, true)
		local now = store.list:peek()
		local left = {}
		for _, x in now.entries do
			if x.path ~= e.path then
				table.insert(left, x)
			end
		end
		store.list:set({ state = now.state, harnesses = now.harnesses, entries = left, error = now.error })
		M.close()
		store.load()
	end, function(err: any)
		failed(if err and err.message then tostring(err.message) else "failed")
	end)
end

local function Filter(id: string?, label: () -> string, count: () -> number, glyph: Node): Node
	local function lit(): boolean
		return store.filter:get() == id
	end
	return ui.button {
		id = `skills-tab-{id or "all"}`,
		role = "tab",
		label = label,
		selected = lit,
		h = 30,
		px = ui.raw(10, "gpui's section tab"),
		gap = ui.raw(8, "gpui's section tab"),
		rounded = "$radius.md",
		items = "center",
		bg = function(): string
			return if lit() then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		color = function(): string
			return if lit() then "$text.primary" else "$text.secondary"
		end,
		on_click = function()
			store.filter:set(id)
		end,
		glyph,
		ui.text { label, text_size = "$font.size.md", wrap = false },
		ui.text { function(): string
			return tostring(count())
		end, text_size = "$font.size.sm", color = "$text.tertiary", tabular = true },
	}
end

local function Filters(): Node
	return ui.presence(function(): string
		local ids = {}
		for _, h in store.list:get().harnesses do
			table.insert(ids, h.id)
		end
		return table.concat(ids, ",")
	end, function(key: string): Node
		local row: { any } = {
			Filter(nil, function(): string
				return locale.s("skills.all")
			end, function(): number
				return #store.shown(nil)
			end, ds.Icon { name = "magic-stick", size = 16 }),
		}
		for id in string.gmatch(key, "[^,]+") do
			table.insert(row, Filter(id, function(): string
				return store.harness_name(id)
			end, function(): number
				return #store.shown(id)
			end, mark_of(id, 16)))
		end
		return ui.row { id = "skills-tabs", role = "tablist", label = locale.s("skills.harnesses"), gap = ui.raw(2, "gpui"), wrap = true, table.unpack(row) }
	end, { w_full = true })
end

local function Card(e: store.Entry, id: string): Node
	return ui.button {
		id = id,
		label = e.title,
		w_full = true,
		min_w = 0,
		p = ui.raw(12, "gpui"),
		gap = ui.raw(12, "gpui"),
		rounded = ui.raw(12, "gpui"),
		items = "center",
		bg = "$zeron.secondary.bg",
		hover = { bg = "$hover" },
		on_click = function()
			M.show(e.path)
		end,
		Tile(34),
		ui.column {
			grow = 1,
			min_w = 0,
			gap = ui.raw(4, "gpui"),
			ui.text { e.title, literal = true, truncate = true, text_size = ui.raw(13, "gpui"), weight = "$font.weight.medium", color = "$text.primary" },
			ui.text { e.description, literal = true, truncate = true, text_size = ui.raw(12, "gpui"), color = "$text.secondary" },
		},
	}
end

local function Grid(width: State<number>): Node
	return ui.presence(function(): string
		local f = store.filter:get() or ""
		local paths = {}
		for _, e in store.shown(store.filter:get()) do
			table.insert(paths, e.path)
		end
		local two = math.min(width:get(), PAGE_W) - 2 * PAD_X >= TWO_COLUMNS
		return `{f}|{if two then 2 else 1}|{table.concat(paths, "\n")}`
	end, function(key: string): Node
		local columns = tonumber(string.match(key, "^[^|]*|(%d)")) or 1
		local list = store.shown(store.filter:peek())
		-- (Two skills of one name, their bytes apart, are two cards: the later ones numbered, as every agent's list has them.)
		local ids: { [string]: string } = {}
		local seen: { [string]: number } = {}
		for _, e in store.shown(nil) do
			local n = (seen[e.name] or 0) + 1
			seen[e.name] = n
			ids[e.path] = if n == 1 then `skills-card-{e.name}` else `skills-card-{e.name}-{n}`
		end
		local rows: { any } = {}
		for i = 1, #list, columns do
			local cells: { any } = {}
			for j = 0, columns - 1 do
				local e = list[i + j]
				table.insert(cells, ui.div { grow = 1, basis = 0, min_w = 0, if e then Card(e, ids[e.path] or `skills-card-{e.name}`) else nil })
			end
			table.insert(rows, ui.row { w_full = true, gap = "$space.2", table.unpack(cells) })
		end
		return ui.column {
			id = "skills-grid",
			w_full = true,
			gap = "$space.2",
			table.unpack(rows),
		}
	end, { w_full = true, enter = motion.transition({ opacity = { 0, 1 }, duration = 150, ease = "out(3)" }) })
end

local function Badge(id: string): Node
	return ui.row {
		h = 24,
		px = ui.raw(10, "gpui"),
		gap = ui.raw(5, "gpui"),
		rounded = "$radius.full",
		bg = "$zeron.secondary.bg",
		items = "center",
		text_size = ui.raw(12, "gpui"),
		color = "$text.secondary",
		mark_of(id, 12),
		ui.text { store.harness_name(id), literal = true, wrap = false },
	}
end

local function IconButton(id: string, icon: string, word: string, on_click: () -> (), extra: { [string]: any }?): Node
	local p: { [any]: any } = {
		id = id,
		label = locale.s(word),
		tooltip = locale.s(word),
		w = 28,
		h = 28,
		p = 0,
		shrink = 0,
		rounded = "$radius.full",
		bg = "#00000000",
		hover = { bg = "$hover" },
		items = "center",
		justify = "center",
		color = "$text.secondary",
		on_click = on_click,
		ds.Icon { name = icon :: any, size = 16 },
	}
	for k, v in extra or {} do
		p[k] = v
	end
	return ui.button(p)
end

local function Body(e: store.Entry): Node
	return ui.presence(function(): string
		local d = doc:get()
		if d and d.path == e.path and d.text then
			return "text"
		elseif d and d.path == e.path and d.error then
			return "error"
		end
		return if e.on_disk then "loading" else "native"
	end, function(state: string): Node
		if state == "text" then
			return Markdown { text = store.body(doc:peek().text) }
		end
		local words = if state == "error"
			then locale.s("skills.couldnt_read", { why = doc:peek().error })
			elseif state == "loading" then locale.s("skills.reading")
			else locale.s("skills.native")
		return ui.text { words, literal = true, wrap = true, color = "$text.secondary" }
	end, { w_full = true })
end

local function Dialog(e: store.Entry): Node
	local on = ui.use_state(store.enabled(e, true))
	ui.effect(function()
		local v = store.enabled(e)
		if on:peek() ~= v then
			on:set(v)
		end
	end)
	local more = if e.on_disk
		then IconButton("skill-dialog-more", "more-horizontal", "skills.more", function()
			menu:set(not menu:peek())
		end, { expanded = function(): boolean
			return menu:get()
		end })
		else nil
	local head: { any } = {
		gap = ui.raw(6, "gpui"),
		items = "center",
		Tile(40, true),
		ui.div { grow = 1 },
		ui.div { id = "skill-dialog-toggle", mr = ui.raw(4, "gpui"), ds.Switch { value = on, label = locale.s("skills.use"), on_change = function(v: boolean)
			store.set_enabled(e, v)
		end } },
	}
	if more then
		table.insert(head, ui.popover(Menu.panel {
			open = menu,
			anchor = more,
			w = 200,
			placement = { side = "bottom", align = "end", offset = 4 },
			build = function(): Node
				return Menu.Group {
					Menu.Row { id = "skill-menu-open", icon = "arrow-up-right", label = locale.s("skills.open"), on_click = function()
						act("open", e)
					end },
					Menu.Row { id = "skill-menu-reveal", icon = "folder", label = locale.s("skills.reveal"), on_click = function()
						act("reveal", e)
					end },
					Menu.Row { id = "skill-menu-copy", icon = "copy", label = locale.s("skills.copy"), on_click = function()
						act("copy", e)
					end },
				}
			end,
		}))
	end
	table.insert(head, IconButton("skill-dialog-close", "close", "skills.close", M.close))
	local badges: { any } = { gap = ui.raw(6, "gpui"), items = "center", wrap = true }
	for _, h in e.harnesses do
		table.insert(badges, Badge(h))
	end
	if e.on_disk then
		table.insert(badges, ui.text { e.shown, literal = true, ml = ui.raw(4, "gpui"), font = "$font.mono", text_size = ui.raw(12, "gpui"), color = "$text.tertiary" })
	end
	local foot = ui.presence(function(): string
		return if confirm:get() then "confirm" else "plain"
	end, function(state: string): Node
		if state == "confirm" then
			return ui.row {
				gap = ui.raw(8, "gpui"),
				items = "center",
				ui.text { locale.s("skills.uninstall_says"), grow = 1, wrap = true, text_size = ui.raw(12.5, "gpui"), color = "$text.secondary" },
				ds.Button { locale.t("common.cancel"), id = "skill-uninstall-cancel", tone = "secondary", size = "sm", on_click = function()
					confirm:set(false)
				end },
				ds.Button { locale.t("skills.move_to_trash"), id = "skill-uninstall-confirm", tone = "danger", size = "sm", on_click = function()
					uninstall(e)
				end },
			}
		end
		return ui.row {
			items = "center",
			if e.on_disk
				then ui.button {
					id = "skill-uninstall",
					label = locale.s("skills.uninstall"),
					px = ui.raw(4, "gpui"),
					py = ui.raw(6, "gpui"),
					bg = "#00000000",
					hover = { opacity = 0.8 },
					color = "$danger",
					on_click = function()
						confirm:set(true)
					end,
					ui.text { locale.s("skills.uninstall"), text_size = ui.raw(13, "gpui") },
				}
				else nil,
			ui.div { grow = 1 },
			ui.button {
				id = "skill-try-now",
				label = locale.s("skills.try"),
				h = 32,
				px = ui.raw(14, "gpui"),
				gap = ui.raw(7, "gpui"),
				rounded = "$radius.full",
				bg = "$text.primary",
				hover = { opacity = 0.9 },
				color = "$background",
				items = "center",
				on_click = function()
					M.try(e)
				end,
				ds.Icon { name = "pen-new-square", size = 14 },
				ui.text { locale.s("skills.try"), text_size = ui.raw(13, "gpui"), weight = "$font.weight.medium" },
			},
		}
	end, { w_full = true })
	return ui.column(glass.style({
		id = "skill-dialog",
		role = "dialog",
		label = e.title,
		w_full = true,
		px = ui.raw(24, "gpui"),
		pt = ui.raw(22, "gpui"),
		pb = ui.raw(20, "gpui"),
		rounded = ui.raw(18, "gpui"),
		ui.row(head),
		ui.column {
			mt = ui.raw(16, "gpui"),
			gap = ui.raw(2, "gpui"),
			ui.text { e.title, literal = true, heading = 2, wrap = true, text_size = ui.raw(20, "gpui"), weight = "$font.weight.semibold", color = "$text.primary" },
			ui.text { locale.s("skills.kind"), text_size = ui.raw(13, "gpui"), color = "$text.tertiary" },
		},
		if e.description ~= ""
			then ui.text { e.description, literal = true, wrap = true, lines = 3, mt = ui.raw(6, "gpui"), text_size = ui.raw(13, "gpui"), line_height = ui.raw(19, "gpui"), color = "$text.secondary" }
			else nil,
		ui.row(badges),
		ui.scroll {
			id = "skill-doc",
			mt = ui.raw(16, "gpui"),
			w_full = true,
			max_h = function(): number
				local ok, size = pcall(function(): any
					return (ui :: any).window.size:get()
				end)
				local h = if ok and type(size) == "table" and type(size.height) == "number" then size.height else 0
				return math.clamp(h * 0.42, 160, 380)
			end,
			px = ui.raw(16, "gpui"),
			py = ui.raw(14, "gpui"),
			rounded = ui.raw(12, "gpui"),
			bg = "$zeron.secondary.bg",
			text_size = ui.raw(13, "gpui"),
			Body(e),
		},
		ui.div { mt = ui.raw(18, "gpui"), w_full = true, foot },
	}, "popup"))
end

local function Overlay(): Node
	return ui.modal {
		open = function(): boolean
			return M.open:get() ~= nil and store.find(M.open:get()) ~= nil
		end,
		-- (Escape: the Uninstall question first, then the dialog; the menu, a panel of its own, closes itself.)
		on_close = function()
			if confirm:peek() then
				confirm:set(false)
			else
				M.close()
			end
		end,
		label = function(): string
			local e = store.find(M.open:get())
			return if e then e.title else locale.s("skills.title")
		end,
		w = 580,
		p = 0,
		bg = "#00000000",
		border = 0,
		rounded = ui.raw(18, "gpui"),
		build = function(): Node
			return ui.presence(function(): string
				return M.open:get() or ""
			end, function(path: string): Node
				local e = store.find(path)
				return if e then Dialog(e) else ui.div {}
			end, { w_full = true })
		end,
	}
end

M.Page = ui.component(function(): Node
	store.load()
	local width = ui.use_state(0)
	local function title(): string
		local f = store.filter:get()
		return if f then store.harness_name(f) else locale.s("skills.all")
	end
	local function subtitle(): string
		local l = store.list:get()
		if l.state == "ready" then
			return locale.s("skills.count", { n = #store.shown(store.filter:get()) })
		elseif l.state == "failed" then
			return locale.s("skills.couldnt_load", { why = l.error or "" })
		end
		return locale.s("skills.loading")
	end
	return ui.div {
		id = "skills-root",
		w_full = true,
		h_full = true,
		ui.scroll {
			id = "skills-page",
			w_full = true,
			h_full = true,
			items = "center",
			on_resize = function(size: { w: number, h: number })
				if math.abs(width:peek() - size.w) >= 1 then
					width:set(size.w)
				end
			end,
			ui.column {
				w_full = true,
				max_w = PAGE_W,
				px = ui.raw(PAD_X, "gpui's page column"),
				pt = ui.raw(16, "gpui's page column"),
				pb = ui.raw(48, "gpui"),
				gap = ui.raw(20, "gpui"),
				ui.column {
					px = ui.raw(8, "gpui's page header"),
					ui.text { title, id = "skills-title", role = "heading", literal = true, text_size = ui.raw(20, "gpui"), line_height = ui.raw(26, "gpui"), weight = "$font.weight.medium", color = "$text.primary" },
					ui.text { subtitle, id = "skills-subtitle", literal = true, wrap = true, mt = ui.raw(4, "gpui"), text_size = ui.raw(13, "gpui"), line_height = ui.raw(17, "gpui"), color = "$text.secondary" },
				},
				Filters(),
				ui.show(function(): boolean
					return store.list:get().state == "ready" and #store.shown(store.filter:get()) == 0
				end, ui.text { locale.s("skills.none"), id = "skills-none", text_size = ui.raw(13, "gpui"), color = "$text.secondary" }),
				Grid(width),
			},
		},
		Overlay(),
	}
end)

return M
