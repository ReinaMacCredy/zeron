-- Settings → Tools: gpui's settings/tools.rs on the engine's tool settings (Compaction's states) and zeron_proto's groups (`settings.tool_groups`).

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local compaction = require("@app/screens/settings/compaction")
local route = require("@app/screens/shell/route")

local M = {}

M.ID = "tools"
M.TITLE = "Tools"

search.index(M.ID, ui.t("settings.tools.title"), ui.t("settings.tools.terms"), {
	{ key = "tools_all", title = ui.t("settings.tools.all_on"), terms = ui.t("settings.tools.all_terms") },
	{ key = "tools_groups", title = ui.t("settings.tools.groups"), terms = ui.t("settings.tools.groups_terms") },
})

local tools = compaction.tools
local groups = ui.state("settings_tools_groups", nil :: { any }?)
local open_groups = ui.state("settings_tools_open", {} :: { [string]: boolean })
local saving = ui.state("settings_tools_saving", 0)
local failed = ui.state("settings_tools_failed", nil :: string?)
-- (Their own switches: Settings → Computer use, and Device hub's agent access.)
local LINKED: { [string]: string } = { computer = "computer", devices = "devices" }
local ICONS: { [string]: string } = {
	threads = "chat-round-line",
	workspace = "terminal",
	delegation = "bot",
	pages = "global",
	devices = "smartphone",
	computer = "computer-cursor",
	plugins = "widget",
}

local function engine_settings(): any?
	local t = tools:get()
	return t and t.settings
end

local function load()
	compaction.engine("GetToolSettings"):after(function(v: any)
		tools:set(v)
		failed:set(nil)
	end, function(err: any)
		failed:set(compaction.says(err))
	end)
end

-- The groups' counts follow what the engine says is off.
ui.effect(function()
	local t = tools:get()
	if not t then
		return
	end
	local ok, p = pcall(host.call, "settings.tool_groups", { disabled = t.disabled or {} })
	if ok and p then
		p:after(function(v: any)
			if type(v) == "table" then
				groups:set(v)
			end
		end, function() end)
	end
end)

local function has(list: { any }?, item: any): boolean
	for _, v in list or {} do
		if v == item then
			return true
		end
	end
	return false
end
local function without(list: { any }?, item: any): { any }
	local out = {}
	for _, v in list or {} do
		if v ~= item then
			table.insert(out, v)
		end
	end
	return out
end

local function save(change: (any) -> ())
	local t = tools:peek()
	if not t or not t.settings then
		return
	end
	local next = table.clone(t.settings)
	change(next)
	local shown = table.clone(t)
	shown.settings = next
	tools:set(shown)
	saving:set(saving:peek() + 1)
	compaction.engine("SetToolSettings", next):after(function(v: any)
		saving:set(saving:peek() - 1)
		if v ~= nil then
			tools:set(v)
		end
		failed:set(nil)
	end, function(err: any)
		saving:set(saving:peek() - 1)
		tools:set(t)
		failed:set(ui.t("settings.tools.couldnt_save", { error = compaction.says(err) }):get())
	end)
end

-- A group as last counted (rows are built once, by group).
local function current(g: any): any
	for _, now in groups:get() or {} do
		if now.group == g.group then
			return now
		end
	end
	return g
end

local function group_on(g: any): boolean
	if LINKED[g.group] then
		return current(g).count.live_tools > 0
	end
	local s = engine_settings()
	return s ~= nil and not has(s.disabledGroups, g.group)
end
local function tool_on(name: string): boolean
	local s = engine_settings()
	return s ~= nil and not has(s.disabledTools, name)
end
local function live(name: string): boolean
	local t = tools:get()
	return t ~= nil and not has(t.disabled, name)
end

-- gpui's group state: OFF, PARTIAL or ALL LIVE.
local function state_of(g: any): string
	local c = current(g).count
	if not group_on(g) or c.live_tools == 0 then
		return "off"
	elseif c.live_tools == c.tools and c.live_actions == c.actions then
		return "on"
	end
	return "partial"
end
local function counted(g: any): string
	local c, partial = current(g).count, state_of(g) == "partial"
	local function part(n: number, of: number): string
		return if partial and n < of then `{n}/{of}` else tostring(of)
	end
	local out = ui.t(if c.tools == 1 then "settings.tools.count_one" else "settings.tools.count", { n = part(c.live_tools, c.tools) }):get()
	if c.actions > 0 then
		out ..= ui.t("settings.tools.actions", { n = part(c.live_actions, c.actions) }):get()
	end
	return out
end

local function totals(): (number, number, boolean, boolean, boolean)
	local live_n, total, all, any_on, any_off = 0, 0, true, false, false
	for _, g in groups:get() or {} do
		live_n += g.count.live_tools
		total += g.count.tools
		local st = state_of(g)
		all = all and st == "on"
		any_on = any_on or (st ~= "off" and not LINKED[g.group])
		any_off = any_off or (st ~= "on" and not LINKED[g.group])
	end
	return live_n, total, all, any_on, any_off
end

local function turn_all(on: boolean)
	save(function(next: any)
		local disabled_groups = {}
		for _, g in groups:peek() or {} do
			if not LINKED[g.group] and not on then
				table.insert(disabled_groups, g.group)
			end
		end
		next.disabledGroups = disabled_groups
		if on then
			next.disabledTools = {}
		end
	end)
end

local function headline(): (string, string, string)
	local live_n, total, all = totals()
	if saving:get() > 0 then
		return ui.t("settings.tools.saving"):get(), ui.t("settings.tools.saving_body"):get(), "saving"
	elseif all then
		return ui.t("settings.tools.all_live"):get(), ui.t("settings.tools.all_live_body", { n = total, groups = #(groups:get() or {}) }):get(), "on"
	elseif live_n > 0 then
		return ui.t("settings.tools.some", { n = live_n, total = total }):get(), ui.t("settings.tools.some_body"):get(), "partial"
	end
	return ui.t("settings.tools.none"):get(), ui.t("settings.tools.none_body"):get(), "off"
end

local TAGS: { [string]: { string } } = {
	saving = { "settings.tools.tag.saving", "neutral" },
	on = { "settings.tools.tag.on", "success" },
	partial = { "settings.tools.tag.partial", "warning" },
	off = { "settings.tools.tag.off", "neutral" },
}

local function status_card(): Node
	return ds.Settings.Group {
		ui.column {
			id = "tools-status",
			w_full = true,
			gap = "$space.2",
			p = "$space.4",
			items = "center",
			ui.row {
				gap = "$space.2",
				items = "center",
				ui.text {
					function(): string
						return (select(1, headline()))
					end,
					id = "tools-headline",
					weight = "$font.weight.medium",
					color = "$text.primary",
				},
				ui.presence(function(): string
					return (select(3, headline()))
				end, function(which: string): Node
					return ds.Badge { ui.t(TAGS[which][1]), tone = TAGS[which][2] :: any }
				end),
			},
			ui.text {
				function(): string
					return (select(2, headline()))
				end,
				text_size = "$font.size.sm",
				color = "$text.secondary",
				wrap = true,
				align = "center",
			},
			search.anchor("tools_all", ui.row {
				gap = "$space.2",
				ds.Button { id = "tools-all-on", tone = "primary", size = "sm", disabled = function(): boolean
					return saving:get() > 0 or not (select(5, totals()))
				end, on_click = function()
					turn_all(true)
				end, ui.t("settings.tools.all_on") },
				ds.Button { id = "tools-all-off", tone = "secondary", size = "sm", disabled = function(): boolean
					return saving:get() > 0 or not (select(4, totals()))
				end, on_click = function()
					turn_all(false)
				end, ui.t("settings.tools.all_off") },
			}),
		},
	}
end

-- A switch on the engine's: it follows what the engine says; a flip goes to it unless locked.
local function switch(label: string, read: () -> boolean, write: (on: boolean) -> (), locked: (() -> boolean)?): Node
	return ui.component(function(): Node
		local on = ui.use_state(read())
		ui.effect(function()
			local v = read()
			if v ~= on:peek() then
				on:set(v)
			end
		end)
		return ui.div {
			disabled = function(): boolean
				return locked ~= nil and locked()
			end,
			ds.Switch {
				value = on,
				label = label,
				on_change = function(v: boolean)
					if (locked and locked()) or v == read() then
						on:set(read())
						return
					end
					write(v)
				end,
			},
		}
	end)()
end

local function tool_row(g: any, tool: any): Node
	local function editable(): boolean
		return group_on(g) and not LINKED[g.group] and (tool.needs == nil or live(tool.needs))
	end
	return ui.column {
		w_full = true,
		ds.Settings.Rule(),
		ds.Settings.Row {
			label = tool.name,
			description = if tool.summary ~= "" then tool.summary else nil,
			detail = function(): string?
				if tool.needs and group_on(g) and not live(tool.needs) then
					return ui.t("settings.tools.needs", { tool = tool.needs }):get()
				end
				return nil
			end,
			disabled = function(): boolean
				return not editable()
			end,
			control = switch(tool.name, function(): boolean
				return tool_on(tool.name)
			end, function(v: boolean)
				save(function(next: any)
					next.disabledTools = without(next.disabledTools, tool.name)
					if not v then
						table.insert(next.disabledTools, tool.name)
					end
				end)
			end, function(): boolean
				return not editable()
			end),
		},
	}
end

local function group_row(g: any): Node
	local function is_open(): boolean
		return open_groups:get()[g.group] == true
	end
	local linked = LINKED[g.group]
	return ui.column {
		id = `tools-group-{g.group}`,
		w_full = true,
		ds.Settings.Row {
			icon = (ICONS[g.group] or "widget") :: any,
			label = g.title,
			description = g.summary,
			detail = if linked then ui.t(`settings.tools.linked.{g.group}`) else nil,
			detail_action = if linked == "computer"
				then ds.Button { id = "tools-link-computer", tone = "ghost", size = "sm", on_click = function()
					route.settings("computer")
				end, ui.t("settings.tools.open") }
				else nil,
			control = ui.row {
				gap = "$space.2",
				items = "center",
				ui.text {
					function(): string
						return counted(g)
					end,
					font = "$font.mono",
					text_size = "$font.size.xs",
					color = "$text.tertiary",
				},
				switch(g.title, function(): boolean
					return group_on(g)
				end, function(v: boolean)
					save(function(next: any)
						next.disabledGroups = without(next.disabledGroups, g.group)
						if not v then
							table.insert(next.disabledGroups, g.group)
						end
					end)
				end, function(): boolean
					return linked ~= nil or saving:get() > 0
				end),
				ds.IconButton {
					id = `tools-open-{g.group}`,
					icon = function(): any
						return if is_open() then "alt-arrow-down" else "alt-arrow-right"
					end,
					size = "sm",
					tone = "quiet",
					label = ui.t("settings.tools.show_tools", { group = g.title }),
					on_click = function()
						open_groups:set_key(g.group, if is_open() then nil else true)
					end,
				},
			},
		},
		ui.show(is_open, ui.column {
			w_full = true,
			ui.show(function(): boolean
				return not group_on(g)
			end, ui.column {
				w_full = true,
				ds.Settings.Rule(),
				ui.text { ui.t("settings.tools.group_off"), id = `tools-off-{g.group}`, px = "$space.4", py = "$space.2", text_size = "$font.size.sm", color = "$text.secondary", wrap = true },
			}),
			ui.each(g.tools, function(tool: any): Node
				return tool_row(g, tool)
			end, { key = function(tool: any): string
				return tool.name
			end }),
		}),
	}
end

function M.view(): Node
	visits.once("tools", load)
	return ds.Settings.Page {
		title = ui.t("settings.tools.title"),
		description = ui.t("settings.tools.description"),
		ui.show(function(): boolean
			return failed:get() ~= nil
		end, ds.Callout {
			tone = "warning",
			title = ui.t("settings.tools.unread"),
			body = function(): string
				return failed:get() or ""
			end,
			action = {
				label = ui.t("common.retry") :: any,
				on_click = function()
					visits.again("tools", load)
				end,
			},
		}),
		ui.show(function(): boolean
			return groups:get() == nil and failed:get() == nil
		end, ui.text { ui.t("settings.compaction.loading"), color = "$text.secondary", text_size = "$font.size.sm" }),
		ui.show(function(): boolean
			return groups:get() ~= nil
		end, ui.column {
			w_full = true,
			gap = "$space.6",
			status_card(),
			ds.Settings.Section {
				title = ui.t("settings.tools.groups"),
				search.anchor("tools_groups", ds.Settings.Group {
					ui.each(ui.computed(function(): { any }
						return groups:get() or {}
					end), function(g: any, i: number): Node
						return ui.column {
							w_full = true,
							if i > 1 then ds.Settings.Rule() else nil,
							group_row(g),
						}
					end, { key = function(g: any): string
						return g.group
					end }),
				}),
			},
		}),
	}
end

return M
