-- The command palette (⌘K, or the sidebar's Search): the design system's
-- (ds.CommandPalette) over the main window, finding any session across
-- your projects (archived ones too, marked so), what you can do (a new
-- chat, in a project of your choosing), where you can go (settings' pages)
-- and what you can flip (the look, the sidebar, the template in use). With
-- nothing typed,
-- pinned sessions and what you used lately come first.
--
-- `Palette()`: placed once in the window.

local CommandPalette = require("@ds/command_palette")
local material = require("@ds/material")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local view = require("@app/screens/shell/view")
local fit = require("@app/screens/shell/fit")
local appearance = require("@app/screens/shell/appearance")
local onboarding_state = require("@app/screens/onboarding/state")
local settings = require("@app/screens/shell/settings")
local boundary = require("@app/screens/shell/boundary")
local paths = require("@app/screens/files/paths")
local files_api = require("@app/screens/files/api")
local templates = require("@app/screens/templates/state")
local themes = require("@app/screens/themes/state")
local keys = require("@app/screens/shell/keys")
local host = require("@app/screens/settings/host")
local Builtins = require("@app/screens/plugins/builtins")

-- What was used lately, by id, the latest first (kept across launches).
local recent = ui.persist("shell_palette_recent", {} :: { string })

type Item = any

-- The sidebar's view, as screens/shell/sidebar shares it (its Show options among it).
local sidebar_view = ui.shared("sidebar.view", {} :: { [string]: any })
-- (A session carried: the page's split shows where it can land, as for the sidebar's rows.)
local split_dragging = ui.shared("split.dragging", false)

-- A session's row lifted out, as the sidebar's (its payload): onto a composer as a mention, into a split.
local function session_drag(id: string): any
	return {
		payload = function(): any
			split_dragging:set(true)
			return { session = id }
		end,
		ended = function()
			split_dragging:set(false)
		end,
	}
end

-- A chat as the palette lists it: its glyph, state, project, branch and
-- where it runs, as the sidebar's regular row says it.
local function session_items(): { Item }
	local now = sessions.now:get()
	local projects: { [string]: sessions.Project } = {}
	for _, p in now.projects do
		projects[p.id] = p
	end
	local devices: { [string]: string } = {}
	for _, d in now.devices do
		devices[d.id] = d.name
	end
	local pinned: { [string]: boolean } = {}
	for _, id in now.places.pinned do
		pinned[id] = true
	end
	-- What the sidebar's Show has on (Zeron's palette rows honour it).
	local details = sidebar_view:get().details or {}
	local with_branch = details.branch ~= false
	local with_harness = details.harness ~= false
	local here = route.session:get()
	local out: { Item } = {}
	local function add(c: sessions.Chat, archived: boolean)
		local p = if c.project then projects[c.project] else nil
		table.insert(out, {
			id = `session:{c.id}`,
			label = c.title,
			section = locale.s("shell.palette.sessions"),
			session = {
				status = c.status,
				time = c.updated,
				agent = if with_harness then c.agent else nil,
				project = if p then p.name else nil,
				hue = if p then (route.look(p.id) or { hue = route.hue(p.id) }).hue else nil,
				src = if p then (route.look(p.id) or {}).src else nil,
				emoji = if p then (route.look(p.id) or {}).emoji else nil,
				branch = if with_branch then c.branch else nil,
				device = devices[c.device],
				archived = archived,
			},
			pinned = pinned[c.id],
			-- (The chat open aside until searched for, as Zeron's recent threads.)
			search_only = here ~= nil and here.id == c.id,
			drag = if archived then nil else session_drag(c.id),
			verb = locale.s("shell.palette.open"),
			run = function()
				route.open_session(c.id)
			end,
			alt = if archived
				then {
					label = locale.s("shell.palette.unarchive"),
					run = function()
						sessions.unarchive(c.id)
					end,
				}
				else nil,
		})
	end
	for _, c in now.chats do
		add(c, false)
	end
	for _, c in now.archived or {} do
		add(c, true)
	end
	-- Side chats have no sidebar row: a search is their way in (Zeron's).
	for parent, list in now.children or {} do
		for _, c in list do
			local id = c.id
			table.insert(out, {
				id = `session:{id}`,
				label = c.title,
				section = locale.s("shell.palette.sessions"),
				session = {
					status = c.status,
					time = c.updated,
					agent = if with_harness then c.agent else nil,
					device = devices[c.device],
				},
				search_only = true,
				verb = locale.s("shell.palette.open"),
				run = function()
					local lanes: any = boundary.load("@app/screens/chat/lanes")
					if lanes and lanes.open_thread then
						lanes.open_thread(id)
					else
						route.open_session(parent)
					end
				end,
			})
		end
	end
	return out
end

-- A new chat in one of your projects (or none).
local function projects_page(): any
	return {
		id = "projects",
		title = locale.s("shell.palette.new_chat_in"),
		placeholder = locale.s("shell.palette.search_projects"),
		items = function(): { Item }
			local out: { Item } = {}
			for _, p in sessions.now:get().projects do
				table.insert(out, {
					id = `project:{p.id}`,
					label = p.name,
					section = locale.s("shell.palette.projects"),
					hue = (route.look(p.id) or { hue = route.hue(p.id) }).hue,
					src = (route.look(p.id) or {}).src,
					emoji = (route.look(p.id) or {}).emoji,
					keywords = p.path,
					verb = locale.s("shell.palette.start"),
					run = function()
						route.new_chat(p.id)
					end,
				})
			end
			table.insert(out, {
				id = "project:none",
				label = locale.s("shell.palette.no_project"),
				section = locale.s("shell.palette.projects"),
				icon = "chat-round-line",
				verb = locale.s("shell.palette.start"),
				run = function()
					route.new_chat("none")
				end,
			})
			return out
		end,
		empty = { title = locale.s("shell.palette.no_projects") },
	}
end

-- The colour scheme, one of three.
local function theme_page(): any
	local items: { Item } = {}
	local schemes: { { value: onboarding_state.Appearance, label: string } } = {
		{ value = "system", label = locale.s("shell.palette.scheme.system") },
		{ value = "light", label = locale.s("shell.palette.scheme.light") },
		{ value = "dark", label = locale.s("shell.palette.scheme.dark") },
	}
	for _, o in schemes do
		local value, label = o.value, o.label
		table.insert(items, {
			id = `theme:{value}`,
			label = label,
			section = locale.s("shell.palette.theme"),
			checked = function(): boolean
				return appearance.scheme:get() == value
			end,
			run = function()
				appearance.scheme:set(value)
			end,
		})
	end
	return { id = "theme", title = locale.s("shell.palette.theme"), items = items }
end

-- The templates you can run Zeron in, Zeron's own first: picking one uses it
-- as the Templates page's Use does (the engine's UseTemplate), live. The
-- one in use is checked.
local function template_items(): { Item }
	local out: { Item } = {}
	for _, t in templates.list:get() do
		local id = t.id
		table.insert(out, {
			id = `template:{id}`,
			label = locale.s("shell.palette.use_template", { name = t.name }),
			section = locale.s("shell.palette.templates"),
			scope = "settings",
			icon = "gallery",
			keywords = `template interface {t.description or ""}`,
			verb = locale.s("shell.palette.use"),
			checked = function(): boolean
				for _, now in templates.list:get() do
					if now.id == id then
						return now.active
					end
				end
				return false
			end,
			run = function()
				templates.use(id)
			end,
		})
	end
	return out
end

-- Every theme Zeron can wear, Zeron's first: picking one wears it for each
-- scheme it has, as the Themes page's Use does. One worn for every scheme
-- it can be is checked.
local function theme_items(): { Item }
	local out: { Item } = {}
	for _, e in themes.entries:get() do
		local id = e.id
		local function now(): any
			for _, it in themes.entries:get() do
				if it.id == id then
					return it
				end
			end
			return nil
		end
		table.insert(out, {
			id = `use-theme:{id}`,
			label = locale.s("shell.palette.use_theme", { name = e.name }),
			section = locale.s("shell.palette.themes"),
			scope = "settings",
			icon = "palette-search",
			keywords = "theme colours colors palette",
			verb = locale.s("shell.palette.use"),
			checked = function(): boolean
				local it = now()
				return it ~= nil and themes.full(it)
			end,
			run = function()
				local it = now()
				if it then
					themes.use(it)
				end
			end,
		})
	end
	return out
end

-- Everything the app's menus hold (src/main.rs `serve_commands` publishes
-- them), each run as its menu item would be, its shortcut beside it. Where
-- the system draws no menu bar (Windows, Linux) this is how a pointer
-- reaches them. Those the palette lists as its own already are left out.
local menu_commands = ui.shared("app.menu_commands", {} :: { any })
local LISTED: { [string]: boolean } = {
	["shell.new_chat"] = true,
	["shell.new_project"] = true,
	["shell.archive"] = true,
	["shell.toggle_sidebar"] = true,
	["shell.settings"] = true,
	["shell.reload"] = true,
	["shell.reload_interface"] = true,
	-- (The palette itself.)
	["shell.palette"] = true,
}
-- The host's own way back from any template (its "Use Default Interface").
local DEFAULT_INTERFACE = "zeron.use-zeron-template"
local function command_items(): { Item }
	local out: { Item } = {}
	for _, c in menu_commands:get() do
		local id = c.id :: string
		if not LISTED[id] and Builtins.command_on(id) then
			local default = id == DEFAULT_INTERFACE
			table.insert(out, {
				id = `menu:{id}`,
				label = if default then locale.s("shell.palette.default_interface") else c.title,
				section = locale.s(if default then "shell.palette.templates" else "shell.palette.commands"),
				icon = if default then "gallery" else nil,
				shortcut = c.key,
				keywords = if default then "template interface safe revert" else c.menu,
				verb = locale.s("shell.palette.run"),
				run = function()
					host.invoke("app.run_menu_command", id)
				end,
			})
		end
	end
	return out
end

-- The files the session's panel has open, the one last in front first.
local function opened_files(session: any, project: any): { string }
	local state: any = boundary.load("@app/screens/panel/state")
	local files: any = boundary.load("@app/screens/files")
	local out: { string } = {}
	if not state or not files then
		return out
	end
	local key = state.key(session, project)
	if not key then
		return out
	end
	local s = state.of(key)
	local by: { [string]: any } = {}
	for _, t in s.tabs do
		by[t.id] = t
	end
	local order: { string } = table.clone(s.visited or {})
	for _, t in s.tabs do
		if not table.find(order, t.id) then
			table.insert(order, t.id)
		end
	end
	for _, id in order do
		local t = by[id]
		if t and t.kind == "files" then
			local path = (files.tab_file and files.tab_file(t.id)) or t.path
			if type(path) == "string" and not table.find(out, path) then
				table.insert(out, path)
			end
		end
	end
	return out
end

-- The open session's files, found by name, the workspace's own search
-- (Zeron's palette Files): a promise of rows, best first; a file opens in
-- the session's panel. Nothing typed: its recent files, those its panel
-- has open first, then what the workspace changed lately (the search's
-- answer to nothing), each once. No session open: none.
local function search_files(query: string): any
	local project, session = route.project:peek(), route.session:peek()
	local target = if session then paths.target_of(project, session) else nil
	if not target then
		return nil
	end
	local opener: any = boundary.load("@app/screens/panel")
	local recent = query == ""
	local function open_at(path: string, at: { line: number, column: number }?)
		if opener and opener.open_file and session then
			opener.open_file(session, path, project, at)
		end
	end
	return {
		-- (Told again as more comes: the names at once, then what's in the
		-- files, as Zeron's palette fills its In Files.)
		after = function(_, ok: (any) -> (), fail: ((any) -> ())?)
			local names: { Item } = {}
			local inside: { Item } = {}
			local function tell()
				local out = table.clone(names)
				for _, it in inside do
					table.insert(out, it)
				end
				ok(out)
			end
			files_api.search(target, query):after(function(found: any)
				local listed: { [string]: boolean } = {}
				local function add(path: string)
					if listed[path] or #names >= 50 then
						return
					end
					listed[path] = true
					table.insert(names, {
						id = `file:{path}`,
						label = paths.base_name(path),
						section = locale.s(if recent then "shell.palette.recent_files" else "shell.palette.files"),
						scope = "files",
						icon = "document",
						hint = string.match(path, "^(.*)/[^/]*$"),
						keywords = path,
						verb = locale.s("shell.palette.open"),
						run = function()
							open_at(path)
						end,
					})
				end
				if recent then
					for _, path in opened_files(session, project) do
						add(path)
					end
				end
				for _, f in (if type(found) == "table" then found else {}) do
					-- (Files: a folder isn't something it opens.)
					if not f.dir and type(f.path) == "string" then
						add(f.path :: string)
					end
				end
				tell()
			end, function(err: any)
				if fail then
					fail(err)
				end
			end)
			-- What's in the files (Zeron's In Files: up to three lines a file),
			-- looked again while the engine is still indexing them.
			if recent or not files_api.search_content then
				return
			end
			local waits = { 500, 1000, 2000 }
			local function look(tries: number)
				files_api.search_content(target, query, { limit = 100, per_file = 3 }):after(function(found: any)
					inside = {}
					for _, m in (if type(found) == "table" and type(found.matches) == "table" then found.matches else {}) do
						if type(m.path) == "string" and type(m.line) == "number" then
							local path, line, column = m.path :: string, m.line :: number, (m.column or 1) :: number
							table.insert(inside, {
								id = `line:{path}:{line}`,
								label = string.match(tostring(m.text or ""), "^%s*(.-)%s*$") or "",
								section = locale.s("shell.palette.in_files"),
								scope = "files",
								icon = "document",
								hint = `{paths.base_name(path)}:{line}`,
								keywords = path,
								verb = locale.s("shell.palette.open"),
								run = function()
									open_at(path, { line = line, column = column })
								end,
							})
						end
					end
					tell()
					if type(found) == "table" and found.indexing and waits[tries] then
						ui.after(waits[tries], function()
							look(tries + 1)
						end)
					end
				end, function() end)
			end
			look(1)
		end,
	}
end

-- A settings page's title in the locale shown: settings' own word for it
-- where it says one (`title_of`), else its title.
local function page_title(id: string, title: string): string
	local of = (settings :: any).title_of
	return if type(of) == "function" then of(id, title) else title
end

-- A command's keys now (a rebind's, else its default), for its row's cap.
local function key_of(id: string): string?
	local combo = keys.combo(id)
	return if combo ~= "" then combo else nil
end

-- Each setting's row (Settings' own search index), found by what's typed, opening Settings at it;
-- made again only as the index or the locale changes. (Not Shortcuts': its rows are the commands' own names.)
local settings_made: { key: string, items: { Item } }? = nil
local function setting_rows(): { Item }
	local search: any = boundary.load("@app/screens/settings/search")
	if not search or not search.rows then
		return {}
	end
	local section = locale.s("shell.palette.in_settings")
	local key = `{search.revision:get()}:{section}`
	if settings_made and settings_made.key == key then
		return settings_made.items
	end
	local out: { Item } = {}
	for _, hit in search.rows() do
		local h = hit
		if h.page ~= "shortcuts" then
			table.insert(out, {
				id = `setting:{h.page}:{h.key}`,
				label = h.title,
				section = section,
				scope = "settings",
				icon = "settings-minimalistic",
				hint = h.where,
				keywords = h.terms,
				search_only = true,
				verb = locale.s("shell.palette.open"),
				run = function()
					search.open(h)
				end,
			})
		end
	end
	settings_made = { key = key, items = out }
	return out
end

local function root(): { Item }
	local out: { Item } = {
		{
			id = "a-new",
			label = locale.s("shell.palette.new_chat"),
			section = locale.s("shell.palette.actions"),
			icon = "pen-new-square",
			shortcut = key_of("shell.new_chat"),
			keywords = "session start",
			run = function()
				route.new_chat()
			end,
		},
		{ id = "a-new-in", label = locale.s("shell.palette.new_chat_in"), section = locale.s("shell.palette.actions"), icon = "folder", keywords = "project session start", page = projects_page },
		{
			id = "a-project",
			label = locale.s("shell.palette.new_project"),
			section = locale.s("shell.palette.actions"),
			icon = "folder",
			shortcut = key_of("shell.new_project"),
			keywords = "add folder repository clone",
			-- Its steps a page of this palette, as "New chat in…": it slides
			-- there, and Back comes back.
			page = function(): any
				local add: any = boundary.load("@app/screens/shell/add_project")
				return if add then add.page() else nil
			end,
		},
		{
			id = "a-sidebar",
			label = locale.s("shell.palette.toggle_sidebar"),
			section = locale.s("shell.palette.actions"),
			icon = "list",
			shortcut = key_of("shell.toggle_sidebar"),
			run = function()
				-- (As ⌘B: hidden for room, it comes back.)
				fit.toggle_sidebar()
			end,
		},
	}
	-- The session open archived (as ⌘⇧A; not while its agent works).
	local open = route.current:get()
	if open.name == "session" and open.session then
		local id = open.session :: string
		for _, c in sessions.now:get().chats do
			if c.id == id and c.status ~= "working" then
				table.insert(out, {
					id = "a-archive",
					label = locale.s("shell.palette.archive_session"),
					section = locale.s("shell.palette.actions"),
					icon = "archive-minimalistic",
					shortcut = key_of("shell.archive"),
					keywords = "thread",
					run = function()
						local sidebar: any = boundary.load("@app/screens/shell/sidebar")
						if sidebar then
							sidebar.archive(id)
						end
					end,
				})
			end
		end
	end
	-- The app itself, for when something has left it in a bad state:
	-- relaunched whole (the engine keeps running, the chat open comes back),
	-- or its interface's scripts evaluated again in place. (Their keys, if
	-- Settings → Shortcuts gave them any.)
	table.insert(out, {
		id = "a-reload",
		label = locale.s("shell.palette.reload_zeron"),
		section = locale.s("shell.palette.app"),
		icon = "restart",
		shortcut = key_of("shell.reload"),
		keywords = "reload restart relaunch refresh quit app",
		run = function()
			host.invoke("app.relaunch")
		end,
	})
	table.insert(out, {
		id = "a-reload-interface",
		label = locale.s("shell.palette.reload_interface"),
		section = locale.s("shell.palette.app"),
		icon = "refresh",
		shortcut = key_of("shell.reload_interface"),
		keywords = "reload refresh scripts ui window",
		run = function()
			host.invoke("app.reload_interface")
		end,
	})
	-- Where settings go: each of its pages.
	for _, id in settings.pages:get() do
		local page = settings.page(id)
		if page then
			table.insert(out, {
				id = `g-settings-{id}`,
				label = if id == "general"
					then locale.s("shell.palette.settings")
					else locale.s("shell.palette.settings_page", { title = page_title(id, page.title) }),
				section = locale.s("shell.palette.go_to"),
				scope = "settings",
				icon = page.icon or "settings-minimalistic",
				shortcut = if id == "general" then key_of("shell.settings") else nil,
				keywords = "preferences",
				verb = locale.s("shell.palette.open"),
				run = function()
					route.settings(id)
				end,
			})
		end
	end
	table.insert(out, { id = "t-theme", label = locale.s("shell.palette.theme"), section = locale.s("shell.palette.settings"), scope = "settings", icon = "palette-search", keywords = "dark light appearance", page = theme_page })
	table.insert(out, { id = "t-frosted", label = locale.s("shell.palette.frosted_glass"), section = locale.s("shell.palette.settings"), icon = "moon", keywords = "material translucent opaque window appearance", toggle = material.frosted })
	for _, t in template_items() do
		table.insert(out, t)
	end
	for _, t in theme_items() do
		table.insert(out, t)
	end
	for _, c in command_items() do
		table.insert(out, c)
	end
	for _, r in setting_rows() do
		table.insert(out, r)
	end
	for _, s in session_items() do
		table.insert(out, s)
	end
	return out
end

-- Open, Settings' pages load (a few a turn) for their rows under In Settings.
ui.effect(function()
	if view.palette:get() then
		settings.load_all()
	end
end)

return function(): Node
	return ui.div {
		id = "shell-palette",
		hook = "shell.palette",
		CommandPalette {
			open = view.palette,
			recent = recent,
			-- (Its placeholder read as it's drawn: the locale shown then.)
			root = setmetatable({ id = "root", title = "Zeron", items = root, search = search_files, scopes = true, escape_closes = true }, {
				__index = function(_, field: string): any
					if field == "shortcut" then
						-- (⌘K's cap at the field's end, as rebound.)
						return key_of("shell.palette")
					end
					return if field == "placeholder" then locale.s("shell.palette.search") else nil
				end,
			}) :: any,
		},
	}
end
