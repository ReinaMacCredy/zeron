--!strict
-- Settings → Themes: every theme Zeron can wear, laid out as Plugins and
-- Templates are (ds.Settings.Catalog): the same search, tabs, card of rows
-- and empty states. A theme is only colours, so it's worn with whichever
-- template runs, and it needs no allowing: it's data (src/themes.rs).
--
-- Its tabs: all of them; Zeron's built in; those templates bring (a
-- template's own colours, worn where you haven't chosen a theme); and
-- yours, imported from VS Code (a theme's .json, or an extension's folder,
-- by Import theme or dropped on the page).
--
-- Each row is its theme's as the list changes: its swatch (ds/theme_swatch,
-- a half for each scheme it has), its name, which schemes it has, where it
-- came from, and what it's worn for now ("In use", or "Light" or "Dark"
-- while following the system with another theme for the other); Use (worn
-- for each scheme it has) or nothing once it's worn for all it can be; and a
-- ⋯ menu: Use only for light or for dark, for one with both; a template's,
-- Show in Templates; yours, Rename and Remove. What's at a row's end swaps
-- only when its use does.

local ds = require("@ds")
local Menu = require("@ds/menu")
local control = require("@ds/control")
local DropOverlay = require("@ds/drop_overlay")
local ThemeSwatch = require("@ds/theme_swatch")
local state = require("@app/screens/themes/state")
local templates = require("@app/screens/templates/state")
local route = require("@app/screens/shell/route")

local M = {}

local tab = ui.state("themes_tab", "all")
local query = ui.state("themes_query", "")
local renaming = ui.state("themes_renaming", nil :: string?)
local renaming_open = ui.state("themes_renaming_open", false)
local new_name = ui.state("themes_new_name", "")
local removing = ui.state("themes_removing", nil :: string?)
local removing_open = ui.state("themes_removing_open", false)
-- Files carried over the page from outside.
local carrying = ui.state("themes_carrying", false)

local TABS = { builtin = "builtin", templates = "template", yours = "imported" }

local function template_name(id: string?): string
	for _, t in templates.list:get() do
		if t.id == id then
			return t.name
		end
	end
	return id or ""
end

-- Which schemes it has, or what's wrong with an imported one (a linked
-- one keeps the colours it last read cleanly: gpui's last known good).
local function about(e: state.Entry): string
	if e.status and e.status.ok == false then
		local message = e.status.message or ui.t("themes.page.needs_fixing"):get()
		return if state.linked(e) then ui.t("themes.page.last_good", { message = message }):get() else message
	end
	if e.id == "zeron" then
		return ui.t("themes.page.about.default"):get()
	end
	local light, dark = state.has(e, "light") ~= nil, state.has(e, "dark") ~= nil
	local key = if light and dark then "both" elseif light then "light" else "dark"
	return ui.t(`themes.page.about.{key}`):get()
end

-- Where it came from, on its own line.
local function byline(e: state.Entry): string
	if e.source == "template" then
		return ui.t("themes.page.byline.template", { name = template_name(e.template) }):get()
	elseif e.source == "imported" then
		if e.origin and e.origin ~= "imported" and e.path then
			return ui.t(`themes.page.byline.{e.origin}`, { path = e.path }):get()
		end
		return ui.t("themes.page.byline.imported"):get()
	end
	return ui.t("themes.page.byline.builtin"):get()
end

local function matches(e: state.Entry, q: string): boolean
	if q == "" then
		return true
	end
	return string.find(string.lower(`{e.name} {byline(e)}`), q, 1, true) ~= nil
end

-- The themes the tab holds that the search finds.
local found = ui.computed(function(): { state.Entry }
	local q = string.lower(query:get())
	local source = TABS[tab:get()]
	local out: { state.Entry } = {}
	for _, e in state.entries:get() do
		if (source == nil or e.source == source) and matches(e, q) then
			table.insert(out, e)
		end
	end
	return out
end)

local function count(which: string): () -> number
	return function(): number
		local source, n = TABS[which], 0
		for _, e in state.entries:get() do
			if source == nil or e.source == source then
				n += 1
			end
		end
		return n
	end
