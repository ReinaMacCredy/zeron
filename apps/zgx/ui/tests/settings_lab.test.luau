-- Settings → Experimental (screens/settings/lab) on ui/shell_fixture.luau with ui/settings_stub.luau's engine.

local PAGE = "#settings-page"

local function open(mac: boolean?): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	if mac then
		app:set_state("platform", "macos")
	end
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	-- (The list scrolls past a short window, as gpui's: its foot scrolled to first.)
	app:scroll("#settings-nav", 2000)
	app:frames(5)
	app:click('#settings-nav >> "Experimental"')
	app:frames(30)
	return app
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end
local function count(app: any, what: string): number
	return select(2, string.gsub(calls(app), what, ""))
end
local function checked(app: any, label: string): boolean
	for line in app:tree(PAGE):gmatch("[^\n]+") do
		if line:find(`switch "{label}"`, 1, true) then
			return line:find("checked", 1, true) ~= nil and line:find("unchecked", 1, true) == nil
		end
	end
	error(`no switch {label}`)
end
local function has(app: any, words: string): boolean
	return app:tree(PAGE):find(words, 1, true) ~= nil
end
local function again(app: any)
	app:scroll("#settings-nav", -2000)
	app:frames(5)
	app:click('#settings-nav >> "General"')
	app:frames(20)
	app:scroll("#settings-nav", 2000)
	app:frames(5)
	app:click('#settings-nav >> "Experimental"')
	app:frames(30)
end

test("Experimental: gpui's Lab rows under Lab, with the engine's values", function()
	local app = open()
	expect(app:tree("#settings-nav")).to_contain('"Lab"')
	for _, w in { "Experimental", "Beta", "Compact ahead of time", "Compact new tasks", "Reminder", "Classifier", "Classifier model", "Agent access", "Agents use the full token" } do
		expect(`{w}: {has(app, w)}`).to_be(`{w}: true`)
	end
	expect(checked(app, "Compact ahead of time")).to_be(false)
	expect(checked(app, "Agents use the full token")).to_be(false)
	-- (Off a Mac there's no Keychain for a Jev key: no source to choose.)
	expect(has(app, "Jev · TypeSafe")).to_be(false)
	expect(calls(app)).to_contain("settings.engine:GetCompactionSettings")
	expect(calls(app)).to_contain("settings.engine:GetToolSettings")
	expect(app:error()).to_be(nil)
end)

test("Experimental: each change goes to the engine at once and is there when the page opens again", function()
	local app = open()
	app:click(`{PAGE} >> role:switch "Compact ahead of time"`)
	app:frames(20)
	expect(count(app, "SetCompactionSettings")).to_be(1)
	expect(checked(app, "Compact ahead of time")).to_be(true)
	app:click(`{PAGE} >> role:switch "Agents use the full token"`)
	app:frames(20)
	expect(count(app, "SetToolSettings")).to_be(1)
	app:click(`{PAGE} >> "Reminder"`)
	app:frames(20)
	expect(count(app, "SetCompactionSettings")).to_be(2)
	expect(has(app, "Classifier model")).to_be(false)
	expect(has(app, "Each message carries a reminder")).to_be(true)
	again(app)
	expect(count(app, "GetCompactionSettings")).to_be(2)
	expect(checked(app, "Compact ahead of time")).to_be(true)
	expect(checked(app, "Agents use the full token")).to_be(true)
	expect(has(app, "Classifier model")).to_be(false)
	-- Compaction's own page reads the same engine.
	app:click('#settings-nav >> "Compaction"')
	app:frames(30)
	expect(has(app, "Remind at")).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("Experimental on a Mac: Jev's source takes a key pasted, says it's saved, and removes it", function()
	local app = open(true)
	app:click(`{PAGE} >> "Jev · TypeSafe"`)
	app:frames(20)
	expect(has(app, "No key. Get one at console.typesafe.ai/keys")).to_be(true)
	expect(has(app, "Classifier model")).to_be(false)
	app:copy("")
	app:click("#lab-paste-key")
	app:frames(10)
	expect(app:text("#lab-refused")).to_be("Copy the API key first, then press Paste key.")
	app:copy("  sk-test-123 \n")
	app:click("#lab-paste-key")
	app:frames(20)
	expect(calls(app)).to_contain("jev_key:sk-test-123")
	expect(has(app, "Key saved in the Keychain.")).to_be(true)
	expect(app:has("#lab-refused")).to_be(false)
	app:click("#lab-remove-key")
	app:frames(20)
	expect(calls(app) .. ",").to_contain("jev_key:,")
	expect(app:has("#lab-remove-key")).to_be(false)
	app:click(`{PAGE} >> "Agent model"`)
	app:frames(20)
	expect(has(app, "Classifier model")).to_be(true)
	again(app)
	expect(has(app, "Classifier model")).to_be(true)
	expect(app:error()).to_be(nil)
end)
