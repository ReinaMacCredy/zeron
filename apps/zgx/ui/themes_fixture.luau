-- Settings → Themes and Settings → Appearance, mounted on their own with
-- themes made up here (no host in a test): the pages
-- tests/themes_in_place.test.luau drives. What src/themes.rs would publish
-- is set here (`themes.library`: a few of Zeron's built-in themes, a
-- template's own, and what you import), and what the pages ask of it is
-- stubbed to change this data as the host does (its Rust tests run the real
-- thing). Settings' route picks the page: Themes, or Appearance for any
-- other.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local palettes = require("@ds/palettes")
-- (No window chrome to restyle in a test host: the scheme is the theme's.)
local actions = require("@app/screens/onboarding/actions");
(actions :: any).apply_scheme = function(mode: string)
	ui.theme.follow_system(false)
	ui.theme.current:set(if mode == "light" then palettes.scheme.light else palettes.scheme.dark)
end
local appearance = require("@app/screens/shell/appearance")
local templates = require("@app/screens/templates/state")
local state = require("@app/screens/themes/state")
-- (The real calls, for `old_host`: a host from before the library's actions has none of them.)
local real_host = table.clone(state.host)
local page = require("@app/screens/themes/page")
local settings_appearance = require("@app/screens/settings/appearance")
local route = require("@app/screens/shell/route")
local data = require("@app/themes_fixture_data")

appearance.scheme:set("system")

local model = { list = table.clone(data.BUILTIN), entries = table.clone(data.ENTRIES), chosen = {} :: { [string]: string } }
local function publish()
	palettes.library():set({ state = "ready", list = table.clone(model.list), entries = table.clone(model.entries), chosen = table.clone(model.chosen) })
end

-- Two templates: the default in use, and Ledger, which brings a dark theme.
templates.now:set({
	state = "ready",
	active = "zeron",
	list = {
		{ id = "zeron", name = "Zeron", builtin = true, active = true, status = "ok" },
		{ id = "ledger", name = "Ledger", author = "you", builtin = false, active = false, status = "ok" },
	},
} :: any)

-- A promise settled at once, as a host's does later.
local function settled(ok: boolean, value: any): any
	return {
		after = function(_self: any, done: (any) -> (), failed: ((any) -> ())?)
			if ok then
				done(value)
			elseif failed then
				failed(value)
			end
		end,
	}
end

local said = ui.state("calls", "")
local worn = ui.state("worn", "")
local slots_said = ui.state("slots", "")
local function note(what: string)
	said:set(if said:peek() == "" then what else `{said:peek()},{what}`)
end

state.host.choose = function(slots: { [string]: string }): any
	for _, kind in { "light", "dark" } do
		local id = slots[kind]
		if id then
			note(`choose:{kind}:{id}`)
			model.chosen[kind] = if id == "" then nil else id
		end
	end
	publish()
	return settled(true, nil)
end
state.host.import = function(path: string): any
	local name = string.match(path, "([^/]+)$") or path
	note(`import:{name}`)
	if string.find(path, "Broken", 1, true) then
		return settled(false, { message = `could not parse JSONC theme {path}` })
	end
	table.insert(model.list, table.clone(data.HARBOR))
	table.insert(model.entries, { id = data.HARBOR.entry, name = data.HARBOR.name, source = "imported", status = { ok = true }, variants = { data.HARBOR.id } })
	publish()
	return settled(true, { entry = data.HARBOR.entry, variants = { data.HARBOR.id }, skipped = {} })
end
state.host.remove = function(id: string)
	note(`remove:{id}`)
	local gone: { [string]: boolean } = {}
	for i = #model.entries, 1, -1 do
		local e = model.entries[i]
		if e.id == id then
			for _, v in e.variants do
				gone[v] = true
			end
			table.remove(model.entries, i)
		end
	end
	for kind, v in model.chosen do
		if gone[v] then
			model.chosen[kind] = nil
		end
	end
	publish()
end
state.host.rename = function(id: string, name: string): any
	note(`rename:{id}:{name}`)
	for _, e in model.entries do
		if e.id == id then
			e.name = name
		end
	end
	publish()
	return settled(true, nil)
end

-- The library's own (src/themes.rs, its Rust tests run the real thing):
-- Analyze lists a source's variants ("Pair" a package of Harbor Night and
-- Paper Mill), Add brings those picked in, a copy or linked, and a linked
-- entry reloads, breaking once the `break_source` command says so.
local PAPER = table.clone(data.HARBOR)
PAPER.id, PAPER.name, PAPER.kind, PAPER.entry, PAPER.background = "custom-pair-paper-mill", "Paper Mill", "light", "custom-pair", "#f6f1e7"
local source_broken = false
state.host.analyze = function(path: string): any
	local name = string.match(path, "([^/]+)$") or path
	note(`analyze:{name}`)
	if string.find(path, "Broken", 1, true) then
		return settled(false, { message = `could not parse JSONC theme {path}` })
	end
	if string.find(path, "Pair", 1, true) then
		return settled(true, { kind = "package", name = "Pair", variants = {
			{ id = "custom-pair-harbor-night", name = "Harbor Night", kind = "dark" },
			{ id = PAPER.id, name = "Paper Mill", kind = "light" },
		}, skipped = { "Ghost: could not resolve ghost.json" } })
	end
	return settled(true, { kind = "file", name = "Harbor Night", variants = { { id = data.HARBOR.id, name = "Harbor Night", kind = "dark" } }, skipped = {} })
end
state.host.add = function(path: string, variants: { string }, link: boolean): any
	local name = string.match(path, "([^/]+)$") or path
	note(`add:{name}:{table.concat(variants, "+")}:{if link then "link" else "copy"}`)
	local pair = string.find(path, "Pair", 1, true) ~= nil
	local entry = if pair then "custom-pair" else data.HARBOR.entry
	local ids: { string } = {}
	for _, id in variants do
		local p = if id == PAPER.id then table.clone(PAPER) else table.clone(data.HARBOR)
		p.id, p.entry = id, entry
		table.insert(model.list, p)
		table.insert(ids, id)
	end
	table.insert(model.entries, {
		id = entry, name = if pair then "Pair" else data.HARBOR.name, source = "imported", status = { ok = true }, variants = ids,
		origin = if link then "linked_file" else "imported", path = path,
	})
	publish()
	return settled(true, { entry = entry, variants = ids, skipped = {} })
end
local function entry_named(id: string): any
	for _, e in model.entries do
		if e.id == id then
			return e
		end
	end
	return nil
end
state.host.entry = function(action: string, id: string): any
	note(`{action}:{id}`)
	local e = entry_named(id)
	if action == "reload" then
		if source_broken then
			e.status = { ok = false, message = "could not parse JSONC theme" }
			publish()
			return settled(false, { message = "could not parse JSONC theme" })
		end
		e.status = { ok = true }
		publish()
	elseif action == "unlink" then
		e.origin, e.status = "imported", { ok = true }
		publish()
	elseif action == "duplicate" then
		local copy = table.clone(e)
		copy.id, copy.name, copy.origin, copy.path = `{id}-copy`, `{e.name} Copy`, "editable", `/data/custom-themes/{id}-copy.zeron-theme.json`
		copy.variants = {}
		for _, v in e.variants do
			local p: any = nil
			for _, x in model.list do
				if x.id == v then
					p = table.clone(x)
				end
			end
			p.id, p.entry = `{v}-copy`, copy.id
			table.insert(model.list, p)
			table.insert(copy.variants, p.id)
		end
		table.insert(model.entries, copy)
		publish()
		return settled(true, copy.id)
	elseif action == "review" then
		return settled(true, { name = e.name, variants = { {
			id = e.variants[1], name = e.name, kind = "dark",
			mappings = { { role = "background", key = "editor.background", value = "#14181f" }, { role = "accent", key = "focusBorder", value = "#7aa2f7" } },
			fallbacks = { "zeron.composer.edge" }, dropped = { "statusBar.debuggingBackground" }, warnings = {},
			adjustments = { { role = "text.tertiary", original = "#3b4252", resolved = "#7c8594", reason = "contrast" } }, validation = {},
		} } })
	end
	return settled(true, nil)
end
ui.command { id = "old_host", title = "Run on a host from before the theme library", run = function()
	state.host.analyze, state.host.add, state.host.entry = real_host.analyze, real_host.add, real_host.entry
end }
ui.command { id = "break_source", title = "Break the linked source", run = function()
	source_broken = true
end }

for _, mode in { "system", "light", "dark" } do
	ui.command { id = `scheme_{mode}`, title = `Scheme: {mode}`, run = function()
		appearance.scheme:set(mode)
	end }
end
ui.command { id = "use_ledger", title = "Run the Ledger template", run = function()
	local now = table.clone(templates.now:peek()) :: any
	now.active = "ledger"
	templates.now:set(now)
end }
ui.command { id = "drop_harbor", title = "Drop Harbor Night on the page", run = function()
	page.dropped({ { path = "/Downloads/Harbor Night.json", name = "Harbor Night.json", size = 1012, type = "application/json" } })
end }
-- What an agent's theme tools do while the window runs: the files change,
-- the engine says so (WatchThemes), and the host publishes the list again.
-- Here: Harbor Night's window turned plum, and a theme made, "Ink".
ui.command { id = "agent_edits", title = "Change the themes from outside", run = function()
	for _, p in model.list do
		if p.id == data.HARBOR.id then
			p.background = "#2a1430"
		end
	end
	local ink = table.clone(data.HARBOR)
	ink.id, ink.name, ink.entry, ink.background = "custom-ink-dark", "Ink", "custom-ink", "#101820"
	table.insert(model.list, ink)
	table.insert(model.entries, { id = "custom-ink", name = "Ink", source = "imported", status = { ok = true }, variants = { "custom-ink-dark" } })
	publish()
end }
ui.command { id = "themes", title = "Open Themes", run = function()
	route.settings("themes")
end }
ui.command { id = "appearance", title = "Open Appearance", run = function()
	route.settings("appearance")
end }
-- What the window wears now: the theme showing, its window colour, and the
-- palette each scheme wears with Ledger's own theme as the template's.
ui.command { id = "read", title = "Read what's worn", run = function()
	local slots = palettes.slots(data.LEDGER_FILE :: any)
	worn:set(`{ui.theme.current:peek()} {ui.theme.get("zeron.window")}`)
	slots_said:set(`{if slots.light then slots.light.id else "zeron"} {if slots.dark then slots.dark.id or "ledger" else "zeron"}`)
end }

publish()

return ui.scroll {
	w_full = true,
	h_full = true,
	bg = "$zeron.window",
	ui.column {
		w_full = true,
		p = "$space.6",
		items = "center",
		ui.presence(function(): string
			return if route.current:get().page == "themes" then "themes" else "appearance"
		end, function(which: string): Node
			return if which == "themes" then page.View() else settings_appearance.view()
		end, { w_full = true, items = "center" }),
	},
}
