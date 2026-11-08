-- General → Predictions (screens/settings/predictions) on ui/shell_fixture.luau with ui/settings_stub.luau's engine.

local PAGE = "#settings-page"

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
local function has(app: any, words: string): boolean
	return app:tree(PAGE):find(words, 1, true) ~= nil
end
local function again(app: any)
	app:click('#settings-nav >> role:tab "Chat"')
	app:frames(20)
	app:click('#settings-nav >> role:tab "General"')
	app:frames(30)
end

test("Predictions: off at first; on, it offers Agent or Custom API, each change to the engine and read back", function()
	local app = open()
	expect(calls(app)).to_contain("settings.engine:GetPredictionSettings")
	expect(has(app, "Show predictions")).to_be(true)
	expect(has(app, "Predict with")).to_be(false)
	app:click(`{PAGE} >> role:switch "Show predictions"`)
	app:frames(20)
	expect(count(app, "SetPredictionSettings")).to_be(1)
	expect(has(app, "Predict with")).to_be(true)
	expect(has(app, "Each chat's own agent, on its cheapest model.")).to_be(true)
	app:click(`{PAGE} >> "Custom API"[1]`)
	app:frames(20)
	expect(has(app, "No key. Local servers like Ollama need none.")).to_be(true)
	app:type("#predictions-base-url >> role:textbox", "http://localhost:11434/v1")
	app:type("#predictions-model >> role:textbox", "qwen3")
	app:click("#predictions-save")
	app:frames(20)
	local saved = count(app, "SetPredictionSettings")
	expect(saved >= 2).to_be(true)
	again(app)
	expect(count(app, "GetPredictionSettings")).to_be(2)
	expect(has(app, "Predict with")).to_be(true)
	expect(app:tree("#predictions-base-url")).to_contain("localhost:11434")
	-- Back to the agent drops the custom API.
	app:click(`{PAGE} >> "Agent"[1]`)
	app:frames(20)
	expect(count(app, "SetPredictionSettings")).to_be(saved + 1)
	again(app)
	expect(has(app, "Each chat's own agent, on its cheapest model.")).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("Predictions: a key pasted is saved and removed; a refusal says why", function()
	local app = open()
	app:click(`{PAGE} >> role:switch "Show predictions"`)
	app:frames(20)
	app:click(`{PAGE} >> "Custom API"[1]`)
	app:frames(20)
	app:copy("sk-local")
	app:click("#predictions-paste-key")
	app:frames(20)
	expect(calls(app)).to_contain("prediction_key:sk-local")
	expect(has(app, "Key saved in the Keychain.")).to_be(true)
	app:click("#predictions-remove-key")
	app:frames(20)
	expect(has(app, "No key. Local servers like Ollama need none.")).to_be(true)
	app:command("settings_stub.refuse_next")
	app:type("#predictions-model >> role:textbox", "llama")
	app:key("enter")
	app:frames(20)
	expect(app:text("#predictions-refused")).to_be("refused")
end)
