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

test("Experimental: agents' access under Lab, with the engine's value, and nothing of compaction", function()
	local app = open()
	expect(app:tree("#settings-nav")).to_contain('"Lab"')
	for _, w in { "Experimental", "Beta", "Agent access", "Agents use the full token" } do
		expect(`{w}: {has(app, w)}`).to_be(`{w}: true`)
	end
	for _, w in { "Compact ahead of time", "Compact new tasks", "Classifier", "Jev" } do
		expect(`{w}: {has(app, w)}`).to_be(`{w}: false`)
	end
	expect(checked(app, "Agents use the full token")).to_be(false)
	expect(calls(app)).to_contain("settings.engine:GetToolSettings")
	expect(app:error()).to_be(nil)
end)

test("Experimental: a change goes to the engine at once and is there when the page opens again", function()
	local app = open(true)
	app:click(`{PAGE} >> role:switch "Agents use the full token"`)
	app:frames(20)
	expect(count(app, "SetToolSettings")).to_be(1)
	expect(count(app, "SetCompactionSettings")).to_be(0)
	again(app)
	expect(checked(app, "Agents use the full token")).to_be(true)
	expect(app:error()).to_be(nil)
end)
