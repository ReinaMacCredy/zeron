-- Settings, in the main window's page: its pages listed at the left, the
-- one open beside them (the route's `settings/:page`). The shell's own is
-- Archived; screens/settings adds the rest (General, Account, …), and
-- others add theirs:
--
--   local settings = require("@app/screens/shell/settings")
--   settings.add { id = "plugins", title = "Plugins", icon = "widget", view = function() return … end }
--   settings.add { id = "general", title = "General", module = "@app/screens/settings/general" }
--
-- An app module adds a page as it loads (the Plugins page does), in the
-- order added, after the app's own; `before = "account"` puts it ahead of
-- one; `group = "Personal"` lists it under that heading. `settings.head(fn)`
-- puts what `fn` builds over the rows (the settings search's field).
-- Plugins add rows under the list in its slot (`settings.nav`) and place
-- by `#settings-nav`, `.settings-nav-row` and `#settings-page`. Account is
-- out of their reach.
--
-- A `module` page loads it as it's first shown or searched (`settings.load_all()`), not with the script.
--
-- Pages that come and go (a plugin's settings, while it's on) are listed
-- by `settings.extra(name, fn)`: `fn` returns them as they are now (read
-- where it reads states, so the list follows), after the app's own.
--
-- `settings.Panel()`: settings floating over the chats while the route is
-- `settings/:page` (the shell's). `settings.View()`: settings, a page of its
-- own over the whole window (a template's shell); `{ panel = true }`, what
-- the panel holds.
--
-- `settings.title_of(id, fallback)`: a page's title in the locale shown, a
-- string read now (`title` stays the English it's kept and compared by).

local ds = require("@ds")
local motion = require("@ds/motion")
local dialog = require("@ds/dialog")
local route = require("@app/screens/shell/route")
local boundary = require("@app/screens/shell/boundary")
local refocus = require("@app/screens/shell/refocus")

local M = {}

export type Page = {
	id: string,
	title: string,
	icon: string?,
	-- What it shows: built as it opens (a page another opens in its place).
	view: (() -> Node)?,
	-- Or the module whose `view` it is, loaded as the page is first shown.
	module: string?,
	-- Put ahead of this page (an id), else after the rest.
	before: string?,
	-- Out of plugins' reach (an account's, anything that spends or
	-- deletes).
	private: boolean?,
	-- The heading it's listed under ("Personal", "Devices", …): a heading
	-- shows where the group changes, down the list's order.
	group: string?,
	-- In the icon's place in the list: a tile (a plugin's, as its row on
	-- the Plugins page has it).
	tile: { label: string, hue: string? }?,
	-- Listed, found and opened only while this says so (a built-in's page, while it's on).
	shown: (() -> boolean)?,
}

-- The pages: their ids in order (a state, so the list follows them), and
-- each by id (a view is a function: state holds data alone).
local order = ui.state("shell_settings_pages", {} :: { string })
local pages: { [string]: Page } = {}
M.pages = order

-- Pages that come and go, by who lists them (`M.extra`).
local extras: { [string]: () -> { Page } } = {}
local extra_names: { string } = {}

function M.extra(name: string, list: () -> { Page })
	if not extras[name] then
		table.insert(extra_names, name)
	end
	extras[name] = list
end

-- Every page that comes and goes, now (followed where it's read).
local function extra_pages(): { Page }
	local out: { Page } = {}
	for _, name in extra_names do
		for _, page in extras[name]() do
			table.insert(out, page)
		end
	end
	return out
end

-- Whether page `id` is listed now (tracked).
function M.shown(id: string): boolean
	local page = pages[id]
	return not (page and page.shown) or (page.shown :: () -> boolean)()
end

-- A page added, by its id (or one listed by `extra`, while it's there).
function M.page(id: string): Page?
	if pages[id] then
		return if M.shown(id) then pages[id] else nil
	end
	for _, page in extra_pages() do
		if page.id == id then
			return page
		end
	end
	return nil
end

-- What heads the list, under "Back to chats" and over the pages (the
-- settings search's field), built with the list; while `opts.replaces()`,
-- the pages are put away (its results, drawn in it, stand in their place).
local head: (() -> Node)? = nil
local replaces: (() -> boolean)? = nil
function M.head(build: () -> Node, opts: { replaces: (() -> boolean)? }?)
	head = build
	replaces = if opts then opts.replaces else nil
end

-- Where the open page is scrolled (a search's hit sets it, to land on its
-- row); a page chosen in the list opens at its top.
M.scroll = ui.state("shell_settings_scroll", 0)

local modules: { [string]: any } = {}
local failures: { [string]: string } = {}
local loads = ui.state("shell_settings_loads", 0)

-- (From a timer alone: a page's effects and computeds are then the script's, not a build's that goes.)
local function load(id: string)
	local page = pages[id]
	if not page or not page.module or modules[id] ~= nil then
		return
	end
	local found, why = boundary.load(page.module)
	modules[id] = if found ~= nil then found else false
	if found == nil then
		failures[id] = why or ""
	end
	loads:set(loads:peek() + 1)
end

-- Whether a page can be built now (its module, if it has one, loaded).
local function ready(id: string): boolean
	loads:get()
	local page = pages[id]
	return not (page and page.module) or modules[id] ~= nil
end

function M.add(page: Page)
	pages[page.id] = page
	if page.module and not page.view then
		local id = page.id
		page.view = function(): Node
			local m = modules[id]
			if not m then
				return boundary.Failed { name = page.title, error = failures[id] }
			end
			return m.view()
		end
	end
	-- (Listed once: the list outlives a reload of the scripts, as the
	-- pages are added again.)
	if table.find(order:peek(), page.id) then
		return
	end
	local ids = table.clone(order:peek())
	local at = if page.before then table.find(ids, page.before) else nil
	table.insert(ids, at or #ids + 1, page.id)
	order:set(ids)
end

-- The list's width (the sidebar's least), and the title bar's strip over
-- the window (Zeron's: the window's buttons sit in it).
local TITLE_H = 38
-- The group the pages that come and go list ahead of.
local EXTRAS_BEFORE = "Zeron"

-- Archived: sessions put away (screens/settings/archived), last.
M.add {
	id = "archived",
	title = "Archived",
	icon = "archive-minimalistic",
	-- (Under a heading of its own, last: Zeron keeps it in its last run of
	-- pages, apart from the account's and the organization's.)
	group = "Workspace",
	module = "@app/screens/settings/archived",
}

-- The route's page, held after settings closes so the panel leaves showing
-- the page it had, not the first one.
local held = ui.state("shell_settings_held_page", "")
ui.effect(function()
	local r = route.current:get()
	if r.name == "settings" and r.page and held:peek() ~= r.page then
		held:set(r.page)
	end
end)
local function route_page(): string?
	local r = route.current:get()
	return if r.name == "settings" then r.page else held:get()
end

-- The page open now: the route's, else (gone, or not added yet) the first.
local function open_id(): string
	local id = route_page()
	if id and M.page(id) then
		return id
	end
	for _, first in order:get() do
		if M.page(first) then
			return first
		end
	end
	return ""
end

-- (`ui.after(0)`: loaded before the next frame is drawn.)
ui.effect(function()
	if route.current:get().name ~= "settings" then
		return
	end
	local id = open_id()
	if id ~= "" and not ready(id) then
		ui.after(0, function()
			load(id)
		end)
	end
end)

-- (A few a turn, each turn well inside a call's budget.)
local all = ui.state("shell_settings_load_all", false)
function M.load_all()
	if not all:peek() then
		all:set(true)
	end
end
local function load_some()
	local start = ui.now()
	for _, id in order:peek() do
		if modules[id] == nil and pages[id] and pages[id].module then
			if ui.now() - start > 12 then
				ui.after(0, load_some)
				return
			end
			load(id)
		end
	end
end
ui.effect(function()
	if all:get() then
		ui.after(0, load_some)
	end
end)

-- What "Back to chats" is among the list's values.
local BACK = "::back"

-- The list's names in the language shown, looked up as it's drawn: a
-- page's title and a group's stay English where they're kept and compared.
-- A page by its id, and only while it's called what the app calls it (a
-- page another adds, or one in a page's place under another name, keeps
-- the name it was given); a heading by its English.
local TITLE_KEYS: { [string]: { string } } = {
	general = { "General", "settings.general.title" },
	appearance = { "Appearance", "settings.appearance.title" },
	activity = { "Activity", "settings.activity.title" },
	notifications = { "Notifications", "settings.notifications.title" },
	voice = { "Voice", "settings.voice.title" },
	files = { "Files", "settings.files.title" },
	browser = { "Browser", "settings.shell.group.browser" },
	shortcuts = { "Shortcuts", "settings.shortcuts.title" },
	appshots = { "Appshots", "settings.appshots.title" },
	computer = { "Computer use", "settings.computer.title" },
	voice = { "Voice", "settings.voice.title" },
	usage = { "Usage", "settings.usage.title" },
	compaction = { "Compaction", "settings.compaction.title" },
	code_review = { "Code review", "settings.code_review.title" },
	providers = { "Providers", "settings.providers.title" },
	["agent-profiles"] = { "Agent profiles", "settings.agent_profiles.title" },
	routing = { "Routing", "settings.routing.title" },
	mcp = { "MCP", "settings.mcp.title" },
	devices = { "Devices", "settings.devices.title" },
	device_hub = { "Device hub", "settings.device_hub.title" },
	projects = { "Projects", "settings.projects.title" },
	members = { "Members", "settings.members.title" },
	profile = { "Account", "settings.profile.title" },
	cloud = { "Cloud", "settings.cloud.title" },
	billing = { "Billing", "settings.billing.title" },
	about = { "About & updates", "settings.about.title" },
	archived = { "Archived", "settings.archived.title" },
	experimental = { "Experimental", "settings.lab.title" },
	tools = { "Tools", "settings.tools.title" },
	plugins = { "Plugins", "plugins.page.title" },
	templates = { "Templates", "templates.page.title" },
}
local GROUP_KEYS: { [string]: string } = {
	Personal = "settings.shell.group.personal",
	Browser = "settings.shell.group.browser",
	Devices = "settings.shell.group.devices",
	Organization = "settings.shell.group.organization",
	Workspace = "settings.shell.group.workspace",
	Cloud = "settings.shell.group.cloud",
	Customize = "settings.shell.group.customize",
	Plugins = "settings.shell.group.plugins",
	Lab = "settings.shell.group.lab",
}
local function title_of(id: string, title: string): string
	local known = TITLE_KEYS[id]
	return if known and known[1] == title then ui.t(known[2]):get() else title
end
local function group_of(group: string): string
	local key = GROUP_KEYS[group]
	return if key then ui.t(key):get() else group
end

-- A page's title in the locale shown, read now (tracked in a function or
-- computed): the page added by that id, else `fallback` (its English), as
-- the list draws it.
function M.title_of(id: string, fallback: string?): string
	local page = M.page(id)
	return title_of(id, if page then page.title else (fallback or id))
end

-- The panel's measures (Claude's settings): its most and the room it keeps
-- from the window's edges, its head's height, the list's width and the
-- page's measure.
local PANEL_W, PANEL_H, PANEL_INSET = 1020, 750, 48
local HEAD_H = 48
local PANEL_NAV_W = 212
local PANEL_MEASURE = 720

-- The page's column, the last one built: a search's hit scrolls by where
-- its row is in it.
local page_column: any = nil
function M.page_top(): number
	local el = if page_column then page_column:peek() else nil
	return if el and el.y then el.y else 0
end

-- Settings, a page of its own over the whole window: its pages listed
-- down the window's full height (the window's buttons in the list's top
-- strip, as a macOS sidebar has them), "Back to chats" at the list's head,
-- the page scrolling beside it under a strip of its own. The gallery's
-- settings window (demos/example_kit `settings`). Esc, ⌘[ and Back to
-- chats go back where the chats were, as they were.
-- `panel`: what the floating panel holds (`M.Panel`): its head (the title
-- and its ×), the list at a fixed width under it, no Back (the chats show
-- round it), the page centred at a measure beside it.
function M.View(props: { panel: boolean? }?): Node
	local panel = props ~= nil and props.panel == true
	-- Where the page is scrolled, as the view has it: `M.scroll` as it was,
	-- put back until a layout keeps it. (A view made again, the scripts
	-- reloaded, lays the page out before its rows have come; its scroll,
	-- clamped to that, would otherwise write 0 over where it was.) Once
	-- it holds, each follows the other: a scroll here, a search's reveal
	-- there.
	-- Put back as the page lays out (its rows coming), until a layout has
	-- room for it; or, never that tall now, as soon as the person scrolls.
	local target = M.scroll:peek()
	local shown = ui.use_state(target)
	local settled = target <= 0
	local view = ui.ref()
	local column = ui.ref()
	page_column = column
	local function laid_out(h: number)
		if settled then
			return
		end
		shown:set(target)
		if h - (view.height or 0) >= target - 0.5 then
			settled = true
			M.scroll:set(target)
		end
	end
	local function scrolled(): boolean
		if not settled then
			settled = true
			M.scroll:set(shown:peek())
		end
		return false
	end
	ui.effect(function()
		local y = shown:get()
		if settled and math.abs(M.scroll:peek() - y) > 0.5 then
			M.scroll:set(y)
		end
	end)
	ui.effect(function()
		local want = M.scroll:get()
		if settled and math.abs(want - shown:peek()) > 0.5 then
			shown:set(want)
		end
	end)
	local body = ui.row {
		id = "settings",
		w_full = true,
		h_full = not panel,
		grow = if panel then 1 else nil,
		min_h = 0,
		-- The pages (the design system's list of them; a page added later
		-- joins it in place), and what plugins add under them.
		ui.div {
			id = "settings-nav",
			hook = "settings.nav",
			h_full = true,
			shrink = 0,
			relative = true,
			ds.Settings.Nav {
				value = ui.computed(route_page) :: any,
				on_choose = function(id: string)
					if id == BACK then
						route.leave_settings()
					else
						M.scroll:set(0)
						route.settings(id)
					end
				end,
				w = if panel then PANEL_NAV_W else nil,
				-- (The main sidebar's width, shared and dragged in either; its
				-- bounds. Asked for here: both are loaded by now.)
				width = if panel then nil else require("@app/screens/shell/sidebar").width,
				min_w = if panel then nil else require("@app/screens/shell/fit").SIDEBAR_MIN,
				max_w = if panel then nil else require("@app/screens/shell/fit").SIDEBAR_MAX,
				top = if panel then nil else TITLE_H,
				lead = if panel then nil else { { value = BACK, label = ui.t("settings.shell.back") :: any, icon = "arrow-left" } },
				items = function(): { any }
					local out: { any } = {}
					-- (Under one heading, "Settings", while no page has a group
					-- of its own; else each group's, where the group changes.)
					local grouped = false
					for _, id in order:get() do
						grouped = grouped or (pages[id] ~= nil and pages[id].group ~= nil)
					end
					if not grouped then
						table.insert(out, { heading = ui.t("settings.shell.heading"):get() })
					end
					local last: string? = nil
					-- Those that come and go (plugins' pages), under their own
					-- headings: ahead of the "Zeron" group (About and updates
					-- stay last), or at the end where there's none.
					local extras_in = false
					local function add_extras()
						if extras_in then
							return
						end
						extras_in = true
						for _, page in extra_pages() do
							if page.group and page.group ~= last then
								table.insert(out, { heading = group_of(page.group) })
							end
							last = page.group
							table.insert(out, { value = page.id, label = title_of(page.id, page.title), icon = page.icon, tile = page.tile, class = "settings-nav-row" })
						end
					end
					for _, id in order:get() do
						local page = if M.shown(id) then pages[id] else nil
						if page then
							if page.group == EXTRAS_BEFORE then
								add_extras()
							end
							if page.group and page.group ~= last then
								table.insert(out, { heading = group_of(page.group) })
							end
							last = page.group
							table.insert(out, { value = id, label = title_of(id, page.title), icon = page.icon, class = "settings-nav-row" })
						end
					end
					add_extras()
					return out
				end,
				head = if head then head() else nil,
				hide_rows = replaces,
				foot = ui.slot("settings.nav", { w_full = true, gap = "$zeron.list.gap" }),
				-- You, as on the rail: you open your profile.
				account = (function(): Node?
					local rail: any = require("@app/screens/shell/rail")
					return ds.AccountRow {
						name = function(): string
							return rail.user().name
						end,
						id = function(): string?
							return rail.user().id
						end,
						avatar = function(): string?
							return rail.user().avatar
						end,
						on_user = rail.account.on_user,
						user_open = rail.account.user_open,
					}
				end)(),
			},
			-- Its strip, with the window's buttons in it, moves the window.
			if panel then nil else ui.div { id = "settings-nav-drag", absolute = true, left = 0, right = 0, top = 0, h = TITLE_H, window_drag = true },
		},
		-- The page open, under its own strip (another page is another page:
		-- it swaps).
		ui.column {
			grow = 1,
			min_w = 0,
			h_full = true,
			-- (Its strip moves the window, a double click zooming it.)
			if panel then nil else ui.div { id = "settings-drag", h = TITLE_H, w_full = true, shrink = 0, window_drag = true },
			ui.scroll {
				id = "settings-scroll",
				w_full = true,
				grow = 1,
				min_h = 0,
				fade = { 16, 16 },
				fade_overflow = true,
				offset = shown,
				ref = view,
				on_wheel = scrolled,
				ui.column {
					id = "settings-page",
					ref = column,
					hook = "settings.page",
					-- (Laid out, as tall as it is: where it was scrolled again, the
					-- scroll's room known now.)
					on_resize = function(size)
						laid_out(size.h)
					end,
					w_full = true,
					px = if panel then ui.raw(28, "the panel's page inset, Claude's") else "$space.6",
					pt = if panel then ui.raw(22, "the panel's page inset, Claude's") else "$space.4",
					pb = if panel then ui.raw(40, "the panel's page inset, Claude's") else "$space.8",
					items = "center",
					ui.presence(function(): string
						local id = open_id()
						return if id ~= "" and ready(id) then id else ""
					end, function(id: string): Node
						if id == "" then
							return ui.div {}
						end
						local page = M.page(id) :: Page
						-- Every page centred in the room beside the list, however
						-- it's built (the presence's own `items` doesn't reach
						-- inside a page's node); a private one out of plugins' reach.
						return ui.column {
							hook = if page.private then false else nil,
							w_full = true,
							max_w = if panel then ui.raw(PANEL_MEASURE, "the panel's page measure, Claude's") else nil,
							items = "center",
							(page.view :: () -> Node)(),
						}
					end, {
						w_full = true,
						items = "center",
						-- (A page chosen rises into place.)
						enter = if panel then motion.transition({ opacity = { 0, 1 }, y = { 4, 0 }, duration = 160, ease = "cubicBezier(0.2, 0.8, 0.2, 1)" }) else nil,
					}),
				},
			},
		},
	}
	if not panel then
		return body
	end
	return ui.column {
		w_full = true,
		h_full = true,
		min_h = 0,
		ui.row {
			id = "settings-head",
			w_full = true,
			h = ui.raw(HEAD_H, "the panel's head, Claude's"),
			shrink = 0,
			pl = ui.raw(18, "the title in from the panel's edge, Claude's"),
			pr = "$space.2",
			items = "center",
			justify = "between",
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border",
			ui.text { ui.t("settings.shell.heading"), heading = 1, text_size = "$font.size.md", weight = "$font.weight.semibold", color = "$text.primary" },
			ds.IconButton {
				id = "settings-close",
				icon = "close",
				size = "lg",
				label = ui.t("common.close") :: any,
				tooltip = false,
				on_click = function()
					route.leave_settings()
				end,
			},
		},
		body,
	}
end

-- The window's size, followed.
local function window_size(): (number, number)
	local size = (ui.window.size :: any):get()
	return math.max(1, size.width or 0), math.max(1, size.height or 0)
end

-- How the panel comes (Claude's: 210 ms, easing out) and goes (140, easing
-- in), and the scrim's fade.
local OPEN = { duration = 210, ease = "cubicBezier(0.2, 0.8, 0.2, 1)" }
local CLOSE = { duration = 140, ease = "cubicBezier(0.4, 0, 1, 1)" }
local SCRIM = { duration = 180, ease = "cubicBezier(0.2, 0.8, 0.2, 1)" }

-- Settings floating over the chats while the route is `settings/:page`, as
-- Claude's: the window dimmed under a scrim, the panel centred over it at
-- `PANEL_W` by `PANEL_H`, shrinking with the window to keep `PANEL_INSET`
-- from its edges, frosted as a dialog is. zgx's modal under it all (focus
-- held in it, given back as it goes), its own dim made clear: the scrim is
-- this one's and fades with the panel. Escape, a click on the scrim, the ×
-- and ⌘, again close it (the route leaves); it stays up until it has gone.
M.inside = ui.ref()
M.Panel = ui.component(function(): Node
	local function open(): boolean
		return route.current:get().name == "settings"
	end
	local now = route.current:peek().name == "settings"
	local phase = ui.use_state(if now then 1 else 0)
	local dim = ui.use_state(if now then 1 else 0)
	local up = ui.use_state(now)
	local moving: { any } = {}
	ui.effect(function()
		local want = open()
		for _, m in moving do
			m:cancel()
		end
		moving = {}
		if want then
			up:set(true)
			table.insert(moving, anime.animate(phase, motion.params({ value = 1, duration = OPEN.duration, ease = OPEN.ease })))
			table.insert(moving, anime.animate(dim, motion.params({ value = 1, duration = SCRIM.duration, ease = SCRIM.ease })))
		elseif up:peek() then
			table.insert(moving, anime.animate(phase, motion.params({ value = 0, duration = CLOSE.duration, ease = CLOSE.ease })))
			table.insert(
				moving,
				anime.animate(
					dim,
					motion.params({
						value = 0,
						duration = SCRIM.duration,
						ease = SCRIM.ease,
						-- (Gone, the keys go back to the chats: the modal gives focus back
						-- where it was as it closes, often nowhere.)
						onComplete = function()
							up:set(false)
							refocus()
						end,
					})
				)
			)
		end
	end)
	local function leave()
		if open() then
			route.leave_settings()
		end
	end
	return ui.div {
		theme = { backdrop = "#00000000" },
		ui.modal {
			open = function(): boolean
				return up:get()
			end,
			on_close = leave,
			label = ui.t("settings.shell.heading"),
			-- (The panel takes the keys as it shows: Escape, ⌘F, the list's arrows.)
			autofocus = false,
			placement = { x = "start", y = "start", inset = 0 },
			-- (The modal is the whole window, the scrim's: a modal panel's size
			-- takes no percentage, and keeps 32 from the window's edges unless told.)
			w = function(): number
				return (window_size())
			end,
			h = function(): number
				local _, h = window_size()
				return h
			end,
			max_w = function(): number
				return (window_size())
			end,
			max_h = function(): number
				local _, h = window_size()
				return h
			end,
			p = 0,
			gap = 0,
			rounded = 0,
			bg = "#00000000",
			border = 0,
			shadow = "$zeron.dialog.stage",
			-- (Gone once it has left, not kept hidden as a modal's content is: no page's
			-- effects run on behind a closed panel.)
			build = function(): Node
				return ui.show(function(): boolean
					return up:get()
				end, ui.div {
					ref = M.inside,
					w_full = true,
					h_full = true,
					relative = true,
					items = "center",
					justify = "center",
					ui.div {
						id = "settings-scrim",
						absolute = true,
						left = 0,
						top = 0,
						right = 0,
						bottom = 0,
						bg = ui.raw("#00000073", "Claude's settings scrim"),
						aria_hidden = true,
						opacity = function(): number
							return math.clamp(dim:get(), 0, 1)
						end,
						on_click = leave,
					},
					ui.column(dialog.panel {
						id = "shell-settings",
						-- Zeron's Settings are no plugin's to change or add to: what you
						-- allow plugins and templates is decided here (Settings →
						-- Plugins and Templates). Sealed, with all it holds.
						hook = false,
						focusable = true,
						focus = { outline = 0 },
						on_mount = function(el)
							el:focus()
						end,
						w = function(): number
							return ui.raw(math.min(PANEL_W, window_size() - 2 * PANEL_INSET), "Claude's settings panel")
						end,
						h = function(): number
							local _, h = window_size()
							return ui.raw(math.min(PANEL_H, h - 2 * PANEL_INSET), "Claude's settings panel")
						end,
						overflow = "hidden",
						opacity = function(): number
							return math.clamp(phase:get(), 0, 1)
						end,
						scale = function(): number
							return 0.97 + 0.03 * phase:get()
						end,
						M.View({ panel = true }),
					}),
					-- What its pages say (a refused change): over the scrim while it's
					-- up, as the window's own stack is under it.
					ds.toast.Host { front = open },
				})
			end,
		},
	}
end)

return M
