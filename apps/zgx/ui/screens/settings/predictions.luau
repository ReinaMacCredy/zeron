-- General → Predictions: gpui's settings/predictions.rs on the engine's `GetPredictionSettings` / `SetPredictionSettings`.

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local chat = require("@app/screens/chat/store")
local Builtins = require("@app/screens/plugins/builtins")

local M = {}

local settings = ui.state("settings_predictions", nil :: any)
local unread = ui.state("settings_predictions_unread", nil :: string?)
local refused = ui.state("settings_predictions_refused", nil :: string?)
-- Agent or Custom API: chosen here first, saved once a custom API is entered.
local custom_mode = ui.state("settings_predictions_custom", false)
local base_url = ui.state("settings_predictions_base_url", "")
local api_model = ui.state("settings_predictions_model", "")

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

-- Read afresh: the mode and the fields follow it; a save's answer leaves them as typed.
local function adopt(v: any, fresh: boolean?)
	settings:set(v)
	if fresh then
		custom_mode:set(v.custom ~= nil)
		base_url:set(if v.custom then v.custom.baseUrl or "" else "")
		api_model:set(if v.custom then v.custom.model or "" else "")
	end
end

function M.load()
	engine("GetPredictionSettings"):after(function(v: any)
		if v then
			adopt(v, true)
		end
		unread:set(nil)
	end, function(err: any)
		unread:set(says(err))
	end)
end

-- The engine's settings with `patch` (`clear` names fields to drop): shown now, back if refused.
local function save(patch: { [string]: any }, clear: { string }?, key: string?)
	local before = settings:peek()
	if not before then
		return
	end
	local next = table.clone(before)
	next.hasApiKey = nil
	for k, v in patch do
		next[k] = v
	end
	for _, k in clear or {} do
		next[k] = nil
	end
	local shown = table.clone(next)
	shown.hasApiKey = before.hasApiKey
	if key ~= nil then
		next.apiKey = key
	end
	settings:set(shown)
	engine("SetPredictionSettings", next):after(function(v: any)
		if v ~= nil then
			adopt(v)
		end
		refused:set(nil)
	end, function(err: any)
		settings:set(before)
		refused:set(says(err))
	end)
end

local function engine_on(): boolean
	local s = settings:get()
	return s ~= nil and s.enabled == true
end

-- One switch for both: the engine's setting and the built-in that has the host watch chats for one.
local function enabled(): boolean
	return engine_on() and Builtins.on("predictions")
end

local AUTO = "auto"
local function model_value(): string
	local s = settings:get()
	if s and s.harness and s.model then
		return `{s.harness}/{s.model}`
	end
	return AUTO
end
local function model_select(): Node
	local function options(): { any }
		local out: { any } = { { value = AUTO, label = ui.t("settings.predictions.session_agent"):get() } }
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
			name = ui.t("settings.predictions.agent_model"),
			w = 200,
			options = options,
			on_change = function(v: string)
				if v == AUTO then
					save({}, { "harness", "model" })
					return
				end
				local harness, model = string.match(v, "^([^/]+)/(.+)$")
				save({ harness = harness, model = model })
			end,
		}
	end)()
end

local function switch(): Node
	return ui.component(function(): Node
		local on = ui.use_state(enabled())
		ui.effect(function()
			local v = enabled()
			if v ~= on:peek() then
				on:set(v)
			end
		end)
		return ds.Switch {
			value = on,
			label = ui.t("settings.predictions.show"),
			on_change = function(v: boolean)
				if settings:peek() == nil then
					on:set(false)
					return
				end
				Builtins.set("predictions", v)
				if v ~= engine_on() then
					save({ enabled = v })
				end
			end,
		}
	end)()
end

local function source(): Node
	return ui.component(function(): Node
		local chosen = ui.use_state(if custom_mode:peek() then "custom" else "agent")
		ui.effect(function()
			local v = if custom_mode:get() then "custom" else "agent"
			if v ~= chosen:peek() then
				chosen:set(v)
			end
		end)
		ui.effect(function()
			local v = chosen:get()
			if (v == "custom") == custom_mode:peek() then
				return
			end
			custom_mode:set(v == "custom")
			refused:set(nil)
			local s = settings:peek()
			if v == "agent" and s and s.custom ~= nil then
				save({}, { "custom" })
			end
		end)
		return ds.Segmented {
			value = chosen,
			label = ui.t("settings.predictions.predict_with"),
			options = {
				{ value = "agent", label = ui.t("settings.predictions.agent") },
				{ value = "custom", label = ui.t("settings.predictions.custom") },
			} :: any,
		}
	end)()
