-- Settings → Experimental (Lab): gpui's settings/compaction.rs `lab` half, on Compaction's states.

local ds = require("@ds")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local compaction = require("@app/screens/settings/compaction")
local chat = require("@app/screens/chat/store")

local M = {}

M.ID = "experimental"
M.TITLE = "Experimental"

search.index(M.ID, ui.t("settings.lab.title"), ui.t("settings.lab.terms"), {
	{ key = "ahead", title = ui.t("settings.lab.ahead.label"), section = ui.t("settings.lab.compaction_section"), terms = ui.t("settings.lab.ahead.terms") },
	{ key = "mode", title = ui.t("settings.lab.mode.label"), section = ui.t("settings.lab.compaction_section"), terms = ui.t("settings.lab.mode.terms") },
	{ key = "source", title = ui.t("settings.lab.source.label"), section = ui.t("settings.lab.compaction_section"), terms = ui.t("settings.lab.source.terms") },
	{ key = "model", title = ui.t("settings.lab.model.label"), section = ui.t("settings.lab.compaction_section"), terms = ui.t("settings.lab.model.terms") },
	{ key = "full_token", title = ui.t("settings.lab.full_token.label"), section = ui.t("settings.lab.access_section"), terms = ui.t("settings.lab.full_token.terms") },
})

local settings = compaction.settings
local tools = compaction.tools
local refused = compaction.refused

local function mac(): boolean
	local ok, platform = pcall(function(): any
		return ui.shared("platform", ""):get()
	end)
	return ok and platform == "macos"
end

local function classifier(): boolean
	local c = settings:get()
	return c == nil or c.mode ~= "threshold"
end

local function jev(): string?
	local c = settings:get()
	return if c and type(c.jev) == "string" then c.jev else nil
end

-- The tool settings changed by `change`: shown now, back if refused.
local function save_tools(change: (any) -> ())
	local t = tools:peek()
	if not t or not t.settings then
		return
	end
	local next = table.clone(t.settings)
	change(next)
	local shown = table.clone(t)
	shown.settings = next
	tools:set(shown)
	compaction.engine("SetToolSettings", next):after(function(v: any)
		if v ~= nil then
			tools:set(v)
		end
		refused:set(nil)
	end, function(err: any)
		tools:set(t)
		refused:set(compaction.says(err))
	end)
end

-- The classifier's source: an agent's model, or Jev on TypeSafe or OpenRouter.
local function source_choice(): Node
	return ui.component(function(): Node
		local chosen = ui.use_state(jev() or "agent")
		ui.effect(function()
			local v = jev() or "agent"
			if v ~= chosen:peek() then
				chosen:set(v)
			end
		end)
		ui.effect(function()
			local v = chosen:get()
			if settings:peek() and v ~= (jev() or "agent") then
				if v == "agent" then
					compaction.save({}, { "jev" })
				else
					compaction.save({ jev = v })
				end
			end
		end)
		return ds.Segmented {
			value = chosen,
			label = ui.t("settings.lab.source.label"),
			options = {
				{ value = "agent", label = ui.t("settings.lab.source.agent") },
				{ value = "typesafe", label = ui.t("settings.lab.source.typesafe") },
				{ value = "openrouter", label = ui.t("settings.lab.source.openrouter") },
			} :: any,
		}
	end)()
end

-- The classifier's model: the agent's own Sonnet-class pick, or one model.
local AUTO = "auto"
local function model_value(): string
	local c = settings:get()
	local m = c and c.classifier
	if m and m.harness and m.model then
		return `{m.harness}/{m.model}`
	end
	return AUTO
end
local function model_select(): Node
	local function options(): { any }
		local out: { any } = { { value = AUTO, label = ui.t("settings.lab.model.auto"):get() } }
		local catalog = chat.catalog:get()
		if catalog.state ~= "ready" then
			return out
		end
		local names: { [string]: string } = {}
		for _, h in catalog.harnesses do
			names[h.id] = h.label
		end
		for _, m in catalog.models do
			table.insert(out, { value = m.value, label = m.label, description = names[m.harness] })
		end
		return out
	end
	return ui.component(function(): Node
		local chosen = ui.use_state(model_value())
		ui.effect(function()
			local v = model_value()
			if v ~= chosen:peek() then
				chosen:set(v)
			end
		end)
		return ds.Select {
			value = chosen,
			name = ui.t("settings.lab.model.label"),
			w = 200,
			options = options,
			on_change = function(v: string)
				if v == AUTO then
					compaction.save({ classifier = {} })
					return
				end
				local harness, model = string.match(v, "^([^/]+)/(.+)$")
				compaction.save({ classifier = { harness = harness, model = model } })
			end,
		}
	end)()
end

-- The Jev key: saved in the Keychain by the engine, never read back.
local function key_status(): string
	local c = settings:get()
	if c and c.hasJevKey then
		return ui.t("settings.lab.key.saved"):get()
	end
	return ui.t(if jev() == "openrouter" then "settings.lab.key.none_openrouter" else "settings.lab.key.none_typesafe"):get()