end

local function imported(done: any)
	local count_ = if done and done.variants then #done.variants else 0
	if count_ > 0 then
		tab:set("yours")
		ds.toast.show {
			title = ui.t("themes.page.imported_toast", { count = count_ }):get(),
			body = ui.t("themes.page.imported_body"):get(),
			tone = "success",
		}
	end
end

-- `path` brought in, said once it's in (or its error over the card).
local function import(path: string)
	local p: any = state.import(path)
	if p ~= nil then
		p:after(imported, function(_: any) end)
	end
end

-- gpui's Add a theme: a source (typed, or Browse…), kept as a copy or
-- linked to it, analyzed to show the themes it holds, those picked brought
-- in. A file dropped on the page skips it: a copy of all it holds.
local adding = ui.state("themes_add_open", false)
local add_path = ui.state("themes_add_path", "")
local add_link = ui.state("themes_add_link", false)
local found_now = ui.state("themes_add_found", nil :: any)
local picked = ui.state("themes_add_picked", {} :: { [string]: boolean })
local add_error = ui.state("themes_add_error", nil :: string?)
local analyzing = ui.state("themes_add_analyzing", false)

local function open_add()
	add_path:set("")
	found_now:set(nil)
	picked:set({})
	add_error:set(nil)
	adding:set(true)
end

-- A new source forgets what the last one held.
ui.effect(function()
	local path = add_path:get()
	local f = found_now:peek()
	if f and f.path ~= path then
		found_now:set(nil)
		picked:set({})
	end
end)

local function analyze()
	local path = string.match(add_path:peek(), "^%s*(.-)%s*$") or ""
	if path == "" then
		add_error:set(ui.t("themes.page.add.no_source"):get())
		return
	end
	add_error:set(nil)
	analyzing:set(true)
	state.analyze(path):after(function(f: any)
		analyzing:set(false)
		f.path = add_path:peek()
		local all: { [string]: boolean } = {}
		for _, v in f.variants or {} do
			all[v.id] = true
		end
		picked:set(all)
		found_now:set(f)
	end, function(err: any)
		analyzing:set(false)
		add_error:set(state.said(err))
	end)
end

local function chosen_ids(): { string }
	local out: { string } = {}
	local f = found_now:peek()
	for _, v in (f and f.variants) or {} do
		if picked:peek()[v.id] then
			table.insert(out, v.id)
		end
	end
	return out
end

local function add_selected()
	local ids = chosen_ids()
	if #ids == 0 then
		return
	end
	local path = string.match(add_path:peek(), "^%s*(.-)%s*$") or ""
	state.add(path, ids, add_link:peek()):after(function(done: any)
		adding:set(false)
		imported(done)
	end, function(err: any)
		add_error:set(state.said(err))
	end)
end

local function browse()
	ui.task(function()
		local files = ui.open_files({
			title = ui.t("themes.page.choose_source"):get(),
			filters = { { ui.t("themes.page.vscode_theme"):get(), { "json", "jsonc" } } },
		}):await()
		local file = files and files[1]
		if file then
			add_path:set(file.path)
			analyze()
		end
	end)
end

local pick = open_add

