-- A model picker on models made up here, for
-- tests/model_picker_in_place.test.luau: two with reasoning (one with Fast
-- mode), one without, and one configured in place (Devin Fusion's shape:
-- its lead and sidekick are options, so it has a card). The model chosen
-- is `test_picker_model`, its options' picks `test_picker_options`. Here,
-- beside app.luau, as a mounted script loads modules from its own
-- directory down.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local ModelPicker = require("@ds/model_picker")
local EffortSlider = require("@ds/effort_slider")

local THREE: { EffortSlider.Level } = {
	{ value = "low", label = "Low" },
	{ value = "medium", label = "Medium" },
	{ value = "high", label = "High" },
}
local FIVE: { EffortSlider.Level } = {
	{ value = "minimal", label = "Minimal" },
	{ value = "low", label = "Low" },
	{ value = "medium", label = "Medium" },
	{ value = "high", label = "High" },
	{ value = "xhigh", label = "Extra high" },
}

local MODELS: { ModelPicker.Model } = {
	{ value = "sonnet", label = "Sonnet", harness = "claude", efforts = THREE, effort = "medium" },
	{
		value = "opus-1m",
		label = "Opus 1M",
		harness = "claude",
		efforts = THREE,
		effort = "high",
		options = {
			{ id = "contextWindow", label = "Context Window", default = "1m", choices = { { id = "200k", label = "200K" }, { id = "1m", label = "1M" } } },
			{ id = "autoCompact", label = "Auto-compact", default = "auto", choices = { { id = "auto", label = "Auto" }, { id = "300k", label = "300K" }, { id = "400k", label = "400K" }, { id = "600k", label = "600K" }, { id = "800k", label = "800K" } } },
		},
	},
	{ value = "opus", label = "Opus", harness = "claude", efforts = THREE, effort = "high", fast = true },
	{ value = "gpt", label = "GPT", harness = "codex", efforts = FIVE, effort = "medium" },
	{ value = "mini", label = "Mini", harness = "codex", description = "the anthropic route" },
	{ value = "adaptive", label = "Adaptive", harness = "devin" },
	{ value = "cursor-auto", label = "Auto", harness = "devin", options = { { id = "effort", label = "Effort", default = "medium", choices = { { id = "low", label = "Low" }, { id = "medium", label = "Medium" }, { id = "high", label = "High" } } } } },
	{
		value = "fusion",
		label = "Fusion",
		harness = "devin",
		efforts = THREE,
		effort = "high",
		fast = true,
		options = {
			{ id = "sidekick", label = "Sidekick", default = "swe-medium", choices = { { id = "swe-medium", label = "SWE-2 Medium" }, { id = "swe-high", label = "SWE-2 High" } } },
			{ id = "lead", label = "Lead", default = "fable", choices = { { id = "fable", label = "Fable 5.1" }, { id = "sol", label = "GPT-6 Sol" } } },
		},
	},
}

-- The least opacity the picker's panel has had since a test last set this,
-- sampled each frame (tests/model_picker_motion.test.luau: never see-through
-- as it opens, changes page or follows its trigger).
local alpha = ui.state("test_picker_alpha", { panel = 1 })
ui.every(16, function()
	local now = alpha:peek()
	local panel = tonumber(anime.utils.get(".dropdown-panel", "opacity")) or 1
	if panel < now.panel then
		alpha:set({ panel = panel })
	end
end)
-- Where the reasoning thumb is drawn (its offset along the groove), each
-- frame.
local thumb = ui.state("test_picker_thumb", -1)
ui.every(16, function()
	local x = tonumber(anime.utils.get(".effort-thumb", "x"))
	if x and x ~= thumb:peek() then
		thumb:set(x)
	end
end)
-- As a chat with its agent, a pick asked about first; what was asked; what
-- the chat holds of its context.
local committed = ui.state("test_picker_committed", false)
local asking = ui.state("test_picker_asking", false)
local taken = ui.state("test_picker_taken", nil :: string?)
local context = ui.state("test_picker_context", nil :: number?)
-- More of a catalog, for its states: a new model, an agent still listing and
-- one that failed (`test_picker_more`); how often Retry was asked.
local more = ui.state("test_picker_more", false)
local retried = ui.state("test_picker_retried", 0)
local HARNESSES = {
	{ id = "claude", label = "Claude", mark = "claude" },
	{ id = "codex", label = "Codex", mark = "openai" },
	{ id = "devin", label = "Devin", mark = "devin" },
}
local MORE_HARNESSES = table.clone(HARNESSES)
table.insert(MORE_HARNESSES, { id = "pi", label = "Pi", mark = "pi", state = "loading" })
table.insert(MORE_HARNESSES, { id = "fx", label = "Fx", mark = "pi", state = "error", error = "Not signed in to Fx" })
local MORE_MODELS = table.clone(MODELS)
table.insert(MORE_MODELS, { value = "sol", label = "Sol", harness = "codex", new = true })
table.insert(MORE_MODELS, { value = "codex-auto", label = "Auto", harness = "codex", description = "Codex picks" })
-- Agents past the compact strip's room (`test_picker_many`), each with a model.
local many = ui.state("test_picker_many", false)
local MANY_HARNESSES = table.clone(HARNESSES)
local MANY_MODELS = table.clone(MODELS)
for i = 1, 6 do
	table.insert(MANY_HARNESSES, { id = `agent{i}`, label = `Agent {i}`, mark = "pi" })
	for k = 1, (if i == 6 then 12 else 1) do
		table.insert(MANY_MODELS, { value = `agent{i}-model{k}`, label = `Model {i}.{k}`, harness = `agent{i}` })
	end
end
-- Room over the picker (its trigger moving, as a composer growing over a
-- chip moves it).
local above = ui.state("test_picker_above", 0)

return ui.column {
	w_full = true,
	h_full = true,
	p = "$space.4",
	ui.div {
		w_full = true,
		shrink = 0,
		h = function(): number
			return above:get()
		end,
	},
	ModelPicker {
		model = ui.state("test_picker_model", "sonnet" :: string?),
		effort = ui.state("test_picker_effort", "medium" :: string?),
		fast = ui.state("test_picker_fast", false),
		options = ui.state("test_picker_options", {} :: { [string]: string }),
		favorites = ui.state("test_picker_favorites", {} :: { [string]: boolean }),
		harnesses = function(): { any }
			return if many:get() then MANY_HARNESSES elseif more:get() then MORE_HARNESSES else HARNESSES
		end,
		models = function(): { ModelPicker.Model }
			return if many:get() then MANY_MODELS elseif more:get() then MORE_MODELS else MODELS
		end,
		on_retry = function()
			retried:set(retried:peek() + 1)
		end,
		committed = function(): boolean
			return committed:get()
		end,
		take = function(value: string): boolean
			taken:set(value)
			return asking:get()
		end,
		context_tokens = function(): number?
			return context:get()
		end,
		-- A model its catalog does not list, by the name its chat gave it.
		model_label = function(): string?
			return if (ui.state("test_picker_model", nil :: string?):get() or ""):find("/", 1, true) then "Unlisted" else nil
		end,
	},
}
