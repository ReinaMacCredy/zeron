-- New project, as the shipping app adds one (crates/ui shell/spaces.rs
-- `AddSpaceFlow`): a palette, its steps a page each. Your devices (one alone
-- is skipped); on one, Home and its drives; then its folders beside a rail
-- (Pinned, Recent, Locations), a click choosing a row and a double click
-- going in. Every device is browsed through the engine, this Mac's too (no
-- folder dialog). Under Home, what's typed also finds folders anywhere under
-- it ("In Home", the engine's index). A path typed (`~/code`, `/mnt`) goes
-- there; ⌘L, the pen or the trail's empty end edit it in the trail. A git
-- address typed in a folder offers to clone it there. A name nothing matches
-- offers a new folder, as ⌘⇧N and the folder button do. A folder dropped from
-- Finder is added. Added (`createSpace`, named after its folder), the window
-- lands in a new chat in it; a folder that's a project already just opens it.
--
-- Cloud, while it's on, is the devices' last row (one for all of Cloud: its
-- sessions' machines are never devices here): its page lists the GitHub
-- repositories Cloud can open, by owner, newest push first, each saying if
-- it's private or a project on a device already; one picked is a Cloud
-- project (`createSpace` on Cloud's device, the repository named, as the
-- shipping app's `add_space_create_cloud`), or the one Cloud has already,
-- and the window lands in a new chat in it. Without GitHub, or without a
-- repository, the page says so and leads to Settings → Cloud. A folder's
-- search also offers those repositories (From GitHub), to clone there.
--
--   AddProject()            placed once in the window
--   add_project.open()      ⌘⇧N, the palette's and the project filter's
--                           "New project", the composer's
--   add_project.shown       whether it's open (a state)
--   add_project.new_project_key()  ⌘⇧N with it showing: names a new folder, else closes; whether it did

local CommandPalette = require("@ds/command_palette")
local Icon = require("@ds/icon")
local IconButton = require("@ds/icon_button").IconButton
local Kbd = require("@ds/kbd")
local FadeText = require("@ds/fade_text")
local motion = require("@ds/motion")
local boundary = require("@app/screens/shell/boundary")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local projects = require("@account/projects")
local cloud = require("@account/cloud")
local route = require("@app/screens/shell/route")
local refocus_chat = require("@app/screens/shell/refocus")
local view = require("@app/screens/shell/view")

local M = {}

M.shown = view.add_project
function M.open()
	view.palette:set(false)
	M.shown:set(true)
end
M.hosted = ui.state("shell_add_project_hosted", false)
ui.effect(function()
	if not view.palette:get() and M.hosted:peek() then
		M.hosted:set(false)
	end
end)
local function showing(): boolean
	return M.shown:get() or M.hosted:get()
end
local function showing_now(): boolean
	return M.shown:peek() or M.hosted:peek()
end
local function close_all()
	M.shown:set(false)
	if M.hosted:peek() then
		view.palette:set(false)
	end
end

type Item = any
type Page = any
type Location = { name: string, path: string? }

local RAIL_W, WIDE, VIEW_H = 160, 720, 360
local METRICS = { row = 30, gap = 1, head = 28, head_first = 22, inset = 8, px = 8 }
local FOLDER_H, RECENT_FOLDERS = 30, 4
local CRUMB_MAX, CRUMB_TAIL = 3, 2
local IN_MS, OUT_MS, FADE_MS = 180, 180, 150

local pins = ui.persist("shell_add_project_pins", {} :: { [string]: { string } })
local at = ui.state("shell_add_project_at", {} :: { [string]: any })
local hidden = ui.state("shell_add_project_hidden", false)
local busy = ui.state("shell_add_project_busy", nil :: string?)
local cloning = ui.state("shell_add_project_cloning", nil :: string?)
local failure = ui.state("shell_add_project_failure", nil :: string?)
local drop_over = ui.state("shell_add_project_drop", false)
local drives_of = ui.state("shell_add_project_drives", {} :: { [string]: { any } })
local home_state = ui.state("shell_add_project_home_state", {} :: { q: string?, status: string? })

local naming = ui.state("shell_add_project_naming", false)
local name_text = ui.state("shell_add_project_name", "")
local name_git = ui.state("shell_add_project_name_git", false)
local name_error = ui.state("shell_add_project_name_error", nil :: string?)
local name_busy = ui.state("shell_add_project_name_busy", false)
local name_shown = ui.state("shell_add_project_name_shown", 0)
local name_leaving = ui.state("shell_add_project_name_leaving", false)

local editing = ui.state("shell_add_project_editing", false)
local path_text = ui.state("shell_add_project_path", "")
local path_error = ui.state("shell_add_project_path_error", nil :: string?)
local path_opening = ui.state("shell_add_project_path_opening", false)
local path_folders = ui.state("shell_add_project_path_folders", {} :: { key: string?, names: { string }? })

local nav: any = nil
local open_folder_at_location: (device: string, location: Location) -> ()
local root_page: Page = nil
local places_page: Page = nil
local name_field: any = nil
local path_field: any = nil
local name_after_load = false
local path_adding = false

-- An answer whose asker has gone is dropped; the device may still have done it, as gpui's dropped tasks.
local tickets_made = 0
local tickets: { [string]: number } = {}
local function on_page(page: Page): boolean
	return nav ~= nil and nav.page() == page
end
local function ticket(what: string, page: Page?): () -> boolean
	tickets_made += 1
	local mine = tickets_made
	tickets[what] = mine
	return function(): boolean
		return tickets[what] == mine and (page == nil or on_page(page))
	end
end
local function leave_path()
	tickets.path = nil
	path_adding = false
	ui.batch(function()
		editing:set(false)
		path_opening:set(false)
		path_folders:set({})
	end)
end

local function says(err: any, path: string?): string
	local text = err and if type(err) == "table" then err.message else tostring(err)
	text = if text then tostring(text) else locale.s("shell.add_project.something_went_wrong")
	if string.find(text, "not connected", 1, true) or text == "offline" then
		return locale.s("shell.add_project.not_connected")
	elseif path and (string.find(text, "os error 2)", 1, true) or string.find(text, "os error 3)", 1, true)) then
		return locale.s("shell.add_project.no_folder_at", { path = path })
	elseif path and (string.find(text, "os error 20)", 1, true) or string.find(text, "os error 267)", 1, true)) then
		return locale.s("shell.add_project.not_a_folder", { path = path })
	end
	return text
end

local arriving = ui.state("shell_add_project_arriving", nil :: string?)
ui.effect(function()
	local id = arriving:get()
	if not id then
		return
	end
	for _, p in sessions.now:get().projects do
		if p.id == id then
			arriving:set(nil)
			route.new_chat(id)
			refocus_chat()
			return
		end
	end
end)

function M.add(device: string, path: string, git: boolean, repo: string?)
	if busy:peek() then
		return
	end
	for _, p in sessions.now:peek().projects do
		local same_repo = repo ~= nil and p.github ~= nil and string.lower(p.github :: string) == string.lower(repo :: string)
		if p.device == device and (p.path == path or same_repo) then
			close_all()
			route.new_chat(p.id)
			refocus_chat()
			return
		end
	end
	busy:set("adding")
	failure:set(nil)
	local alive = ticket("add")
	projects.create(device, path, git, repo):after(function(id: any)
		if not alive() then
			return
		end
		busy:set(nil)
		cloning:set(nil)
		close_all()
		if type(id) == "string" then
			arriving:set(id)
			local options: any = boundary.load("@app/screens/shell/sidebar")
			if options and options.options.project:peek() ~= nil then
				options.options.project:set(id)
			end
		end
	end, function(err: any)
		if not alive() then
			return
		end
		busy:set(nil)
		cloning:set(nil)
		failure:set(says(err, path))
	end)
end

local function is_windows(path: string): boolean
	return string.match(path, "^%a:[\\/]") ~= nil or string.sub(path, 1, 2) == "\\\\"
end
local function join(parent: string, name: string): string
	local sep = if string.find(parent, "\\", 1, true) and not string.find(parent, "/", 1, true) then "\\" else "/"
	if string.sub(parent, -1) == sep then
		return parent .. name
	end
	return parent .. sep .. name
end
local function key_of(path: string): string
	local trimmed = (string.gsub(path, "[/\\]+$", ""))
	return if trimmed == "" then string.sub(path, 1, 1) else trimmed
end
local function same(a: string, b: string): boolean
	return key_of(a) == key_of(b)
end
local function parent_of(path: string): string?
	local t = (string.gsub(path, "[/\\]+$", ""))
	if t == "" or string.match(t, "^%a:$") then
		return nil
	end
	local folder = string.match(t, "^(.*)[/\\][^/\\]*$")
	if not folder then
		return nil
	end
	if folder == "" then
		return "/"
	end
	return if string.match(folder, "^%a:$") then folder .. "\\" else folder
end
local function under(path: string, base: string): boolean
	local b = key_of(base)
	if b == "/" or string.match(b, "^%a:[\\/]?$") then
		return is_windows(path) == is_windows(base)
	end
	local p = key_of(path)
	if p == b then
		return true
	end
	local next_char = string.sub(p, #b + 1, #b + 1)
	return string.sub(p, 1, #b) == b and (next_char == "/" or next_char == "\\")
end
local function leaf_name(path: string): string
	return string.match(key_of(path), "([^/\\]+)$") or path
end
local function crumbs_of(path: string): { { name: string, full: string } }
	local out: { { name: string, full: string } } = {}
	local windows = is_windows(path)
	local root = if string.sub(path, 1, 2) == "\\\\" then string.match(path, "^\\\\[^/\\]+[/\\][^/\\]+[/\\]?") or path elseif windows then string.sub(path, 1, 3) else ""
	local acc = root
	for segment in string.gmatch(string.sub(path, #root + 1), "[^/\\]+") do
		acc = if acc == "" then "/" .. segment else join(acc, segment)
		table.insert(out, { name = segment, full = acc })
	end
	return out
end

local function git_url(text: string): string?
	local t = string.match(text, "^%s*(.-)%s*$") or ""
	local tail: string? = nil
	for _, scheme in { "https://", "http://", "ssh://", "git://" } do
		if string.sub(t, 1, #scheme) == scheme then
			tail = string.match(string.sub(t, #scheme + 1), "^[^/]+/(.+)$")
		end
	end
	if not tail then
		tail = string.match(t, "^[%w%.%-_]+@[%w%.%-]+:(.+)$")
	end
	if not tail then
		return nil
	end
	tail = string.gsub(tail, "%.git$", "")
	tail = string.gsub(tail, "/$", "")
	local owner, name = string.match(tail, "^([^/]+)/([^/]+)$")
	return if owner and name then `{owner}/{name}` else nil
end

local folder_page: (device: string, path: string?, title: string, git: boolean, location: Location?, preload: any?) -> Page

local HOME_LEAST, HOME_RETRY, HOME_RETRY_MOST = 2, 500, 2000
local warmed: { [string]: boolean } = {}
local function warm(device: string)
	if not warmed[device] then
		warmed[device] = true
		projects.search_home(device, ""):after(function() end, function() end)
	end
end
local function release_homes()
	for device in warmed do
		projects.release_home(device)
	end
	table.clear(warmed)
end
ui.effect(function()
	if showing() then
		return
	end
	release_homes()
	table.clear(tickets)
	path_adding = false
	ui.batch(function()
		at:set({})
		hidden:set(false)
		busy:set(nil)
		cloning:set(nil)
		failure:set(nil)
		drop_over:set(false)
		naming:set(false)
		name_leaving:set(false)
		name_shown:set(0)
		name_busy:set(false)
		editing:set(false)
		path_opening:set(false)
		path_folders:set({})
		home_state:set({})
	end)
	nav, root_page, places_page, name_field, path_field = nil, nil, nil, nil, nil
	name_after_load = false
end)

local function typed_path(text: string): boolean
	return string.sub(text, 1, 1) == "/"
		or text == "~"
		or string.match(text, "^~[/\\]") ~= nil
		or string.match(text, "^%a:[\\/]") ~= nil
		or string.sub(text, 1, 2) == "\\\\"
end

local homes: { [string]: string } = {}
local function with_home(device: string, done: (string?, any?) -> ())
	if homes[device] then
		done(homes[device])
		return
	end
	projects.folders(device, nil):after(function(listing: any)
		if type(listing) == "table" and type(listing.path) == "string" then
			homes[device] = listing.path
			done(listing.path)
		else
			done(nil, "Device is not connected")
		end
	end, function(err: any)
		done(nil, err)
	end)
end
local function resolve(device: string, text: string, done: (string?, any?) -> ())
	local t = string.match(text, "^%s*(.-)%s*$") or ""
	if not (t == "/" or string.match(t, "^%a:[\\/]$")) then
		t = (string.gsub(t, "[/\\]+$", ""))
	end
	if t ~= "~" and not string.match(t, "^~[/\\]") then
		done(if t == "" then nil else t)
		return
	end
	with_home(device, function(home: string?, why: any?)
		if not home then
			done(nil, why)
			return
		end
		local rest = string.sub(t, 3)
		done(if rest == "" then home else join(home, rest))
	end)
end
local function tilde(device: string, path: string): string
	local home = homes[device]
	if home and path == home then
		return "~"
	elseif home and string.sub(path, 1, #home + 1) == home .. "/" then
		return "~" .. string.sub(path, #home + 1)
	end
	return path
end
local function has_git(listing: any): boolean
	for _, e in (if type(listing) == "table" and type(listing.entries) == "table" then listing.entries else {}) do
		if e.name == ".git" then
			return true
		end
	end
	return false
end
local function home_place(relative: string): (string, string)
	local parent, name = string.match(relative, "^(.*)/([^/]+)$")
	if parent then
		return name, `~/{parent}`
	end
	return relative, "~"
end
local function location_of(device: string, path: string): Location
	local home = homes[device]
	if home and under(path, home) then
		return { name = locale.s("shell.add_project.home"), path = nil }
	end
	local best: any = nil
	for _, d in drives_of:peek()[device] or {} do
		if type(d.path) == "string" and under(path, d.path) and (not best or #key_of(d.path) > #key_of(best.path)) then
			best = d
		end
	end
	if best then
		return { name = best.name, path = best.path }
	end
	local root = if is_windows(path) then string.sub(path, 1, 3) else "/"
	return { name = root, path = root }
end
local function pinned_in(device: string): { string }
	return pins:get()[device] or {}
end
local function is_pinned(device: string, path: string): boolean
	for _, pin in pinned_in(device) do
		if same(pin, path) then
			return true
		end
	end
	return false
end
local function toggle_pin(device: string, path: string)
	local all = table.clone(pins:peek())
	local list = table.clone(all[device] or {})
	local found = nil
	for i, pin in list do
		if same(pin, path) then
			found = i
		end
	end
	if found then
		table.remove(list, found)
	else
		table.insert(list, path)
	end
	all[device] = if #list > 0 then list else nil
	pins:set(all)
end
local function is_added(device: string, path: string): boolean
	for _, p in sessions.now:get().projects do
		if p.device == device and same(p.path, path) then
			return true
		end
	end
	return false
end
local function recent_folders(device: string): { string }
	local list = {}
	for _, p in sessions.now:get().projects do
		if p.device == device then
			table.insert(list, p)
		end
	end
	table.sort(list, function(a: any, b: any): boolean
		return (a.created or 0) > (b.created or 0)
	end)
	local out: { string } = {}
	local seen: { [string]: boolean } = {}
	local home = homes[device]
	for _, p in list do
		local parent = parent_of(p.path)
		if parent and not (home and same(parent, home)) and not seen[key_of(parent)] and #out < RECENT_FOLDERS then
			seen[key_of(parent)] = true
			if not is_pinned(device, parent) then
				table.insert(out, parent)
			end
		end
	end
	return out
end

local function offer(row: any): any
	return {
		after = function(_, ok: (any) -> ())
			ok({ row })
		end,
	}
end
local function offer_all(rows: { any }): any
	return {
		after = function(_, ok: (any) -> ())
			ok(rows)
		end,
	}
end
local CLONE_MATCHES = 5

local function set_at(fields: { [string]: any })
	local now = table.clone(at:peek())
	for k, v in fields do
		now[k] = v
	end
	now.empty = nil
	at:set(now)
end
local function refocus_search()
	if nav then
		nav.focus()
	end
end

local function dismiss_name(fast: boolean?)
	tickets.name = nil
	if not naming:peek() then
		return
	end
	name_busy:set(false)
	if fast or motion.reduced_now() then
		ui.batch(function()
			naming:set(false)
			name_leaving:set(false)
			name_shown:set(0)
		end)
		return
	end
	name_leaving:set(true)
	name_error:set(nil)
	anime.animate(name_shown, { value = 0, duration = OUT_MS, ease = "outCubic" })
	ui.after(OUT_MS, function()
		if name_leaving:peek() then
			ui.batch(function()
				naming:set(false)
				name_leaving:set(false)
				name_shown:set(0)
			end)
		end
	end)
end
local function naming_now(): boolean
	return naming:peek() and not name_leaving:peek()
end
local function start_name(name: string)
	local a = at:peek()
	if a.step ~= "folders" or not a.open or busy:peek() then
		return
	end
	leave_path()
	if naming_now() then
		if name_field then
			name_field:focus()
		end
		return
	end
	local still_there = naming:peek()
	ui.batch(function()
		name_leaving:set(false)
		name_text:set(name)
		name_git:set(false)
		name_error:set(nil)
		name_busy:set(false)
		naming:set(true)
		if nav then
			nav.set_query("")
		end
	end)
	if still_there and name_field then
		name_field:focus()
	end
	if motion.reduced_now() then
		name_shown:set(1)
	else
		if not still_there then
			name_shown:set(0)
		end
		anime.animate(name_shown, { value = 1, duration = IN_MS, ease = motion.EASE })
	end
end
function M.new_project_key(): boolean
	if not M.shown:peek() and not (M.hosted:peek() and nav ~= nil and nav.page().plain) then
		return false
	end
	local a = at:peek()
	if a.step == "folders" and a.open and not busy:peek() then
		start_name("")
	else
		close_all()
	end
	return true
end

local function open_folder(device: string, path: string, git: boolean, location: Location?)
	if nav then
		nav.replace(folder_page(device, path, leaf_name(path), git, location))
	end
end
local function create_folder(add: boolean)
	local a = at:peek()
	if not naming_now() or name_busy:peek() or not a.open then
		return
	end
	local name = string.match(name_text:peek(), "^%s*(.-)%s*$") or ""
	if name == "" then
		name_error:set(locale.s("shell.add_project.type_folder_name"))
		if name_field then
			name_field:focus()
		end
		return
	end
	local device, git = a.device, name_git:peek()
	name_busy:set(true)
	name_error:set(nil)
	local alive = ticket("name", nav and nav.page())
	projects.create_folder(device, a.open, name, git):after(function(made: any)
		if not alive() then
			return
		end
		dismiss_name(true)
		name_busy:set(false)
		local path = if type(made) == "string" then made else join(a.open, name)
		if add then
			M.add(device, path, git)
		else
			open_folder(device, path, git, a.location)
			refocus_search()
		end
	end, function(err: any)
		if not alive() then
			return
		end
		name_busy:set(false)
		local text = says(err)
		name_error:set(if string.find(text, "unknown method", 1, true) then locale.s("shell.add_project.update_to_create") else text)
		if name_field then
			name_field:focus()
		end
	end)
end
local function toggle_hidden()
	local a = at:peek()
	if a.step ~= "folders" then
		return
	end
	dismiss_name(true)
	leave_path()
	hidden:set(not hidden:peek())
	if nav then
		nav.replace(folder_page(a.device, a.open or a.path or nil, a.title, a.repo == true, a.location or nil))
		refocus_search()
	end
end

local function windows_now(): boolean
	local a = at:peek()
	local open = a.open or homes[a.device or ""]
	if open then
		return is_windows(open)
	end
	return a.platform == "windows"
end
local function split_leaf(text: string): (string, string)
	local windows = windows_now() or is_windows(text)
	local folder, name = string.match(text, if windows then "^(.*[/\\])([^/\\]*)$" else "^(.*/)([^/]*)$")
	if folder then
		return folder, name
	end
	return "", text
end
local function unescape(text: string): string
	if windows_now() or is_windows(text) then
		return text
	end
	return (string.gsub(text, "\\(.)", "%1"))
end
local function escape(text: string): string
	if windows_now() or is_windows(text) then
		return text
	end
	return (string.gsub(text, "\\", "\\\\"))
end
local function normalized(path: string): string
	local windows = is_windows(path)
	local root = if windows then string.sub(path, 1, 3) else "/"
	local kept: { string } = {}
	for segment in string.gmatch(if windows then string.sub(path, 4) else path, "[^/\\]+") do
		if segment == ".." then
			table.remove(kept)
		elseif segment ~= "." then
			table.insert(kept, segment)
		end
	end
	return root .. table.concat(kept, if windows then "\\" else "/")
end
local function clean(text: string): string
	local t = string.match(text, "^%s*(.-)%s*$") or ""
	local inner = string.match(t, '^"(.*)"$') or string.match(t, "^'(.*)'$")
	return unescape(inner or t)
end
local function resolve_typed(text: string, base: string?, home: string?): string?
	local t = clean(text)
	if t == "" then
		return nil
	end
	local target: string
	if t == "~" or string.match(t, "^~[/\\]") then
		if not home then
			return nil
		end
		local rest = string.sub(t, 3)
		target = if rest == "" then home else join(home, rest)
	elseif string.sub(t, 1, 1) == "/" or is_windows(t) then
		target = t
	else
		if not base then
			return nil
		end
		target = join(base, t)
	end
	return normalized(target)
end
local function refresh_completion()
	local a = at:peek()
	if not editing:peek() or not a.device then
		return
	end
	local folder = split_leaf(path_text:peek())
	local base = a.open or homes[a.device]
	local parent = if folder == "" then base else resolve_typed(folder, base, homes[a.device])
	if not parent then
		path_folders:set({})
		return
	end
	local key = key_of(parent)
	if path_folders:peek().key == key then
		return
	end
	path_folders:set({ key = key })
	projects.folders(a.device, parent):after(function(listing: any)
		if path_folders:peek().key ~= key then
			return
		end
		local names: { string } = {}
		for _, e in (if type(listing) == "table" and type(listing.entries) == "table" then listing.entries else {}) do
			if e.is_dir and type(e.name) == "string" then
				table.insert(names, e.name)
			end
		end
		path_folders:set({ key = key, names = names })
	end, function() end)
end
local function path_completion(): (string?, string?)
	local names = path_folders:get().names
	if not names then
		return nil, nil
	end
	local folder, typed = split_leaf(path_text:get())
	local name = unescape(typed)
	if name == "" or name == "." or name == ".." then
		return nil, nil
	end
	local found: string? = nil
	for _, n in names do
		if n == name then
			found = n
			break
		end
	end
	if not found then
		for _, n in names do
			if string.lower(string.sub(n, 1, #name)) == string.lower(name) then
				found = n
				break
			end
		end
	end
	if not found then
		return nil, nil
	end
	local full = found :: string
	local last = string.sub(folder, -1)
	local sep = if last == "/" or last == "\\" then last elseif windows_now() then "\\" else "/"
	local ghost = if #name < #full then string.sub(full, #name + 1) else nil
	return folder .. escape(full) .. sep, ghost
end
local function cancel_edit(): boolean
	if not editing:peek() then
		return false
	end
	leave_path()
	refocus_search()
	return true
end
local function edit_path()
	local a = at:peek()
	if a.step == "devices" then
		local item = if nav then nav.current() else nil
		if not item or not item.device_row then
			return
		end
		nav.activate()
		a = at:peek()
	end
	if a.step ~= "locations" and a.step ~= "folders" then
		return
	end
	dismiss_name()
	if editing:peek() then
		if path_field then
			path_field:focus()
			path_field:select_all()
		end
		return
	end
	local open = a.open or homes[a.device]
	local text = if open then escape(tilde(a.device, open)) else "~"
	local last = string.sub(text, -1)
	if last ~= "/" and last ~= "\\" then
		text ..= if windows_now() then "\\" else "/"
	end
	ui.batch(function()
		path_text:set(text)
		path_error:set(nil)
		path_opening:set(false)
		path_folders:set({})
		editing:set(true)
	end)
	tickets.path = nil
	if not homes[a.device] then
		with_home(a.device, function()
			refresh_completion()
		end)
	else
		refresh_completion()
	end
end
local function open_typed(add: boolean)
	local a = at:peek()
	if not editing:peek() or busy:peek() or not a.device then
		return
	end
	if path_opening:peek() then
		path_adding = path_adding or add
		return
	end
	local text = path_text:peek()
	if (string.match(text, "^%s*(.-)%s*$") or "") == "" then
		path_error:set(locale.s("shell.add_project.type_path"))
		return
	end
	local alive = ticket("path", nav and nav.page())
	path_adding = add
	path_opening:set(true)
	path_error:set(nil)
	local device = a.device
	with_home(device, function(home: string?, why: any?)
		if not alive() then
			return
		end
		local target = resolve_typed(text, a.open or home, home)
		if not target then
			path_opening:set(false)
			path_error:set(if why then says(why) else locale.s("shell.add_project.cant_open", { path = text }))
			return
		end
		local found = target :: string
		projects.folders(device, found, hidden:peek()):after(function(listing: any)
			if not alive() then
				return
			end
			path_opening:set(false)
			local adding = path_adding
			path_adding = false
			local there = if type(listing) == "table" and type(listing.path) == "string" then listing.path else found
			editing:set(false)
			if nav then
				nav.replace(folder_page(device, there, leaf_name(there), false, location_of(device, there), listing))
				refocus_search()
			end
			if adding then
				M.add(device, there, has_git(listing))
			end
		end, function(err: any)
			if not alive() then
				return
			end
			path_opening:set(false)
			path_adding = false
			path_error:set(says(err, tilde(device, found)))
			if path_field then
				path_field:focus()
			end
		end)
	end)
end

local function submit()
	local a = at:peek()
	if busy:peek() or a.step ~= "folders" then
		return
	end
	local item = if nav then nav.current() else nil
	if item and item.alt then
		item.alt.run()
	elseif a.open and not item then
		M.add(a.device, a.open, a.repo == true)
	end
end

local function picker_keys(e: any): boolean
	local k = e.key
	local cmd = e.meta == true or e.cmd == true
	if cmd and (k == "l" or k == "L") and not e.shift then
		edit_path()
		return true
	end
	if cmd and e.shift and (k == "." or k == ">") then
		toggle_hidden()
		return true
	end
	if k == "ArrowLeft" then
		if nav then
			nav.up()
		end
		return true
	elseif k == "ArrowRight" then
		if nav then
			nav.activate()
		end
		return true
	elseif k == "Backspace" and nav and nav.query() == "" then
		nav.up()
		return true
	elseif k == "Escape" then
		close_all()
		return true
	elseif k == "Enter" and cmd then
		if not e["repeat"] then
			submit()
		end
		return true
	elseif k == "Tab" then
		local page = if nav then nav.page() else nil
		local complete = if page then page.complete else nil
		local q = if nav then nav.query() else ""
		local to = if complete and q ~= "" and not e.shift then complete(q) else nil
		if to and to ~= q then
			nav.set_query(to)
		end
		return true
	end
	return false
end

local function legend(keys: Node, what: string): Node
	return ui.row {
		gap = "$zeron.space.tight",
		items = "center",
		shrink = 0,
		keys,
		ui.text { what, literal = true, text_size = "$font.size.xs", color = "$text.tertiary" },
	}
end
local function foot_button(props: { id: string, label: () -> string, enabled: () -> boolean, run: () -> (), strong: boolean?, keys: string? }): Node
	return ui.button {
		id = props.id,
		label = props.label,
		h = 24,
		px = "$space.2",
		gap = "$space.1",
		items = "center",
		max_w = 280,
		min_w = if props.strong then 0 else nil,
		shrink = if props.strong then 1 else 0,
		rounded = "$radius.md",
		bg = if props.strong then "$zeron.primary.bg" else "#00000000",
		color = if props.strong then "$zeron.primary.text" else "$text.primary",
		border = if props.strong then 0 else 1,
		border_color = "$border",
		hover = { bg = if props.strong then "$zeron.primary.hover" else "$hover" },
		active = { bg = if props.strong then "$zeron.primary.active" else "$pressed" },
		disabled = function(): boolean
			return not props.enabled()
		end,
		opacity = function(): number
			return if props.enabled() then 1 else 0.4
		end,
		text_size = "$font.size.xs",
		weight = if props.strong then "$font.weight.medium" else nil,
		on_click = function()
			if props.enabled() then
				props.run()
			end
		end,
		ui.text { props.label, literal = true, truncate = true, min_w = 0, shrink = 1, text_size = "$font.size.xs", color = if props.strong then "$zeron.primary.text" else "$text.primary" },
		if props.keys then ui.div { shrink = 0, opacity = 0.7, ui.text { (string.gsub(ui.shortcut_text(props.keys), "↩", "Enter")), text_size = "$font.size.xs" } } else nil,
	}
end
local function add_label(): string
	local a = at:get()
	local c = cloning:get()
	if c then
		return locale.s("shell.add_project.cloning", { repo = c })
	end
	if busy:get() then
		return locale.s("shell.add_project.adding")
	end
	if editing:get() then
		return locale.s("shell.add_project.add_project")
	end
	if naming:get() and not name_leaving:get() then
		if name_busy:get() then
			return locale.s("shell.add_project.creating")
		end
		local n = string.match(name_text:get(), "^%s*(.-)%s*$") or ""
		return if n == "" then locale.s("shell.add_project.create_and_add") else locale.s("shell.add_project.create_and_add_named", { name = n })
	end
	local item = if nav then nav.current() else nil
	if item and item.clone_of then
		return locale.s("shell.add_project.clone_named", { repo = item.clone_of })
	elseif item and item.create_of then
		return locale.s("shell.add_project.create_named", { name = item.create_of })
	end
	local path: string? = if item and item.target then item.target else (if item then nil else a.open)
	if not path then
		return locale.s("shell.add_project.add_project")
	end
	local found = path :: string
	local home = homes[a.device or ""]
	local name = if home and same(found, home) then locale.s("shell.add_project.home") else leaf_name(found)
	return locale.s(if is_added(a.device, found) then "shell.add_project.open_named" else "shell.add_project.add_named", { name = name })
end
local function can_add(): boolean
	if busy:get() then
		return false
	end
	local a = at:get()
	if editing:get() then
		return not path_opening:get() and (string.match(path_text:get(), "^%s*(.-)%s*$") or "") ~= ""
	end
	if naming:get() and not name_leaving:get() then
		return not name_busy:get() and (string.match(name_text:get(), "^%s*(.-)%s*$") or "") ~= ""
	end
	local item = if nav then nav.current() else nil
	return not not a.open and (if item then item.alt ~= nil else true)
end
local function step_foot(): Node
	return ui.column {
		w_full = true,
		ui.presence(function(): string?
			return failure:get()
		end, function(text: string)
			return ui.text { text, literal = true, w_full = true, px = "$space.4", py = "$space.2", text_size = "$font.size.xs", color = "$danger" }
		end, { w_full = true }),
		ui.row {
			min_h = 40,
			w_full = true,
			shrink = 0,
			px = "$space.3",
			py = ui.raw(7, "gpui's palette_footer: one row stays 40 tall"),
			gap = "$space.3",
			flex_wrap = true,
			items = "center",
			ui.presence(function(): string
				local a = at:get()
				if editing:get() then
					return "editing"
				elseif naming:get() and not name_leaving:get() then
					return "naming"
				end
				return a.step or "devices"
			end, function(mode: string)
				local parts: { [any]: any } = { gap = "$space.3", items = "center" }
				local nav_keys = ui.row { gap = "$zeron.space.hair", Kbd { keys = "up" }, Kbd { keys = "down" } }
				if mode == "editing" then
					table.insert(parts, legend(Kbd { text = "⇥" }, locale.s("shell.add_project.complete")))
					table.insert(parts, legend(Kbd { keys = "enter" }, locale.s("shell.add_project.open")))
					table.insert(parts, legend(Kbd { text = "esc" }, locale.s("common.cancel")))
				elseif mode == "naming" then
					table.insert(parts, legend(Kbd { keys = "enter" }, locale.s("shell.add_project.create_and_open")))
				elseif mode == "folders" then
					table.insert(parts, legend(nav_keys, locale.s("ds.command_palette.navigate")))
					table.insert(parts, legend(Kbd { keys = "right" }, locale.s("shell.add_project.open")))
					table.insert(parts, legend(Kbd { keys = "left" }, locale.s("ds.command_palette.back")))
					table.insert(parts, legend(Kbd { keys = "cmd+shift+n" }, locale.s("shell.add_project.new_folder")))
				else
					table.insert(parts, legend(nav_keys, locale.s("ds.command_palette.navigate")))
					table.insert(parts, legend(Kbd { keys = "enter" }, if mode == "repos" then locale.s("shell.add_project.add") else locale.s("shell.add_project.select")))
					if mode ~= "devices" then
						table.insert(parts, legend(Kbd { keys = "left" }, locale.s("ds.command_palette.back")))
					end
					table.insert(parts, legend(Kbd { text = "esc" }, locale.s("ds.command_palette.close")))
					if mode == "locations" then
						table.insert(parts, legend(Kbd { keys = "cmd+l" }, locale.s("shell.add_project.edit_path")))
					end
				end
				return ui.row(parts)
			end, { shrink = 0 }),
			ui.div { grow = 1 },
			ui.show(function(): boolean
				local a = at:get()
				return a.step == "folders" and not editing:get()
			end, foot_button {
				id = "project-cancel",
				label = function(): string
					return locale.s("common.cancel")
				end,
				enabled = function(): boolean
					return true
				end,
				run = function()
					if naming_now() then
						dismiss_name()
						refocus_search()
					else
						close_all()
					end
				end,
			}),
			ui.show(function(): boolean
				return at:get().step == "folders" or editing:get()
			end, foot_button {
				id = "project-add",
				label = add_label,
				enabled = can_add,
				strong = true,
				keys = "cmd+enter",
				run = function()
					if editing:peek() then
						open_typed(true)
					elseif naming_now() then
						create_folder(true)
					else
						submit()
					end
				end,
			}),
		},
	}
end

local function bar_button(props: { id: string, icon: any, label: string | () -> string, lit: (() -> boolean)?, run: () -> () }): Node
	return IconButton {
		id = props.id,
		icon = props.icon,
		label = props.label,
		size = "md",
		lit = props.lit,
		tooltip_side = "bottom",
		on_click = function()
			props.run()
		end,
	}
end
local function crumb(name: string, glyph: any, current: boolean, run: (() -> ())?): Node
	local look: { [any]: any } = {
		shrink = 0,
		h = 24,
		px = 6,
		gap = 5,
		items = "center",
		rounded = "$radius.sm",
		bg = "#00000000",
		color = if current then "$text.primary" else "$text.secondary",
		text_size = "$font.size.xs",
		if glyph then Icon { name = glyph, size = 14 } else nil,
		ui.text { name, literal = true, max_w = 180, truncate = true },
	}
	if current or not run then
		return ui.row(look)
	end
	look.label = name
	look.hover = { bg = "$hover", color = "$text.primary" }
	look.active = { bg = "$pressed", color = "$text.primary" }
	look.on_click = function()
		(run :: () -> ())()
		refocus_search()
	end
	look.tooltip = if #name > 26 then name else nil
	return ui.button(look)
end
local function separator(): Node
	return ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "alt-arrow-right", size = 12 } }
end
local crumb_menu = ui.state("shell_add_project_crumb_menu", false)
local function fold_menu(folded: { { name: string, full: string } }, a: any): Node
	local props: { [any]: any } = {
		open = crumb_menu,
		label = locale.t("shell.add_project.show_folded"),
		w = 200,
		anchor = ui.button {
			id = "project-crumb-more",
			label = locale.t("shell.add_project.show_folded"),
			tooltip = locale.s("shell.add_project.show_folded"),
			shrink = 0,
			h = 24,
			min_w = 24,
			px = 6,
			items = "center",
			justify = "center",
			rounded = "$radius.sm",
			bg = "#00000000",
			color = "$text.secondary",
			text_size = "$font.size.xs",
			hover = { bg = "$hover", color = "$text.primary" },
			on_click = function()
				crumb_menu:set(not crumb_menu:peek())
			end,
			ui.text { "…", literal = true },
		},
	}
	for _, f in folded do
		table.insert(props, ui.menu_item {
			label = f.name,
			on_click = function()
				open_folder(a.device, f.full, false, a.location)
				refocus_search()
			end,
			ui.row { gap = "$space.2", items = "center", Icon { name = "folder", size = 16 }, ui.text { f.name, literal = true } },
		})
	end
	return ui.menu(props)
end
local function trail_nodes(a: any, include_place: boolean): { Node }
	local out: { Node } = {}
	local function add(node: Node)
		if #out > 0 then
			table.insert(out, separator())
		end
		table.insert(out, node)
	end
	add(crumb(locale.s("shell.add_project.title"), nil, a.step == "devices", function()
		if nav and root_page then
			nav.pop_to(root_page)
		end
	end))
	if not a.device_name then
		return out
	end
	add(crumb(a.device_name, if a.device_here then "laptop" else "remote-server", a.step == "locations" or a.step == "repos", function()
		if nav and places_page then
			nav.pop_to(places_page)
		end
	end))
	local location: Location? = a.location
	if not include_place or not location then
		return out
	end
	local loc = location :: Location
	local root = loc.path or homes[a.device]
	local open = a.open or a.path or root
	local at_root = not open or (root ~= nil and same(open, root))
	add(crumb(loc.name, if loc.path then "hard-drive" else "home", at_root, function()
		open_folder_at_location(a.device, loc)
	end))
	if open then
		local folders = {}
		for _, c in crumbs_of(open) do
			if not (root and under(root, c.full)) then
				table.insert(folders, c)
			end
		end
		if #folders > CRUMB_MAX then
			local folded = table.move(folders, 1, #folders - CRUMB_TAIL, 1, {})
			folders = table.move(folders, #folders - CRUMB_TAIL + 1, #folders, 1, {})
			add(fold_menu(folded, a))
		end
		for _, c in folders do
			add(crumb(c.name, nil, same(c.full, open), function()
				open_folder(a.device, c.full, false, loc)
			end))
		end
	end
	return out
end
local function path_row(): Node
	return ui.row {
		id = "project-path",
		grow = 1,
		min_w = 0,
		h = 26,
		ml = 4,
		pl = "$space.2",
		pr = 3,
		gap = 6,
		items = "center",
		rounded = "$radius.md",
		bg = "$zeron.secondary.bg",
		border = 1,
		border_color = function(): string
			return if path_error:get() then "$danger" else "$accent"
		end,
		on_mount = function(el)
			anime.animate(el, motion.params({ opacity = { 0, 1 }, translateX = { -6, 0 }, duration = FADE_MS, ease = motion.EASE } :: any))
		end,
		ui.div { shrink = 0, color = "$text.secondary", Icon { name = "folder", size = 14 } },
		ui.div {
			grow = 1,
			min_w = 0,
			h = 20,
			relative = true,
			ui.input {
				id = "project-path-input",
				value = function(): string
					return path_text:get()
				end,
				label = locale.t("shell.add_project.folder_path"),
				placeholder = function(): string
					return locale.s(if windows_now() then "shell.add_project.folder_path_windows" else "shell.add_project.folder_path_like")
				end,
				placeholder_color = "$text.tertiary",
				w_full = true,
				h = 20,
				px = 0,
				border = 0,
				bg = "#00000000",
				focus = { border = 0, outline = 0 },
				text_size = "$font.size.xs",
				on_change = function(text: string)
					path_text:set(text)
					tickets.path = nil
					path_error:set(nil)
					path_opening:set(false)
					path_adding = false
					refresh_completion()
				end,
				on_key = function(e: any): boolean
					local k = e.key
					local cmd = e.meta == true or e.cmd == true
					if cmd and (k == "l" or k == "L") then
						edit_path()
						return true
					elseif cmd and e.shift and (k == "." or k == ">") then
						toggle_hidden()
						return true
					elseif k == "Tab" then
						if not e.shift then
							local to = path_completion()
							if to then
								path_text:set(to)
								refresh_completion()
								if path_field then
									local n = utf8.len(to) or #to
									path_field:set_selection(n, n)
								end
							end
						end
						return true
					elseif k == "Escape" then
						return cancel_edit()
					elseif k == "Enter" then
						if not e["repeat"] then
							open_typed(cmd)
						end
						return true
					end
					return false
				end,
				on_mount = function(el)
					path_field = el
					ui.after(0, function()
						el:focus()
						el:select_all()
					end)
				end,
			},
			ui.show(
				function(): boolean
					local _, ghost = path_completion()
					return ghost ~= nil
				end,
				ui.row {
					id = "project-path-ghost",
					absolute = true,
					left = 0,
					top = 0,
					bottom = 0,
					items = "center",
					pointer_events = "none",
					ui.text {
						function(): string
							return path_text:get()
						end,
						literal = true,
						text_size = "$font.size.xs",
						color = "#00000000",
					},
					ui.text {
						function(): string
							local _, ghost = path_completion()
							return ghost or ""
						end,
						literal = true,
						text_size = "$font.size.xs",
						color = "$text.tertiary",
					},
				}
			),
		},
		ui.show(
			function(): boolean
				return path_opening:get()
			end,
			ui.text { locale.t("shell.add_project.opening"), px = 5, shrink = 0, text_size = "$font.size.xs", color = "$text.secondary" },
			IconButton {
				id = "project-path-open",
				icon = "return",
				label = locale.t("shell.add_project.open_path"),
				size = "sm",
				tooltip_side = "bottom",
				on_click = function()
					open_typed(false)
				end,
			}
		),
	}
end
local crumb_offset = ui.state("shell_add_project_crumb_offset", 0)
local CRUMB_FADE = 18
local function step_bar(): Node
	local menu_was_open = false
	ui.effect(function()
		local now = crumb_menu:get()
		if menu_was_open and not now then
			ui.after(0, refocus_search)
		end
		menu_was_open = now
	end)
	return ui.row {
		id = "project-bar",
		w_full = true,
		h_full = true,
		px = "$space.3",
		gap = 6,
		items = "center",
		ui.presence(function(): string
			return if at:get().step == "devices" then "shell.add_project.back_to_commands" else "ds.command_palette.back"
		end, function(word: string)
			return IconButton {
				id = "project-crumb-back",
				icon = "arrow-left",
				label = locale.s(word),
				size = "md",
				tooltip_side = "bottom",
				on_click = function()
					if at:peek().step == "devices" and not M.hosted:peek() then
						M.shown:set(false)
						view.palette:set(true)
					elseif nav then
						nav.up()
						refocus_search()
					end
				end,
			}
		end, { shrink = 0 }),
		ui.presence(function(): string
			local a = at:get()
			local place = if a.location then a.location.name else ""
			return table.concat({ tostring(a.step), a.device or "", place, a.open or a.path or "", if editing:get() then "e" else "" }, "\n")
		end, function()
			local a = at:peek()
			if editing:peek() then
				local parts: { [any]: any } = { grow = 1, min_w = 0, h_full = true, gap = 2, items = "center" }
				for _, node in trail_nodes(a, false) do
					table.insert(parts, node)
				end
				table.insert(parts, separator())
				table.insert(parts, path_row())
				return ui.row(parts)
			end
			local view, content = ui.ref(), ui.ref()
			local parts: { [any]: any } = {
				ref = content,
				min_w = function(): number
					return view.width or 0
				end,
				h_full = true,
				gap = 2,
				items = "center",
			}
			for _, node in trail_nodes(a, true) do
				table.insert(parts, node)
			end
			if a.step == "locations" or a.step == "folders" then
				table.insert(parts, ui.div {
					id = "project-crumb-edit",
					grow = 1,
					min_w = 24,
					h_full = true,
					cursor = "text",
					on_click = function()
						edit_path()
					end,
				})
			end
			return ui.scroll_x {
				id = "project-crumbs",
				ref = view,
				grow = 1,
				min_w = 0,
				h_full = true,
				fade_x = function(): { number }
					local most = math.max((content.width or 0) - (view.width or 0), 0)
					local at = math.min(crumb_offset:get(), most)
					return { math.clamp(at, 0, CRUMB_FADE), math.clamp(most - at, 0, CRUMB_FADE) }
				end,
				offset = crumb_offset,
				on_mount = function()
					ui.after(0, function()
						crumb_offset:set(1e6)
					end)
				end,
				ui.row(parts),
			}
		end, { grow = 1, min_w = 0, h_full = true }),
		ui.show(function(): boolean
			return at:get().step == "folders" and not editing:get()
		end, ui.row {
			shrink = 0,
			gap = 6,
			items = "center",
			ui.presence(function(): string
				return if hidden:get() then "shown" else "hidden"
			end, function(kind: string)
				return bar_button {
					id = "project-hidden",
					icon = if kind == "shown" then "lucide-eye" else "lucide-eye-off",
					label = locale.s(if kind == "shown" then "shell.add_project.hide_hidden" else "shell.add_project.show_hidden") .. " " .. ui.shortcut_text("cmd+shift+."),
					lit = function(): boolean
						return kind == "shown"
					end,
					run = toggle_hidden,
				}
			end, { shrink = 0 }),
			bar_button {
				id = "project-new-folder-button",
				icon = "lucide-folder-plus",
				label = locale.s("shell.add_project.new_folder") .. " " .. ui.shortcut_text("cmd+shift+n"),
				run = function()
					start_name("")
				end,
			},
		}),
		ui.show(function(): boolean
			local step = at:get().step
			return (step == "locations" or step == "folders") and not editing:get()
		end, bar_button {
			id = "project-path-edit",
			icon = "pen",
			label = locale.s("shell.add_project.edit_path") .. " " .. ui.shortcut_text("cmd+l"),
			run = edit_path,
		}),
	}
end
local function step_under(): Node
	return ui.presence(function(): string?
		return if editing:get() then path_error:get() else nil
	end, function(text: string)
		return ui.text { text, literal = true, id = "project-path-error", w_full = true, px = "$space.4", pt = "$space.2", pb = 2, text_size = "$font.size.xs", color = "$danger" }
	end, { w_full = true, shrink = 0 })
end

local function rail_item(id: string, glyph: any, name: string, target: string?, run: () -> ()): Node
	local a = at:peek()
	local function selected(): boolean
		local now = at:get()
		local open = now.open or now.path
		local want = target or homes[now.device or ""]
		return if open and want then same(open, want) else false
	end
	local hint = if target then tilde(a.device, target) else (if homes[a.device] then tilde(a.device, homes[a.device]) else name)
	return ui.button {
		id = id,
		label = name,
		tooltip = hint,
		tooltip_delay = 350,
		w_full = true,
		shrink = 0,
		h = 26,
		px = "$space.2",
		gap = 7,
		items = "center",
		justify = "start",
		rounded = "$radius.md",
		text_size = "$font.size.xs",
		bg = function(): string
			return if selected() then "$zeron.menu.selected" else "#00000000"
		end,
		color = function(): string
			return if selected() then "$text.primary" else "$text.secondary"
		end,
		hover = { bg = "$hover", color = "$text.primary" },
		on_click = function()
			run()
			refocus_search()
		end,
		Icon { name = glyph, size = 14 },
		FadeText { name, literal = true, shrink = 1, text_size = "$font.size.xs" },
	}
end
local function rail_head(text: string): Node
	return ui.text { text, literal = true, shrink = 0, px = "$space.2", pt = "$space.2", pb = "$space.1", text_size = "$font.size.xs", weight = "$font.weight.medium", color = "$text.tertiary" }
end
local function go_location(device: string, location: Location)
	if nav then
		nav.replace(folder_page(device, location.path, location.name, false, location))
	end
end
open_folder_at_location = go_location
local function step_rail(): Node
	return ui.row {
		id = "project-rail",
		w = RAIL_W,
		h_full = true,
		ui.presence(function(): string
			local a = at:get()
			if not a.device then
				return ""
			end
			local names = { a.device }
			for _, p in pinned_in(a.device) do
				table.insert(names, "p" .. p)
			end
			for _, f in recent_folders(a.device) do
				table.insert(names, "r" .. f)
			end
			for _, d in drives_of:get()[a.device] or {} do
				table.insert(names, "d" .. tostring(d.path))
			end
			return table.concat(names, "\n")
		end, function()
			local a = at:peek()
			local device = a.device
			local parts: { [any]: any } = { grow = 1, min_w = 0, h_full = true, p = 6 }
			if not device then
				return ui.scroll(parts)
			end
			local pinned = pins:peek()[device] or {}
			if #pinned > 0 then
				table.insert(parts, rail_head(locale.s("shell.add_project.pinned")))
				for i, pin in pinned do
					table.insert(parts, rail_item(`project-rail-pin-{i}`, "pin", leaf_name(pin), pin, function()
						open_folder(device, pin, false, location_of(device, pin))
					end))
				end
			end
			local recent = recent_folders(device)
			if #recent > 0 then
				table.insert(parts, rail_head(locale.s("shell.add_project.recent")))
				for i, folder in recent do
					table.insert(parts, rail_item(`project-rail-recent-{i}`, "lucide-clock", leaf_name(folder), folder, function()
						open_folder(device, folder, false, location_of(device, folder))
					end))
				end
			end
			table.insert(parts, rail_head(locale.s("shell.add_project.locations")))
			local home = locale.s("shell.add_project.home")
			table.insert(parts, rail_item("project-rail-location-0", "home", home, nil, function()
				go_location(device, { name = home, path = nil })
			end))
			for i, d in drives_of:peek()[device] or {} do
				if type(d.path) == "string" then
					local dname = if type(d.name) == "string" and d.name ~= "" then d.name else d.path
					table.insert(parts, rail_item(`project-rail-location-{i}`, "hard-drive", dname, d.path, function()
						go_location(device, { name = dname, path = d.path })
					end))
				end
			end
			return ui.scroll(parts)
		end, { grow = 1, min_w = 0, h_full = true }),
		ui.div { w = 1, h_full = true, shrink = 0, bg = "$border.subtle" },
	}
end

local function name_height(): number
	local t = name_shown:get()
	local err = name_error:get()
	local lines = if err then math.ceil(#err / 80) else 0
	return FOLDER_H * t + lines * 16 + (if lines > 0 then 6 else 0) + METRICS.gap
end
local function name_row(): Node
	return ui.column {
		id = "project-new-folder",
		w_full = true,
		ui.div {
			w_full = true,
			h = function(): number
				return FOLDER_H * name_shown:get()
			end,
			opacity = name_shown,
			overflow = "hidden",
			shrink = 0,
			ui.row {
				w_full = true,
				h = FOLDER_H,
				pl = 7,
				pr = 3,
				gap = "$space.2",
				items = "center",
				rounded = "$radius.md",
				bg = "$zeron.secondary.bg",
				border = 1,
				border_color = function(): string
					return if name_error:get() then "$danger" else "$accent"
				end,
				ui.div { shrink = 0, w = 18, items = "center", color = "$text.secondary", Icon { name = "folder", size = 16 } },
				ui.input {
					id = "project-new-folder-input",
					value = function(): string
						return name_text:get()
					end,
					label = locale.t("shell.add_project.folder_name"),
					placeholder = locale.t("shell.add_project.folder_name"),
					placeholder_color = "$text.tertiary",
					grow = 1,
					min_w = 0,
					h = 20,
					px = 0,
					border = 0,
					bg = "#00000000",
					focus = { border = 0, outline = 0 },
					text_size = "$font.size.md",
					on_change = function(text: string)
						name_text:set(text)
						name_error:set(nil)
					end,
					on_key = function(e: any): boolean
						local k = e.key
						local cmd = e.meta == true or e.cmd == true
						if k == "Enter" then
							if not e["repeat"] then
								create_folder(cmd)
							end
							return true
						elseif k == "Escape" then
							dismiss_name()
							refocus_search()
							return true
						elseif k == "Tab" then
							return true
						elseif cmd and (k == "l" or k == "L") then
							edit_path()
							return true
						elseif cmd and e.shift and (k == "." or k == ">") then
							toggle_hidden()
							return true
						end
						return false
					end,
					on_mount = function(el)
						name_field = el
						ui.after(0, function()
							el:focus()
							local n = utf8.len(name_text:peek()) or 0
							el:set_selection(n, n)
						end)
					end,
				},
				ui.button {
					id = "project-new-folder-git",
					role = "checkbox",
					label = locale.t("shell.add_project.git_init"),
					checked = name_git,
					tooltip = locale.s("shell.add_project.git_init_tip"),
					shrink = 0,
					h = 22,
					px = 7,
					gap = 5,
					items = "center",
					rounded = "$radius.sm",
					text_size = "$font.size.xs",
					bg = function(): string
						return if name_git:get() then "$zeron.secondary.active" else "#00000000"
					end,
					color = function(): string
						return if name_git:get() then "$text.primary" else "$text.secondary"
					end,
					hover = { bg = "$zeron.secondary.active", color = "$text.primary" },
					on_click = function()
						name_git:set(not name_git:peek())
						if name_field then
							name_field:focus()
						end
					end,
					Icon { name = "git-branch", size = 12 },
					ui.text { locale.t("shell.add_project.git_init") },
				},
				ui.show(function(): boolean
					return name_busy:get()
				end, ui.text { locale.t("shell.add_project.creating"), shrink = 0, px = 5, text_size = "$font.size.xs", color = "$text.secondary" }, IconButton {
					id = "project-new-folder-create",
					icon = "return",
					label = locale.t("shell.add_project.create_and_open"),
					tooltip = locale.s("shell.add_project.create_and_open") .. " ↵",
					size = "sm",
					on_click = function()
						create_folder(false)
					end,
				}),
			},
		},
		ui.presence(function(): string?
			return name_error:get()
		end, function(text: string)
			return ui.text { text, literal = true, id = "project-new-folder-error", wrap = true, pl = 34, pt = 4, pb = 2, text_size = "$font.size.xs", color = "$danger" }
		end, {
			w_full = true,
			enter = motion.transition({ opacity = { 0, 1 }, duration = FADE_MS, ease = "outQuad" }),
		}),
	}
end
local function step_inline(): any
	if not naming:get() then
		return nil
	end
	return { key = "inline:new-folder", after = "here", h = name_height, build = name_row }
end
local function refocus(): boolean
	local holder = if editing:peek() then path_field elseif naming_now() then name_field else false
	if holder == false then
		return false
	end
	if holder then
		holder:focus()
	end
	return true
end

local function drop_target(): boolean
	local a = at:peek()
	return a.step ~= "devices" and a.device ~= nil and a.device == sessions.now:peek().here
end
local function dropped(files: { any })
	drop_over:set(false)
	if not drop_target() or busy:peek() then
		return
	end
	local device = at:peek().device
	local alive = ticket("drop")
	local function try(i: number)
		local f = files[i]
		if not f or type(f.path) ~= "string" then
			return
		end
		projects.folders(device, f.path, true):after(function(listing: any)
			if not alive() then
				return
			end
			M.add(device, if type(listing) == "table" and type(listing.path) == "string" then listing.path else f.path, has_git(listing))
		end, function()
			if alive() then
				try(i + 1)
			end
		end)
	end
	try(1)
end
local function drop_card(): Node
	return ui.presence(function(): string?
		return if drop_over:get() then "over" else nil
	end, function()
		return ui.column {
			id = "project-drop",
			w_full = true,
			h_full = true,
			items = "center",
			justify = "center",
			gap = 4,
			rounded = "$radius.xl",
			border = 2,
			border_dash = { 6, 4 },
			border_color = "$accent",
			ui.div { absolute = true, left = 0, right = 0, top = 0, bottom = 0, rounded = "$radius.xl", bg = "$zeron.palette.solid", opacity = 0.88 },
			ui.text { locale.t("shell.add_project.drop_title"), text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
			ui.text { locale.t("shell.add_project.drop_hint"), text_size = "$font.size.xs", color = "$text.secondary" },
		}
	end, {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		enter = motion.transition({ opacity = { 0, 1 }, duration = FADE_MS, ease = "outQuad" }),
		exit = motion.transition({ opacity = { 1, 0 }, duration = FADE_MS, ease = "outQuad" }),
	})
end

local function dress(page: Page, fields: { [string]: any }): Page
	page.plain = true
	page.metrics = METRICS
	page.view_h = VIEW_H
	page.bar = step_bar
	page.under = step_under
	page.foot = step_foot
	page.over = drop_card
	page.keys = picker_keys
	page.edit_path = edit_path
	page.shortcut = "cmd+shift+n"
	page.refocus = refocus
	page.on_search_focus = function()
		if editing:peek() then
			leave_path()
		end
		if naming_now() then
			dismiss_name()
		end
	end
	page.file_hover = function(inside: boolean)
		drop_over:set(inside and drop_target())
	end
	page.file_drop = dropped
	local told = fields.shown
	page.on_show = function(palette: any)
		nav = palette
		name_after_load = false
		tickets.typed = nil
		if naming_now() or editing:peek() then
			ui.after(0, refocus_search)
		end
		leave_path()
		dismiss_name(true)
		told()
	end
	return page
end

local function folder_item(page: any, device: string, opts: { id: string, path: string, name: string, repo: boolean, detail: string?, quiet: boolean?, strong: boolean?, here: boolean? }): Item
	local path, repo, here = opts.path, opts.repo, opts.here == true
	local function add()
		M.add(device, path, repo)
	end
	return {
		id = opts.id,
		label = opts.name,
		section = page.row_section or "",
		icon = "folder",
		detail = opts.detail,
		quiet = opts.quiet,
		strong = opts.strong,
		idle_only = here,
		inert = here,
		stay = true,
		click_selects = true,
		keywords = opts.keywords,
		target = path,
		page = if here then nil else function(): Page
			return folder_page(device, path, opts.name, repo, page.location)
		end,
		alt = { label = locale.s("shell.add_project.add_project"), run = add },
		tag = function(): string?
			return if is_added(device, path) then locale.s("shell.add_project.added") else nil
		end,
		actions = function(): Node
			local parts: { [any]: any } = { gap = 2, items = "center" }
			if not here then
				table.insert(parts, IconButton {
					icon = "lucide-folder-plus",
					label = locale.t("shell.add_project.new_folder_inside"),
					size = "sm",
					on_click = function()
						open_folder(device, path, repo, page.location)
						name_after_load = true
						refocus_search()
					end,
				})
			end
			table.insert(parts, ui.presence(function(): string
				return if is_pinned(device, path) then "on" else "off"
			end, function(state: string)
				return IconButton {
					icon = "pin",
					label = locale.s(if state == "on" then "shell.add_project.unpin" else "shell.add_project.pin"),
					size = "sm",
					lit = function(): boolean
						return state == "on"
					end,
					on_click = function()
						toggle_pin(device, path)
						if not refocus() then
							refocus_search()
						end
					end,
				}
			end, { shrink = 0 }))
			table.insert(parts, ui.button {
				label = locale.t("shell.add_project.add_as_project"),
				h = 22,
				px = "$space.2",
				items = "center",
				rounded = "$radius.sm",
				bg = "#00000000",
				border = 1,
				border_color = "$border",
				text_size = "$font.size.xs",
				color = "$text.primary",
				hover = { bg = "$hover" },
				on_click = add,
				ui.text { locale.t("shell.add_project.add") },
			})
			return ui.row(parts)
		end,
		trail = function(): Node
			local parts: { [any]: any } = { shrink = 0, gap = "$space.1", items = "center" }
			if repo then
				table.insert(parts, ui.div { color = "$text.secondary", Icon { name = "git-branch", size = 14 } })
			end
			if not here then
				table.insert(parts, ui.button {
					label = locale.t("shell.add_project.open"),
					w = 20,
					h = 20,
					items = "center",
					justify = "center",
					rounded = "$radius.sm",
					bg = "#00000000",
					color = "$text.tertiary",
					hover = { bg = "$hover" },
					on_click = function()
						open_folder(device, path, repo, page.location)
						refocus_search()
					end,
					Icon { name = "alt-arrow-right", size = 14 },
				})
			end
			return ui.row(parts)
		end,
	}
end

local function go_to(device: string, query: string): any
	local typed = string.match(query, "^%s*(.-)%s*$") or query
	local name = string.match((string.gsub(typed, "[/\\]+$", "")), "([^/\\]+)$") or typed
	return {
		id = `goto:{typed}`,
		label = typed,
		section = locale.s("shell.add_project.go_to"),
		icon = "folder",
		keywords = query,
		verb = locale.s("shell.add_project.open"),
		stay = true,
		page = function(): Page
			return folder_page(device, typed, name, false, nil)
		end,
		alt = {
			label = locale.s("shell.add_project.add_project"),
			run = function()
				local alive = ticket("typed", nav and nav.page())
				resolve(device, typed, function(at_path: string?, why: any?)
					if not alive() then
						return
					end
					if not at_path then
						failure:set(says(why, typed))
						return
					end
					local found = at_path :: string
					projects.folders(device, found, true):after(function(listing: any)
						if alive() then
							M.add(device, found, has_git(listing))
						end
					end, function(err: any)
						if alive() then
							failure:set(says(err, found))
						end
					end)
				end)
			end,
		},
	}
end

local function jump(device: string, query: string): Page?
	local q = string.match(query, "^%s*(.-)%s*$") or ""
	local last = string.sub(q, -1)
	if (last ~= "/" and last ~= "\\") or not typed_path(q) then
		return nil
	end
	if key_of(q) == "/" or string.match(q, "^%a:[\\/]$") or string.sub(q, 1, 2) == "\\\\" and not string.find(string.sub(q, 3), "[/\\].") then
		return nil
	end
	local name = string.match(key_of(q), "([^/\\]+)$") or q
	return folder_page(device, q, name, false, nil)
end

type Dir = { name: string, repo: boolean }
local function starts(name: string, typed: string): boolean
	return string.lower(string.sub(name, 1, #typed)) == string.lower(typed)
end
local function named(dirs: { Dir }, typed: string): Dir?
	for _, d in dirs do
		if d.name == typed then
			return d
		end
	end
	for _, d in dirs do
		if string.lower(d.name) == string.lower(typed) then
			return d
		end
	end
	local only: Dir? = nil
	for _, d in dirs do
		if starts(d.name, typed) then
			if only then
				return nil
			end
			only = d
		end
	end
	return only
end
local function completing(dirs: { Dir }, typed: string): Dir?
	for _, d in dirs do
		if d.name == typed then
			return d
		end
	end
	for _, d in dirs do
		if starts(d.name, typed) then
			return d
		end
	end
	return nil
end

folder_page = function(device: string, path: string?, title: string, git: boolean, location: Location?, preload: any?): Page
	local page: Page = {
		id = `folder:{device}:{path or "~"}:{if hidden:peek() then "h" else ""}`,
		title = title,
		placeholder = locale.s("shell.add_project.search_folders"),
		loading = { title = locale.s("shell.add_project.loading_folders") },
		location = location,
		width = WIDE,
		fill = true,
		replaces = true,
		rail = step_rail,
		inline = step_inline,
		keys_elsewhere = function(): boolean
			return naming:get() and not name_leaving:get()
		end,
	}
	local here: string? = path
	local listed_ok = false
	local failed: string? = nil
	local listed: { [string]: boolean } = {}
	local dirs: { Dir } = {}
	local others: { Dir } = {}
	local function is_home(): boolean
		return page.location ~= nil and page.location.path == nil
	end
	local function list(ok: (any) -> (), fail: (any) -> ())
		if preload then
			ok(preload)
			return
		end
		local show = hidden:peek()
		if not path then
			if location and location.path then
				projects.folders(device, location.path, show):after(ok, fail)
			else
				projects.folders(device, nil, show):after(ok, fail)
			end
			return
		end
		resolve(device, path, function(found: string?, why: any?)
			if found then
				projects.folders(device, found, show):after(ok, fail)
			else
				fail(why)
			end
		end)
	end
	local function told_at()
		set_at({
			step = "folders",
			device = device,
			path = here or path or (if page.location then page.location.path else nil) or false,
			open = if listed_ok then here else false,
			location = page.location or false,
			title = title,
			repo = git,
		})
	end
	page.load = function(done: ({ Item }) -> ())
		list(function(listing: any)
			here = if type(listing) == "table" and type(listing.path) == "string" then listing.path else path
			local function rows()
				if not path and here and is_home() then
					homes[device] = here :: string
				end
				git = git or has_git(listing)
				listed_ok = true
				if here then
					listed[key_of(here :: string)] = true
				end
				local out: { Item } = {}
				local at_path = here :: string
				local root = if page.location and page.location.path then page.location.path else homes[device]
				if here and not (root and same(at_path, root)) then
					table.insert(out, folder_item(page, device, { id = "here", path = at_path, name = locale.s("shell.add_project.this_folder"), repo = git, detail = tilde(device, at_path), strong = true, here = true }))
				end
				local show = hidden:peek()
				for _, e in (if type(listing) == "table" and type(listing.entries) == "table" then listing.entries else {}) do
					if not e.is_dir and type(e.name) == "string" then
						table.insert(others, { name = e.name, repo = false })
					end
					local dot = type(e.name) == "string" and string.sub(e.name, 1, 1) == "."
					if e.is_dir and type(e.name) == "string" and (show or not dot) and here then
						local child = join(at_path, e.name)
						local repo = e.is_repo == true
						listed[key_of(child)] = true
						table.insert(dirs, { name = e.name, repo = repo })
						table.insert(out, folder_item(page, device, { id = `dir:{child}`, path = child, name = e.name, repo = repo, quiet = dot }))
					end
				end
				if on_page(page) then
					told_at()
				end
				done(out)
				if name_after_load and on_page(page) then
					name_after_load = false
					start_name("")
				end
			end
			if page.location or not here then
				rows()
			elseif homes[device] then
				page.location = location_of(device, here :: string)
				rows()
			else
				with_home(device, function()
					page.location = location_of(device, here :: string)
					rows()
				end)
			end
		end, function(err: any)
			failed = says(err, path)
			if on_page(page) then
				told_at()
			end
			done({})
		end)
	end
	page.up = function(): Page?
		local open = here or path
		if not open then
			return nil
		end
		local root = if page.location and page.location.path then page.location.path else homes[device]
		if root and same(open, root) then
			return nil
		end
		local parent = parent_of(open)
		if not parent then
			return nil
		end
		return folder_page(device, parent, leaf_name(parent), false, page.location or location_of(device, parent))
	end
	page.note = function(query: string): any
		if failed then
			return {
				title = failed,
				hint = locale.s("shell.add_project.folder_failed_hint"),
				action = {
					label = locale.s("shell.add_project.retry"),
					run = function()
						if nav then
							nav.replace(folder_page(device, path, title, git, page.location))
						end
					end,
				},
			}
		end
		if query == "" then
			return {
				title = locale.s("shell.add_project.no_folders_here"),
				hint = if editing:get() then locale.s("shell.add_project.no_folders_here_editing") else locale.s("shell.add_project.no_folders_here_hint", { keys = ui.shortcut_text("cmd+enter") }),
			}
		end
		local state = home_state:get()
		if is_home() and state.q == query and state.status == "searching" then
			return { title = locale.s("shell.add_project.searching_home"), hint = locale.s("shell.add_project.searching_home_hint") }
		elseif is_home() and state.q == query and state.status == "indexing" then
			return { title = locale.s("shell.add_project.no_match_yet"), hint = locale.s("shell.add_project.no_match_yet_hint") }
		end
		return { title = locale.s("shell.add_project.no_folders_match"), hint = locale.s("shell.add_project.no_folders_hint") }
	end
	page.status = function(section: string, query: string): string?
		local state = home_state:get()
		if section == locale.s("shell.add_project.in_home") and state.q == query then
			return if state.status == "indexing" then locale.s("shell.add_project.indexing") elseif state.status == "searching" then locale.s("shell.add_project.searching") else nil
		end
		return nil
	end
	page.path = function(): string?
		if not here then
			return nil
		end
		local shown_at = tilde(device, here :: string)
		local sep = if string.find(shown_at, "\\", 1, true) and not string.find(shown_at, "/", 1, true) then "\\" else "/"
		return if string.sub(shown_at, -1) == sep then shown_at else shown_at .. sep
	end
	page.typed = function(query: string): Page?
		local q = string.match(query, "^%s*(.-)%s*$") or ""
		local last = string.sub(q, -1)
		if last ~= "/" and last ~= "\\" or not here then
			return nil
		end
		if typed_path(q) then
			local shown_at = page.path()
			if shown_at and (q == shown_at or key_of(q) == key_of(here :: string)) then
				return nil
			end
			return jump(device, q)
		end
		local typed = string.sub(q, 1, -2)
		if typed == "" or string.find(typed, "[/\\]") then
			return nil
		end
		local d = named(dirs, typed) or named(others, typed)
		if not d then
			return nil
		end
		local dir = d :: Dir
		return folder_page(device, join(here :: string, dir.name), dir.name, dir.repo, page.location)
	end
	local function completion(query: string): (string?, string?)
		if query == "" or query == "." or query == ".." or string.find(query, "[/\\]") then
			return nil, nil
		end
		local d = completing(dirs, query)
		if not d then
			return nil, nil
		end
		local name = (d :: Dir).name
		if #query >= #name then
			return nil, nil
		end
		return name, string.sub(name, #query + 1)
	end
	page.complete = function(query: string): string?
		local name = completion(query)
		return name
	end
	page.ghost = function(query: string): string?
		local _, rest = completion(query)
		return rest
	end
	local asked = 0
	local function in_home(query: string): any
		asked += 1
		local mine = asked
		warm(device)
		local q = string.match(query, "^%s*(.-)%s*$") or ""
		if (utf8.len(q) or 0) < HOME_LEAST or typed_path(q) then
			home_state:set({})
			return nil
		end
		home_state:set({ q = q, status = "searching" })
		return {
			after = function(_, ok: (any) -> ())
				local wait = HOME_RETRY
				local function look()
					projects.search_home(device, q):after(function(found: any)
						if mine ~= asked or not on_page(page) then
							return
						end
						local out: { Item } = {}
						for _, m in (if type(found) == "table" and type(found.matches) == "table" then found.matches else {}) do
							if type(m.path) == "string" and not listed[key_of(m.path)] then
								local name, place = home_place(if type(m.relative) == "string" then m.relative else m.path)
								local item = folder_item({ row_section = locale.s("shell.add_project.in_home"), location = page.location }, device, { id = `home:{m.path}`, path = m.path, name = name, repo = m.is_repo == true, detail = place, keywords = q })
								table.insert(out, item)
							end
						end
						local more = type(found) == "table" and found.indexing == true
						home_state:set({ q = q, status = if more then "indexing" else nil })
						ok(out)
						if more then
							ui.after(wait, look)
							wait = math.min(wait * 2, HOME_RETRY_MOST)
						end
					end, function()
						if mine == asked and on_page(page) then
							home_state:set({})
							ok({})
						end
					end)
				end
				look()
			end,
		}
	end
	local function clone_into(url: string, repo: string)
		local parent = here
		if not parent or busy:peek() then
			return
		end
		leave_path()
		busy:set("cloning")
		cloning:set(repo)
		failure:set(nil)
		local alive = ticket("add")
		projects.clone(device, url, parent :: string):after(function(cloned: any)
			if not alive() then
				return
			end
			busy:set(nil)
			if type(cloned) == "table" and type(cloned.path) == "string" then
				M.add(device, cloned.path, true)
			else
				cloning:set(nil)
			end
		end, function(err: any)
			if not alive() then
				return
			end
			busy:set(nil)
			cloning:set(nil)
			failure:set(locale.s("shell.add_project.couldnt_clone", { repo = repo }) .. ": " .. says(err))
		end)
	end
	local function clone_item(id: string, url: string, repo: string, section: string, aria: string?, private: boolean?): Item
		return {
			id = id,
			label = repo,
			aria = aria,
			section = section,
			icon = "git-branch",
			keywords = url .. " " .. repo,
			hint = locale.s("shell.add_project.clone_here_short"),
			detail = if private then locale.s("shell.add_project.private") else nil,
			verb = locale.s("shell.add_project.clone"),
			stay = true,
			clone_of = repo,
			run = function()
				clone_into(url, repo)
			end,
			alt = {
				label = locale.s("shell.add_project.clone"),
				run = function()
					clone_into(url, repo)
				end,
			},
		}
	end
	local function from_github(query: string): { Item }
		local needle = string.lower(string.match(query, "^%s*(.-)%s*$") or "")
		if needle == "" or string.find(needle, "^[/~%.]") then
			return {}
		end
		local out: { Item } = {}
		for _, r in cloud.now:peek().repos or {} do
			if #out >= CLONE_MATCHES then
				break
			end
			if r.clone_url and string.find(string.lower(r.name), needle, 1, true) then
				table.insert(out, clone_item(`github:{r.name}`, r.clone_url :: string, r.name, locale.s("shell.add_project.from_github"), nil, r.private))
			end
		end
		return out
	end
	page.tail = function(query: string, matched: number, searching: boolean): { Item }
		local q = string.match(query, "^%s*(.-)%s*$") or ""
		if q == "" or matched > 0 or not listed_ok or naming:get() or searching or typed_path(q) or git_url(q) then
			return {}
		end
		local state = home_state:get()
		if is_home() and state.q == q and state.status ~= nil then
			return {}
		end
		return {
			{
				id = `create:{q}`,
				label = locale.s("shell.add_project.create_folder_in", { name = q, path = tilde(device, here :: string) }),
				section = "",
				icon = "lucide-folder-plus",
				keywords = q,
				stay = true,
				create_of = q,
				run = function()
					start_name(q)
				end,
				alt = {
					label = locale.s("shell.add_project.create_named", { name = q }),
					run = function()
						start_name(q)
						create_folder(true)
					end,
				},
			},
		}
	end
	page.search = function(query: string): any
		local repo = git_url(query)
		if not repo then
			if typed_path(string.match(query, "^%s*(.-)%s*$") or "") then
				asked += 1
				home_state:set({})
				return offer(go_to(device, query))
			end
			local github = from_github(query)
			local found = if is_home() then in_home(query) else nil
			if not found then
				return if #github > 0 then offer_all(github) else nil
			end
			return {
				after = function(_, ok: (any) -> ())
					if #github > 0 then
						ok(github)
					end
					found:after(function(list: any)
						local all = table.clone(list or {})
						for _, g in github do
							table.insert(all, g)
						end
						ok(all)
					end)
				end,
			}
		end
		asked += 1
		home_state:set({})
		local url = string.match(query, "^%s*(.-)%s*$") or query
		return {
			after = function(_, ok: (any) -> ())
				ok({ clone_item(`clone:{url}`, url, repo, "", locale.s("shell.add_project.clone_here", { repo = repo }), nil) })
			end,
		}
	end
	return dress(page, {
		shown = function()
			told_at()
			if not is_home() and location ~= nil then
				release_homes()
			end
		end,
	})
end

local function locations_page(device: string, name: string, platform: string?, device_here: boolean): Page
	local page: Page = {
		id = `places:{device}`,
		title = name,
		placeholder = locale.s("shell.add_project.search_locations"),
		loading = { title = locale.s("shell.add_project.loading_locations") },
		empty = { title = locale.s("shell.add_project.no_locations"), hint = locale.s("shell.add_project.no_locations_hint") },
	}
	page.search = function(query: string): any
		if typed_path(string.match(query, "^%s*(.-)%s*$") or "") then
			return offer(go_to(device, query))
		end
		return nil
	end
	page.typed = function(query: string): Page?
		return jump(device, query)
	end
	page.load = function(done: ({ Item }) -> ())
		local function home(): Item
			return {
				id = "home",
				label = locale.s("shell.add_project.home"),
				section = "",
				icon = "home",
				verb = locale.s("shell.add_project.open"),
				page = function(): Page
					local at_name = locale.s("shell.add_project.home")
					return folder_page(device, nil, at_name, false, { name = at_name, path = nil })
				end,
			}
		end
		projects.drives(device):after(function(drives: any)
			local out: { Item } = { home() }
			local known = {}
			for _, d in (if type(drives) == "table" then drives else {}) do
				if type(d.path) == "string" then
					local dname = if type(d.name) == "string" and d.name ~= "" then d.name else d.path
					table.insert(known, { name = dname, path = d.path })
					table.insert(out, {
						id = `drive:{d.path}`,
						label = dname,
						section = "",
						icon = "hard-drive",
						verb = locale.s("shell.add_project.open"),
						page = function(): Page
							return folder_page(device, d.path, dname, false, { name = dname, path = d.path })
						end,
					})
				end
			end
			local all = table.clone(drives_of:peek())
			all[device] = known
			drives_of:set(all)
			done(out)
		end, function()
			done({ home() })
		end)
	end
	return dress(page, {
		shown = function()
			places_page = page
			release_homes()
			set_at({ step = "locations", device = device, device_name = name, device_here = device_here, platform = platform, path = false, open = false, location = false })
		end,
	})
end

local function cloud_page(): Page
	local page: Page = {
		id = "cloud-repos",
		title = locale.s("shell.add_project.cloud_title"),
		placeholder = locale.s("shell.add_project.search_repos"),
		loading = { title = locale.s("shell.add_project.loading_repos") },
		empty = { title = locale.s("shell.add_project.no_repos"), hint = locale.s("shell.add_project.no_repos_hint") },
	}
	local function to_settings(id: string, label: string, section: string): Item
		return {
			id = id,
			label = label,
			section = section,
			icon = "arrow-up-right",
			verb = locale.s("shell.add_project.open"),
			run = function()
				route.settings("cloud")
			end,
		}
	end
	page.load = function(done: ({ Item }) -> ())
		cloud.load_repos(function()
			local now = cloud.now:peek()
			local why = now.repos_failure
			if why == "github_not_connected" then
				done({ to_settings("connect-github", locale.s("shell.add_project.connect_github"), locale.s("shell.add_project.github_not_connected")) })
				return
			elseif why and not now.repos then
				page.empty = {
					title = if why == "offline" then locale.s("shell.add_project.repos_offline") else locale.s("shell.add_project.repos_failed"),
					hint = locale.s("shell.add_project.repos_failed_hint"),
				}
				done({})
				return
			end
			local repos = table.clone(now.repos or {})
			if #repos == 0 then
				done({ to_settings("choose-repos", locale.s("shell.add_project.choose_repos"), locale.s("shell.add_project.no_repos_yet")) })
				return
			end
			local function owner(r: cloud.Repo): string
				return string.match(r.name, "^([^/]+)/") or r.name
			end
			local newest: { [string]: number } = {}
			for _, r in repos do
				local o = owner(r)
				newest[o] = math.max(newest[o] or 0, r.pushed or 0)
			end
			table.sort(repos, function(a, b)
				local oa, ob = owner(a), owner(b)
				if oa ~= ob then
					if newest[oa] ~= newest[ob] then
						return newest[oa] > newest[ob]
					end
					return string.lower(oa) < string.lower(ob)
				end
				if (a.pushed or 0) ~= (b.pushed or 0) then
					return (a.pushed or 0) > (b.pushed or 0)
				end
				return string.lower(a.name) < string.lower(b.name)
			end)
			local snap = sessions.now:peek()
			local names: { [string]: string } = {}
			for _, d in snap.devices do
				names[d.id] = d.name
			end
			local out: { Item } = {}
			for _, r in repos do
				local on: string? = nil
				for _, p in snap.projects do
					if p.github and string.lower(p.github :: string) == string.lower(r.name) and string.sub(p.device, 1, 6) ~= "cloud-" then
						on = names[p.device]
					end
				end
				local words = {}
				if on then
					table.insert(words, locale.s("shell.add_project.on_device", { device = on }))
				end
				if r.private then
					table.insert(words, locale.s("shell.add_project.private"))
				end
				local name = r.name
				table.insert(out, {
					id = `repo:{name}`,
					label = name,
					section = owner(r),
					icon = "git-branch",
					hint = if #words > 0 then table.concat(words, " · ") else nil,
					verb = locale.s("shell.add_project.add_cloud_project"),
					stay = true,
					run = function()
						local cloud_at = cloud.device()
						if not cloud_at then
							return
						end
						local short = string.match(name, "([^/]+)$") or name
						M.add(cloud_at :: string, `/home/user/{short}`, true, name)
					end,
				})
			end
			done(out)
		end)
	end
	return dress(page, {
		shown = function()
			places_page = page
			set_at({ step = "repos", device = "cloud", device_name = locale.s("shell.add_project.cloud"), device_here = false, path = false, open = false, location = false })
		end,
	})
end

local function devices(): { Item }
	local now = sessions.now:get()
	local out: { Item } = {}
	for _, d in sessions.computers(now.devices) do
		local here = d.here or d.id == now.here
		local online = here or d.online ~= false
		local item = {
			id = `device:{d.id}`,
			label = d.name,
			section = "",
			icon = if here then "laptop" else "remote-server",
			hint = if not online then locale.s("shell.add_project.offline") else nil,
			verb = locale.s("shell.add_project.open"),
			device_row = true,
			trail = function(): Node
				return ui.div { shrink = 0, w = 5, h = 5, rounded = "$radius.full", bg = if online then "$success" else "$text.tertiary" }
			end,
			page = function(): Page
				return locations_page(d.id, d.name, d.platform, here)
			end,
		}
		if here then
			table.insert(out, 1, item)
		else
			table.insert(out, item)
		end
	end
	if cloud.on() then
		table.insert(out, {
			id = "device:cloud",
			label = locale.s("shell.add_project.cloud"),
			section = "",
			icon = "cloud",
			hint = locale.s("shell.add_project.cloud_hint"),
			verb = locale.s("shell.add_project.open"),
			page = cloud_page,
		})
	end
	return out
end

local recent = ui.state("shell_add_project_recent", {} :: { string })

local function only_device(): (any, boolean)
	if cloud.on() then
		return nil, false
	end
	local now = sessions.now:peek()
	local list = sessions.computers(now.devices)
	if #list ~= 1 then
		return nil, false
	end
	local d = list[1]
	return d, d.here or d.id == now.here
end

local function root(): Page
	local page: Page = dress({ id = "new-project", items = devices }, {
		shown = function()
			at:set({ step = "devices" })
		end,
	})
	local on_show = page.on_show
	page.on_show = function(palette: any)
		root_page = page
		on_show(palette)
	end
	page.enter = function(): { Page }?
		local d, here = only_device()
		if not d then
			return nil
		end
		local home = locale.s("shell.add_project.home")
		return { locations_page(d.id, d.name, d.platform, here), folder_page(d.id, nil, home, false, { name = home, path = nil }) }
	end
	return setmetatable(page, {
		__index = function(_, field: string): any
			if field == "title" then
				return locale.s("shell.add_project.title")
			elseif field == "placeholder" then
				return locale.s("shell.add_project.search_devices")
			elseif field == "empty" then
				return { title = locale.s("shell.add_project.no_devices"), hint = locale.s("shell.add_project.no_devices_hint") }
			end
			return nil
		end,
	})
end
function M.page(): Page
	M.hosted:set(true)
	return root()
end

return setmetatable(M, {
	__call = function(): Node
		return ui.div {
			id = "shell-add-project",
			CommandPalette {
				open = M.shown,
				recent = recent,
				root = root(),
				rises = true,
			},
		}
	end,
})