local function add_dialog(): Node
	local function variant_row(v: any): Node
		return ds.Checkbox {
			value = function(): boolean
				return picked:get()[v.id] == true
			end,
			on_change = function(on: boolean)
				local next = table.clone(picked:peek())
				next[v.id] = on or nil
				picked:set(next)
			end,
			label = v.name,
			description = ui.t(`themes.page.add.kind_{v.kind}`),
		}
	end
	return ds.Dialog {
		open = adding,
		title = ui.t("themes.page.add.title"),
		body = ui.t("themes.page.add.body"),
		size = "md",
		confirm = {
			label = function(): string
				return if found_now:get() then ui.t("themes.page.add.import_selected"):get() else ui.t("themes.page.add.analyze"):get()
			end,
			keep_open = true,
			busy = function(): boolean
				return analyzing:get() or state.busy:get()
			end,
			disabled = function(): boolean
				local f = found_now:get()
				if f then
					return next(picked:get()) == nil
				end
				return (string.match(add_path:get(), "^%s*(.-)%s*$") or "") == ""
			end,
			on_click = function()
				if found_now:peek() then
					add_selected()
				else
					analyze()
				end
			end,
		},
		ui.column {
			id = "themes-add",
			w_full = true,
			gap = "$space.3",
			ui.row {
				w_full = true,
				gap = "$space.2",
				items = "end",
				ui.div { grow = 1, min_w = 0, ds.TextField { value = add_path, label = ui.t("themes.page.add.source"), placeholder = ui.t("themes.page.add.source_hint"), w_full = true } },
				ds.Button { tone = "secondary", on_click = browse, ui.t("themes.page.add.browse") },
			},
			ui.column {
				w_full = true,
				gap = "$space.1",
				ui.text { ui.t("themes.page.add.keep"), text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.secondary" },
				ds.Segmented {
					value = add_link,
					label = ui.t("themes.page.add.keep"),
					options = {
						{ value = false, label = ui.t("themes.page.add.copy") },
						{ value = true, label = ui.t("themes.page.add.link") },
					} :: any,
				},
				ui.text {
					function(): string
						return if add_link:get() then ui.t("themes.page.add.link_hint"):get() else ui.t("themes.page.add.copy_hint"):get()
					end,
					text_size = "$font.size.sm",
					color = "$text.tertiary",
				},
			},
			ui.show(function(): boolean
				return found_now:get() ~= nil
			end, ui.column {
				id = "themes-add-found",
				w_full = true,
				gap = "$space.1",
				ui.text {
					function(): string
						local f = found_now:get()
						return ui.t("themes.page.add.detected", { count = if f then #(f.variants or {}) else 0 }):get()
					end,
					text_size = "$font.size.sm",
					weight = "$font.weight.medium",
					color = "$text.secondary",
				},
				ui.each(ui.computed(function(): { any }
					local f = found_now:get()
					return (f and f.variants) or {}
				end), function(v: any): Node
					return variant_row(v)
				end, { key = function(v: any): string
					return v.id
				end }),
				ui.each(ui.computed(function(): { string }
					local f = found_now:get()
					return (f and f.skipped) or {}
				end), function(line: string): Node
					return ui.text { ui.t("themes.page.add.skipped_one", { line = line }), text_size = "$font.size.sm", color = "$zeron.tone.warning.ink" }
				end),
			}),
			ui.show(function(): boolean
				return add_error:get() ~= nil
			end, ui.text {
				id = "themes-add-error",
				function(): string
					return add_error:get() or ""
				end,
				text_size = "$font.size.sm",
				color = "$zeron.tone.danger.ink",
			}),
		},
	}
end

-- gpui's mapping review: what each Zeron colour took from the source, and
-- what fell back, was dropped or adjusted.
local reviewing = ui.state("themes_review_open", false)
local review_data = ui.state("themes_review", nil :: any)
local function open_review(id: string)
	review_data:set(nil)
	state.review(id):after(function(r: any)
		review_data:set(r)
		reviewing:set(true)
	end, function(err: any)
		ds.toast.show { title = ui.t("themes.page.couldnt_review"):get(), body = state.said(err), tone = "warning" }
	end)
end

local function review_dialog(): Node
	local function lines(title: string, list: { string }): Node?
		if #list == 0 then
			return nil
		end
		local out: { any } = { w_full = true, gap = "$zeron.space.hair", ui.text { ui.t(`themes.page.review.{title}`, { count = #list }), text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.secondary" } }
		for _, line in list do
			table.insert(out, ui.text { line, text_size = "$font.size.xs", font = "$font.mono", color = "$text.tertiary" })
		end
		return ui.column(out)
	end
	local function variant(v: any): Node
		local mapped: { string } = {}
		for _, m in v.mappings or {} do
			table.insert(mapped, `{m.role} ← {m.key} {m.value}`)
		end
		local adjusted: { string } = {}
		for _, a in v.adjustments or {} do
			table.insert(adjusted, `{a.role}: {a.original} → {a.resolved} ({a.reason})`)
		end
		local parts: { any } = {
			class = "theme-review-variant",
			w_full = true,
			gap = "$space.2",
			ui.text { `{v.name} · {ui.t(`themes.page.add.kind_{v.kind}`):get()}`, weight = "$font.weight.medium", color = "$text.primary" },
		}
		for _, part in {
			lines("mapped", mapped),
			lines("fallbacks", v.fallbacks or {}),
			lines("dropped", v.dropped or {}),
			lines("adjusted", adjusted),
			lines("warnings", v.warnings or {}),
			lines("checks", v.validation or {}),
		} do
			table.insert(parts, part)
		end
		return ui.column(parts)
	end
	return ds.Dialog {
		open = reviewing,
		title = function(): string
			local r = review_data:get()
			return ui.t("themes.page.review.title", { name = if r then r.name else "" }):get()
		end,
		size = "lg",
		cancel = ui.t("themes.page.review.done"),
		ui.scroll {
			id = "themes-review",
			w_full = true,
			max_h = 420,
			ui.column {
				w_full = true,
				gap = "$space.4",
				ui.each(ui.computed(function(): { any }
					local r = review_data:get()
					return (r and r.variants) or {}
				end), function(v: any): Node
					return variant(v)
				end, { key = function(v: any): string
					return v.id
				end }),
			},
		},
	}
end

-- A library entry's action, said if it fails.
local function act(run: () -> any, failed: string)
	local p: any = run()
	if p ~= nil then
		p:after(function() end, function(err: any)
			ds.toast.show { title = ui.t(failed):get(), body = state.said(err), tone = "warning" }
		end)
	end
end

-- Files dropped on the page: each brought in (a folder as an extension).
function M.dropped(files: { any })
	carrying:set(false)
	for _, f in files do
		if type(f) == "table" and type(f.path) == "string" then
			import(f.path)
		end
	end
end

-- What it's worn for now, after its name.
local function badge(e: state.Entry): string?
	local fills = state.fills(e)
	local showing = state.showing()
	if #showing == 1 then
		return if fills[showing[1]] then ui.t("themes.page.in_use"):get() else nil
	elseif fills.light and fills.dark then
		return ui.t("themes.page.in_use"):get()
	elseif fills.light then
		return ui.t("themes.page.for_light"):get()
	elseif fills.dark then
		return ui.t("themes.page.for_dark"):get()
	end
	return nil
end

-- Use and its empty slot take the same width, the wider as laid out, so a
-- row's end never shifts as its use changes (as Templates').
local slot = ui.state("themes_use_w", 0)
local function slot_w(): number
	return slot:get()
end
local function grow(size: any)
	if type(size) == "table" and type(size.w) == "number" and size.w > slot:peek() then
		slot:set(size.w)
	end
end

-- Use, or its slot kept empty; then its ⋯ menu (or the menu's room, for a
-- theme with nothing more to do, so the column stays one down the list).
local function controls(e: state.Entry, now: State<any>): Node
	local id = e.id
	local menu_open = ui.use_state(false)
	local function pick_(run: () -> ()): () -> ()
		return function()
			menu_open:set(false)
			run()
		end
	end
	local groups: { Node } = {}
	local light, dark = state.has(e, "light"), state.has(e, "dark")
	if light and dark then
		table.insert(groups, Menu.Group {
			Menu.Row {
				icon = "sun",
				label = ui.t("themes.page.use_light"),
				role = "menuitem",
				on_click = pick_(function()
					state.use_for("light", light)
				end),
			},
			Menu.Row {
				icon = "moon",
				label = ui.t("themes.page.use_dark"),
				role = "menuitem",
				on_click = pick_(function()
					state.use_for("dark", dark)
				end),
			},
		})
	end
	if e.source == "template" then
		table.insert(groups, Menu.Group {
			Menu.Row {
				icon = "gallery",
				label = ui.t("themes.page.show_template"),
				role = "menuitem",
				on_click = pick_(function()
					route.settings("templates")
				end),
			},
		})
	elseif e.source == "imported" then
		local library: { Node } = {}
		if state.linked(e) then
			table.insert(library, Menu.Row {
				icon = "refresh",
				label = ui.t("themes.page.reload"),
				role = "menuitem",
				on_click = pick_(function()
					act(function()
						return state.reload(id)
					end, "themes.page.couldnt_reload")
				end),
			})
		end
		if e.path then
			table.insert(library, Menu.Row {
				icon = "folder",
				label = ui.t("themes.page.reveal"),
				role = "menuitem",
				on_click = pick_(function()
					act(function()
						return state.reveal(id)
					end, "themes.page.couldnt_reveal")
				end),
			})
		end
		table.insert(library, Menu.Row {
			icon = "eye",
			label = ui.t("themes.page.review_mapping"),
			role = "menuitem",
			on_click = pick_(function()
				open_review(id)
			end),
		})
		table.insert(library, Menu.Row {
			icon = "copy",
			label = ui.t("themes.page.duplicate"),
			role = "menuitem",
			on_click = pick_(function()
				act(function()
					return state.duplicate(id)
				end, "themes.page.couldnt_duplicate")
			end),
		})
		if state.linked(e) then
			table.insert(library, Menu.Row {
				icon = "lock-keyhole-unlocked",
				label = ui.t("themes.page.unlink"),
				role = "menuitem",
				on_click = pick_(function()
					act(function()
						return state.unlink(id)
					end, "themes.page.couldnt_unlink")
				end),
			})
		end
		table.insert(groups, Menu.Group(library))
		table.insert(groups, Menu.Group {
			Menu.Row {
				icon = "pen",
				label = ui.t("themes.page.rename"),
				role = "menuitem",
				on_click = pick_(function()
					renaming:set(id)
					new_name:set(now:peek().name)
					renaming_open:set(true)
				end),
			},
		})
		table.insert(groups, Menu.Group {
			Menu.Row {
				icon = "trash-bin-minimalistic",
				label = ui.t("themes.page.remove"),
				role = "menuitem",
				danger = true,
				on_click = pick_(function()
					removing:set(id)
					removing_open:set(true)
				end),
			},
		})
	end
	local more: Node
	if #groups == 0 then
		more = ui.div { w = "$zeron.icon_button.sm", h = "$zeron.icon_button.sm", shrink = 0 }
	else
		local anchor = ds.IconButton {
			icon = "more-horizontal",
			label = function(): string
				return ui.t("themes.page.more_for", { name = now:get().name }):get()
			end,
			size = "sm",
			tone = "quiet",
			on_click = function()
				menu_open:set(not menu_open:peek())
			end,
		}
		local panel: { [any]: any } = { open = menu_open, anchor = anchor, placement = { side = "bottom", align = "end", offset = 4 }, w = 220 }
		for i, g in groups do
			if i > 1 then
				table.insert(panel, Menu.Rule {})
			end
			table.insert(panel, g)
		end
		-- (The menu draws its anchor, the ⋯, in place.)
		more = ui.menu(Menu.panel(panel))
	end
	return ui.row {
		gap = "$space.2",
		items = "center",
		ui.switch(function(): string
			return if state.full(now:get()) then "using" else "use"
		end, {
			using = ui.div { min_w = slot_w, h = control.H.sm },
			use = ds.Button {
				tone = "secondary",
				size = "sm",
				min_w = slot_w,
				on_resize = grow,
				on_click = function()
					state.use(now:peek())
				end,
				ui.t("themes.page.use"),
			},
		}),
		more,
	}
end

local function row(e: state.Entry, _i: number, now: State<any>): Node
	return ui.column {
		id = `theme-{e.id}`,
		class = "theme-row",
		w_full = true,
		ds.Settings.Row {
			label = function(): string
				return now:get().name
			end,
			description = function(): string
				return about(now:get())
			end,
			byline = function(): string
				return byline(now:get())
			end,
			lead = ThemeSwatch.Swatch {
				themes = function(): { string }
					return state.previews:get()[e.id] or {}
				end,
				class = "theme-swatch",
			},
			badge = ui.show(function(): boolean
				return badge(now:get()) ~= nil
			end, ds.Badge {
				function(): string
					return badge(now:get()) or ""
				end,
				tone = "accent",
				class = "theme-in-use",
			}),
			truncate = true,
			control = controls(e, now),
		},
	}
end

-- What a tab with nothing in it says (All and Built in always hold Zeron's).
local function none(which: string): Node
	if which == "templates" then
		return ds.EmptyState {
			icon = "gallery",
			title = ui.t("themes.page.none_templates"),
			body = ui.t("themes.page.none_templates_body"),
			size = "sm",
		}
	end
	return ds.EmptyState {
		icon = "palette",
		title = ui.t("themes.page.none_yours"),
		body = ui.t("themes.page.none_yours_body"),
		size = "sm",
		action = { label = ui.t("themes.page.import"), on_click = pick },
	}
end

-- An import's error, over the card until the next one.
local function notice(): Node
	return ds.Callout {
		tone = "danger",
		title = ui.t("themes.page.import_failed") :: any,
		body = function(): string
			return state.error:get() or ""
		end,
	}
end

local function dialogs(): { Node }
	return {
		add_dialog(),
		review_dialog(),
		ds.Dialog {
			open = renaming_open,
			title = ui.t("themes.page.rename_title"),
			size = "sm",
			confirm = {
				label = ui.t("themes.page.rename") :: any,
				on_click = function()
					local id, name = renaming:peek(), string.match(new_name:peek(), "^%s*(.-)%s*$") or ""
					renaming:set(nil)
					if id and name ~= "" then
						local p: any = state.rename(id, name)
						if p ~= nil then
							p:after(function() end, function(err: any)
								ds.toast.show { title = ui.t("themes.page.couldnt_rename"):get(), body = state.said(err), tone = "warning" }
							end)
						end
					end
				end,
			},
			ds.TextField { value = new_name, label = ui.t("themes.page.name"), w_full = true, select_on_focus = true },
		},
		ds.Dialog {
			open = removing_open,
			title = ui.t("themes.page.remove_title"),
			body = ui.t("themes.page.remove_body"),
			tone = "danger",
			confirm = {
				label = ui.t("themes.page.remove") :: any,
				on_click = function()
					local id = removing:peek()
					removing:set(nil)
					if id then
						state.remove(id)
					end
				end,
			},
		},
	}
end

function M.View(): Node
	-- The swatches' themes, kept while the page is open.
	state.swatches()
	return ui.column {
		id = "themes-page",
		w_full = true,
		items = "center",
		relative = true,
		-- A theme file or an extension's folder dropped anywhere on it.
		on_file_hover = function(inside: boolean)
			carrying:set(inside)
		end,
		on_file_drop = M.dropped,
		ds.Settings.Catalog {
			title = ui.t("themes.page.title"),
			description = ui.t("themes.page.description"),
			search = { value = query, placeholder = ui.t("themes.page.search") },
			tabs = {
				value = tab,
				label = ui.t("themes.page.tabs"),
				options = {
					{ value = "all", label = ui.t("themes.page.all"), count = count("all") },
					{ value = "builtin", label = ui.t("themes.page.builtin"), count = count("builtin") },
					{ value = "templates", label = ui.t("themes.page.from_templates"), count = count("templates") },
					{ value = "yours", label = ui.t("themes.page.yours"), count = count("yours") },
				},
			},
			import = {
				label = ui.t("themes.page.import"),
				on_click = pick,
			},
			notice = notice,
			notice_when = function(): boolean
				return state.error:get() ~= nil
			end,
			status = function(): string
				return if state.ready() then "ready" else "loading"
			end,
			found = found,
			row = row,
			-- (A row's menu is built with it: an entry linked or unlinked is a row anew.)
			key = function(e: state.Entry): string
				return `{e.id}:{e.origin or ""}`
			end,
			none = none,
			hint = { id = "themes-hint", text = ui.t("themes.page.hint") },
			overlays = dialogs(),
		},
		DropOverlay { shown = function(): boolean
			return carrying:get()
		end, id = "themes-drop", label = ui.t("themes.page.drop") :: any },
	}
end

return M
