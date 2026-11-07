ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local AgentFan = require("@ds/agent_fan")
local Roster = require("@app/screens/chat/roster")

local input = ui.state("fan_input", {} :: any)
local output = ui.state("fan_output", {} :: any)
local tiles = ui.computed(function(): { any }
	return Roster.tiles(Roster.runs(input:get()).listed)
end)
ui.effect(function()
	local runs = Roster.runs(input:get())
	local keys: { string } = {}
	for _, t in Roster.tiles(runs.listed) do
		table.insert(keys, t.key .. (if t.asking then "!" else ""))
	end
	local rows: { string } = {}
	for _, r in runs.listed do
		table.insert(rows, `{r.id}:{r.state}:{r.depth}`)
	end
	output:set {
		tiles = table.concat(keys, ","),
		shown = #AgentFan.shown(Roster.tiles(runs.listed)),
		rows = table.concat(rows, ","),
		words = table.concat({ Roster.elapsed(45), Roster.elapsed(65), Roster.elapsed(3720), Roster.compact(950), Roster.compact(29216), Roster.compact(1250000) }, "|"),
	}
end)

return ui.column {
	gap = 8,
	AgentFan { id = "fan", tiles = function()
		return tiles:get()
	end, size = 20 },
	AgentFan { id = "fan16", tiles = function()
		return tiles:get()
	end, size = 16 },
}
