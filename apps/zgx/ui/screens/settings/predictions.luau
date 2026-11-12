-- General → Predictions: gpui's settings/predictions.rs on the engine's `GetPredictionSettings` / `SetPredictionSettings`.

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")

local M = {}

local settings = ui.state("settings_predictions", nil :: any)
local unread = ui.state("settings_predictions_unread", nil :: string?)
local refused = ui.state("settings_predictions_refused", nil :: string?)

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

function M.load()
	engine("GetPredictionSettings"):after(function(v: any)
		if v then
			settings:set(v)
		end
		unread:set(nil)
	end, function(err: any)
		unread:set(says(err))
	end)
end

local function enabled(): boolean
	local s = settings:get()
	return s ~= nil and s.enabled == true
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
				local before = settings:peek()
				if before == nil then
					on:set(false)
					return
				end
				settings:set({ enabled = v })
				engine("SetPredictionSettings", { enabled = v }):after(function(saved: any)
					if saved ~= nil then
						settings:set(saved)
					end
					refused:set(nil)
				end, function(err: any)
					settings:set(before)
					refused:set(says(err))
				end)
			end,
		}
	end)()
end

function M.section(): Node
	return ds.Settings.Section {
		title = ui.t("settings.predictions.title"),
		ds.Settings.Group {
			search.anchor("predictions", ds.Settings.Row {
				label = ui.t("settings.predictions.show"),
				description = function(): string
					return unread:get() or ui.t("settings.predictions.show_description"):get()
				end,
				control = switch(),
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
