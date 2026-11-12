-- General → Predictions (screens/settings/predictions) on ui/shell_fixture.luau with ui/settings_stub.luau's engine.

local PAGE = "#settings-page"
local SWITCH = `{PAGE} >> role:switch "Show predictions"`

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1800)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> role:tab "General"')
	app:frames(30)
	return app
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end
local function count(app: any, what: string): number
	return select(2, string.gsub(calls(app), what, ""))
end

test("Predictions: one switch, off at first, saved to the engine and read back", function()
	local app = open()
	expect(calls(app)).to_contain("settings.engine:GetPredictionSettings")
	expect(app:tree(PAGE)).to_contain("Suggest your next message after Claude and Codex turns.")
	expect(app:tree(SWITCH)).to_contain("unchecked")
	app:click(SWITCH)
	app:frames(20)
	expect(count(app, "SetPredictionSettings")).to_be(1)
	app:click('#settings-nav >> role:tab "Chat"')
	app:frames(20)
	app:click('#settings-nav >> role:tab "General"')
	app:frames(30)
	expect(count(app, "GetPredictionSettings")).to_be(2)
	expect(app:tree(SWITCH)).never.to_contain("unchecked")
	expect(app:tree(PAGE)).never.to_contain("Predict with")
	expect(app:error()).to_be(nil)
end)

test("Predictions: a refusal puts the switch back and says why", function()
	local app = open()
	app:command("settings_stub.refuse_next")
	app:click(SWITCH)
	app:frames(20)
	expect(app:text("#predictions-refused")).to_be("refused")
	expect(app:tree(SWITCH)).to_contain("unchecked")
end)