end
local function save_key(key: string)
	compaction.save({ jevApiKey = key })
end
local function paste_key()
	local clip = ui.clipboard.read()
	local key = if type(clip) == "string" then string.match(clip, "^%s*(.-)%s*$") or "" else ""
	if key == "" then
		refused:set(ui.t("settings.lab.key.copy_first"):get())
		return
	end
	save_key(key)
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

function M.view(): Node
	visits.once("experimental", function()
		compaction.load()
		if chat.catalog:peek().state ~= "ready" then
			chat.load_catalog()
		end
	end)
	local function ready(): boolean
		return settings:get() ~= nil and tools:get() ~= nil
	end
	return ds.Settings.Page {
		title = ui.t("settings.lab.title"),
		description = ui.t("settings.lab.description"),
		trail = ui.div { id = "lab-beta", ds.Badge { ui.t("settings.lab.beta"), tone = "accent" } },
		ui.show(function(): boolean
			return compaction.unread:get() ~= nil
		end, ds.Callout {
			tone = "warning",
			title = ui.t("settings.lab.unread"),
			body = function(): string
				return compaction.unread:get() or ""
			end,
			action = {
				label = ui.t("common.retry") :: any,
				on_click = function()
					visits.again("experimental", compaction.load)
				end,
			},
		}),
		ui.show(function(): boolean
			return not ready() and compaction.unread:get() == nil
		end, ui.text { ui.t("settings.compaction.loading"), color = "$text.secondary", text_size = "$font.size.sm" }),
		ui.show(ready, ui.column {
			w_full = true,
			gap = "$space.6",
			ds.Settings.Section {
				title = ui.t("settings.lab.compaction_section"),
				ds.Settings.Group {
					row("ahead", {
						label = ui.t("settings.lab.ahead.label"),
						description = ui.t("settings.lab.ahead.description"),
						control = compaction.engine_switch(ui.t("settings.lab.ahead.label"), function(): boolean?
							local c = settings:get()
							return if c then c.compactAhead == true else nil
						end, function(on: boolean)
							compaction.save({ compactAhead = on })
						end),
					}),
					row("mode", {
						label = ui.t("settings.lab.mode.label"),
						description = function(): string
							return ui.t(if classifier() then "settings.lab.mode.classifier_description" else "settings.lab.mode.reminder_description"):get()
						end,
						control = compaction.engine_choice(ui.t("settings.lab.mode.label"), "mode", {
							{ value = "threshold" :: any, label = ui.t("settings.lab.mode.reminder") },
							{ value = "classifier" :: any, label = ui.t("settings.lab.mode.classifier") },
						}),
					}),
					ui.show(function(): boolean
						return classifier() and mac()
					end, ui.column {
						w_full = true,
						ds.Settings.Rule(),
						row("source", {
							label = ui.t("settings.lab.source.label"),
							description = ui.t("settings.lab.source.description"),
							stacked = true,
							control = source_choice(),
						}),
					}),
					ui.show(function(): boolean
						return classifier() and jev() == nil
					end, ui.column {
						w_full = true,
						ds.Settings.Rule(),
						row("model", {
							label = ui.t("settings.lab.model.label"),
							description = ui.t("settings.lab.model.description"),
							control = model_select(),
						}),
					}),
					ui.show(function(): boolean
						return classifier() and jev() ~= nil
					end, ui.column {
						w_full = true,
						ds.Settings.Rule(),
						ds.Settings.Row {
							label = ui.t("settings.lab.key.label"),
							description = key_status,
							control = ui.row {
								gap = "$space.2",
								items = "center",
								ds.Button { id = "lab-paste-key", tone = "secondary", size = "sm", on_click = paste_key, ui.t("settings.lab.key.paste") },
								ui.show(function(): boolean
									local c = settings:get()
									return c ~= nil and c.hasJevKey == true
								end, ds.Button { id = "lab-remove-key", tone = "ghost", size = "sm", on_click = function()
									save_key("")
								end, ui.t("settings.lab.key.remove") }),
							},
						},
					}),
				},
				ui.show(function(): boolean
					return refused:get() ~= nil
				end, ui.text {
					function(): string
						return refused:get() or ""
					end,
					id = "lab-refused",
					role = "alert",
					wrap = true,
					px = "$space.4",
					py = "$space.2",
					text_size = "$font.size.sm",
					color = "$danger",
				}),
			},
			ds.Settings.Section {
				title = ui.t("settings.lab.access_section"),
				ds.Settings.Group {
					row("full_token", {
						label = ui.t("settings.lab.full_token.label"),
						description = ui.t("settings.lab.full_token.description"),
						control = compaction.engine_switch(ui.t("settings.lab.full_token.label"), function(): boolean?
							local t = tools:get()
							return if t and t.settings then t.settings.agentsUseFullToken == true else nil
						end, function(on: boolean)
							save_tools(function(next: any)
								next.agentsUseFullToken = on
							end)
						end),
					}),
				},
			},
		}),
	}
end

return M