end

local function save_custom()
	local url = string.match(base_url:peek(), "^%s*(.-)%s*$") or ""
	local model = string.match(api_model:peek(), "^%s*(.-)%s*$") or ""
	local s = settings:peek()
	local was = s and s.custom
	if was and was.baseUrl == url and was.model == model then
		return
	end
	save({ custom = { baseUrl = url, model = model } })
end

local function paste_key()
	local clip = ui.clipboard.read()
	local key = if type(clip) == "string" then string.match(clip, "^%s*(.-)%s*$") or "" else ""
	if key == "" then
		refused:set(ui.t("settings.lab.key.copy_first"):get())
		return
	end
	save({}, nil, key)
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

function M.section(): Node
	return ds.Settings.Section {
		title = ui.t("settings.predictions.title"),
		ds.Settings.Group {
			row("predictions", {
				label = ui.t("settings.predictions.show"),
				description = function(): string
					return unread:get() or ui.t("settings.predictions.show_description"):get()
				end,
				control = switch(),
			}),
			ui.show(enabled, ui.column {
				w_full = true,
				ds.Settings.Rule(),
				row("predict_with", {
					label = ui.t("settings.predictions.predict_with"),
					description = ui.t("settings.predictions.predict_with_description"),
					control = source(),
				}),
				ds.Settings.Rule(),
				ui.show(function(): boolean
					return not custom_mode:get()
				end, row("prediction_model", {
					label = ui.t("settings.predictions.agent_model"),
					description = function(): string
						return ui.t(if model_value() == AUTO then "settings.predictions.follows_session" else "settings.predictions.one_model"):get()
					end,
					control = ui.row {
						gap = "$space.2",
						items = "center",
						ui.show(function(): boolean
							return model_value() ~= AUTO
						end, ds.Button { id = "predictions-reset", tone = "ghost", size = "sm", on_click = function()
							save({}, { "harness", "model" })
						end, ui.t("settings.predictions.reset") }),
						model_select(),
					},
				}), ds.Settings.Row {
					label = ui.t("settings.predictions.custom_api"),
					description = function(): string
						local s = settings:get()
						return ui.t(if s and s.hasApiKey then "settings.lab.key.saved" else "settings.predictions.no_key"):get()
					end,
					stacked = true,
					control = ui.column {
						w_full = true,
						gap = "$space.2",
						ui.row {
							w_full = true,
							gap = "$space.2",
							ui.div { id = "predictions-base-url", grow = 1, ds.TextField { value = base_url, name = ui.t("settings.predictions.base_url"), placeholder = "https://api.openai.com/v1", w_full = true, on_commit = save_custom } },
							ui.div { id = "predictions-model", ds.TextField { value = api_model, name = ui.t("settings.predictions.model"), placeholder = ui.t("settings.predictions.model_placeholder"), w = 200, on_commit = save_custom } },
						},
						ui.row {
							gap = "$space.2",
							ds.Button { id = "predictions-save", tone = "secondary", size = "sm", on_click = save_custom, ui.t("settings.predictions.save") },
							ds.Button { id = "predictions-paste-key", tone = "secondary", size = "sm", on_click = paste_key, ui.t("settings.lab.key.paste") },
							ui.show(function(): boolean
								local s = settings:get()
								return s ~= nil and s.hasApiKey == true
							end, ds.Button { id = "predictions-remove-key", tone = "ghost", size = "sm", on_click = function()
								save({}, nil, "")
							end, ui.t("settings.lab.key.remove") }),
						},
					},
				}),
			}),
			ui.show(function(): boolean
				return refused:get() ~= nil
			end, ui.text {
				function(): string
					return refused:get() or ""
				end,
				id = "predictions-refused",
				role = "alert",
				wrap = true,
				px = "$space.4",
				py = "$space.2",
				text_size = "$font.size.sm",
				color = "$danger",
			}),
		},
	}
end

return M
