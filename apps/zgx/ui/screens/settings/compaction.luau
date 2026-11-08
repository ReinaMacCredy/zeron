-- Settings → Compaction: the shipping app's (crates/ui/src/settings/
-- compaction.rs, its Agents > Compaction page). The engine keeps these
-- (`GetCompactionSettings`, `SetCompactionSettings` through
-- `settings.engine`); "Let agents compact themselves" is the `compact`
-- tool's switch in its tool settings (`GetToolSettings`, `SetToolSettings`).
-- Asked once a visit; each change shown at once, back as it was with the
-- engine's words under the rows when it refuses. The rest of gpui's page
-- (compact ahead, the classifier's source and model) is its Lab's
-- (lab.luau, on these same states).

local host = require("@app/screens/settings/host")
local ds = require("@ds")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")

local M = {}

M.ID = "compaction"
M.TITLE = "Compaction"

local COMPACT_TOOL = "compact"

search.index(M.ID, ui.t("settings.compaction.title"), ui.t("settings.compaction.terms"), {
	{ key = "idle", title = ui.t("settings.compaction.idle.label"), section = ui.t("settings.compaction.idle_section"), terms = ui.t("settings.compaction.idle.terms") },
	{ key = "threshold", title = ui.t("settings.compaction.threshold.label"), section = ui.t("settings.compaction.idle_section"), terms = ui.t("settings.compaction.threshold.terms") },
	{ key = "agents", title = ui.t("settings.compaction.agents.label"), section = ui.t("settings.compaction.chat_section"), terms = ui.t("settings.compaction.agents.terms") },
	{ key = "tasks", title = ui.t("settings.compaction.tasks.label"), section = ui.t("settings.compaction.chat_section"), terms = ui.t("settings.compaction.tasks.terms") },
	{ key = "check", title = ui.t("settings.compaction.check.label"), section = ui.t("settings.compaction.chat_section"), terms = ui.t("settings.compaction.check.terms") },
})

-- The engine's, as last read or written (nil until read).
local settings = ui.state("settings_compaction", nil :: any)
local tools = ui.state("settings_compaction_tools", nil :: any)
-- Why it couldn't be read, or the last change refused.
local unread = ui.state("settings_compaction_unread", nil :: string?)
local refused = ui.state("settings_compaction_refused", nil :: string?)

local function engine(method: string, params: { [string]: any }?): any
	return host.call("settings.engine", { method = method, params = params or {} })
end

local function says(err: any): string
	local word = if type(err) == "table" then err.message else err
	if word == "outdated" then
		return ui.t("settings.engine.outdated"):get()
	elseif word == "offline" then
		return ui.t("settings.engine.offline"):get()
	end
	return if type(word) == "string" and word ~= "" then word else ui.t("common.something_went_wrong"):get()
end

local function load()
	engine("GetCompactionSettings"):after(function(v: any)
		settings:set(v)
		unread:set(nil)
	end, function(err: any)
		unread:set(says(err))
	end)
	engine("GetToolSettings"):after(function(v: any)
		tools:set(v)
	end, function(err: any)
		unread:set(says(err))
	end)
end

M.load = load
M.settings = settings
M.tools = tools
M.refused = refused
M.unread = unread
M.engine = engine
M.says = says

-- A change to the engine's: shown now, back as it was if refused (its
-- words under the rows, as gpui's error strip).
local function save(patch: { [string]: any }, clear: { string }?)
	local before = settings:peek()
	if not before then
		return
	end
	local next = table.clone(before)
	next.hasJevKey = nil
	for k, v in patch do
		next[k] = v
	end
	local shown = table.clone(before)
	for k, v in patch do
		shown[k] = v
	end
	for _, k in clear or {} do
		next[k] = nil
		shown[k] = nil
	end
	shown.jevApiKey = nil
	settings:set(shown)
	engine("SetCompactionSettings", next):after(function(v: any)
		if v ~= nil then
			settings:set(v)
		end
		refused:set(nil)
	end, function(err: any)
		settings:set(before)
		refused:set(says(err))
	end)
end

local function agents_on(): boolean?
	local t = tools:get()
	if not t then
		return nil
	end
	for _, name in t.disabled or {} do
		if name == COMPACT_TOOL then
			return false
		end
	end
	return true
end

local function save_tool(on: boolean)
	local t = tools:peek()
	if not t or not t.settings then
		return
	end
	local next = table.clone(t.settings)
	local list = {}
	for _, name in next.disabledTools or {} do
		if name ~= COMPACT_TOOL then
			table.insert(list, name)
		end
	end
	if not on then
		table.insert(list, COMPACT_TOOL)
	end
	next.disabledTools = list
	local before = t
	local shown = table.clone(t)
	local disabled = {}
	for _, name in t.disabled or {} do
		if name ~= COMPACT_TOOL then
			table.insert(disabled, name)
		end
	end
	if not on then
		table.insert(disabled, COMPACT_TOOL)
	end
	shown.disabled = disabled
	tools:set(shown)
	engine("SetToolSettings", next):after(function(v: any)
		if v ~= nil then
			tools:set(v)
		end
		refused:set(nil)
	end, function(err: any)
		tools:set(before)
		refused:set(says(err))
	end)
end

-- A switch on one of the engine's: it follows what the engine says, a
-- flip goes to it.
local function engine_switch(label: Message, read: () -> boolean?, write: (on: boolean) -> ()): Node
	return ui.component(function(): Node
		local on = ui.use_state(read() or false)
		ui.effect(function()
			local v = read()
			if v ~= nil and v ~= on:peek() then
				on:set(v)
			end
		end)
		return ds.Switch {
			value = on,
			label = label,
			on_change = function(v: boolean)
				if read() ~= v then
					write(v)
				end
			end,
		}
	end)()
end

-- One of a few sizes of the engine's, as a segmented control.
local function engine_choice(label: Message, field: string, options: { { value: number, label: any } }): Node
	return ui.component(function(): Node
		local chosen = ui.use_state((settings:peek() or {})[field])
		ui.effect(function()
			local c = settings:get()
			if c and c[field] ~= chosen:peek() then
				chosen:set(c[field])
			end
		end)
		ui.effect(function()
			local v = chosen:get()
			local c = settings:peek()
			if c and v ~= nil and v ~= c[field] then
				save({ [field] = v })
			end
		end)
		return ds.Segmented { value = chosen, label = label, options = options :: any }
	end)()
end

M.save = save
M.engine_switch = engine_switch
M.engine_choice = engine_choice

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

local function size(n: number): string
	return `{n // 1000}K`
end

function M.view(): Node
	visits.once("compaction", load)
	local function ready(): boolean
		return settings:get() ~= nil and tools:get() ~= nil
	end
	local function classifier(): boolean
		local c = settings:get()
		return c == nil or c.mode ~= "threshold"
	end
	local function refusal(): Node
		return ui.show(function(): boolean
			return refused:get() ~= nil
		end, ui.text {
			function(): string
				return refused:get() or ""
			end,
			id = "compaction-refused",
			role = "alert",
			wrap = true,
			px = "$space.4",
			py = "$space.2",
			text_size = "$font.size.sm",
			color = "$danger",
		})
	end
	return ds.Settings.Page {
		title = ui.t("settings.compaction.title"),
		ui.show(function(): boolean
			return unread:get() ~= nil
		end, ds.Callout {
			tone = "warning",
			title = ui.t("settings.compaction.unread"),
			body = function(): string
				return unread:get() or ""
			end,
			action = {
				label = ui.t("common.retry") :: any,
				on_click = function()
					visits.again("compaction", load)
				end,
			},
		}),
		ui.show(function(): boolean
			return not ready() and unread:get() == nil
		end, ui.text { ui.t("settings.compaction.loading"), color = "$text.secondary", text_size = "$font.size.sm" }),
		ui.show(ready, ui.column {
			w_full = true,
			gap = "$space.6",
			ds.Settings.Section {
				title = ui.t("settings.compaction.idle_section"),
				ds.Settings.Group {
					row("idle", {
						label = ui.t("settings.compaction.idle.label"),
						description = ui.t("settings.compaction.idle.description"),
						control = engine_switch(ui.t("settings.compaction.idle.label"), function(): boolean?
							local c = settings:get()
							return if c then c.enabled ~= false else nil
						end, function(on: boolean)
							save({ enabled = on })
						end),
					}),
					row("threshold", {
						label = ui.t("settings.compaction.threshold.label"),
						description = ui.t("settings.compaction.threshold.description"),
						control = engine_choice(ui.t("settings.compaction.threshold.label"), "minTokens", {
							{ value = 100000, label = size(100000) },
							{ value = 150000, label = size(150000) },
							{ value = 200000, label = size(200000) },
						}),
					}),
				},
			},
			ds.Settings.Section {
				title = ui.t("settings.compaction.chat_section"),
				ds.Settings.Group {
					row("agents", {
						label = ui.t("settings.compaction.agents.label"),
						description = ui.t("settings.compaction.agents.description"),
						control = engine_switch(ui.t("settings.compaction.agents.label"), agents_on, save_tool),
					}),
					row("tasks", {
						label = ui.t("settings.compaction.tasks.label"),
						description = ui.t("settings.compaction.tasks.description"),
						control = engine_switch(ui.t("settings.compaction.tasks.label"), function(): boolean?
							local c = settings:get()
							return if c then c.missions ~= false else nil
						end, function(on: boolean)
							save({ missions = on })
						end),
					}),
					row("check", {
						label = function(): string
							return ui.t(if classifier() then "settings.compaction.check.label" else "settings.compaction.remind.label"):get()
						end,
						description = function(): string
							return ui.t(if classifier() then "settings.compaction.check.description" else "settings.compaction.remind.description"):get()
						end,
						control = engine_choice(ui.t("settings.compaction.check.label"), "nudgeTokens", {
							{ value = 0, label = ui.t("settings.compaction.off") },
							{ value = 100000, label = size(100000) },
							{ value = 150000, label = size(150000) },
							{ value = 200000, label = size(200000) },
						}),
					}),
				},
				refusal(),
			},
		}),
	}
end

return M
