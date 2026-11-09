-- Zeron's own plugins in Settings → Plugins (screens/plugins/builtins): the nine listed first, each Built-in and on in a fresh store; each switch takes its doors away at once and brings them back with no reload; a launch after keeps a switch off. On ui/shell_fixture.luau (Settings, the rail, a new chat's composer) and ui/panel_fixture.luau (the browser).

local NINE = { "Review", "Usage", "Home widgets", "Keep awake", "Appshots", "Predictions", "Browser annotate", "Device", "Voice" }

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1800)
	app:set_state("shell_fixture.plugins", true)
	app:frames(30)
	return app
end

local function settings(app: any, page: string)
	app:command("shell.settings")
	app:frames(10)
	app:click(`#settings-nav >> role:tab "{page}"`)
	app:frames(30)
end

local function switch_of(app: any, name: string): string?
	return app:tree("#settings-page"):match(`switch "{name}"[^\n]*`)
end

local function flip(app: any, name: string)
	app:click(`#settings-page >> role:switch "{name}"`)
	app:frames(20)
end

local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end

local function nav_has(app: any, page: string): boolean
	return app:has(`#settings-nav >> role:tab "{page}"`)
end

test("Settings → Plugins lists Zeron's nine first, in order, each Built-in and on, none removable", function()
	local app = open()
	settings(app, "Plugins")
	local tree = app:tree("#settings-page")
	local at = 0
	for _, name in NINE do
		local found = string.find(tree, `switch "{name}"`, 1, true)
		expect(found ~= nil).to_be(true)
		expect(found > at).to_be(true)
		at = found :: number
		expect(switch_of(app, name)).to_contain("checked")
		expect(app:has(`#settings-page >> role:button "{name} is part of Zeron"`)).to_be(true)
	end
	local _, tags = string.gsub(tree, '"Built%-in"', "")
	expect(tags).to_be(9)
	expect(tree).to_contain("The Usage page and the limit ring on the rail")
	-- (Keep awake as the engine's file has it.)
	expect(calls(app)).to_contain("keep_awake.get")
end)

test("each switch off takes its doors away at once, and on brings them back", function()
	local app = open()
	expect(app:has("#rail-usage")).to_be(true)
	-- (The mic, as gpui's, once Dictation is on and its model here: U28.)
	expect(app:has('role:button "Hold to dictate"')).to_be(false)
	app:command("voice_stub.dictation_on")
	app:frames(10)
	expect(app:has('role:button "Hold to dictate"')).to_be(true)
	settings(app, "Plugins")
	expect(nav_has(app, "Usage")).to_be(true)
	expect(nav_has(app, "Appshots")).to_be(true)
	expect(nav_has(app, "Voice")).to_be(true)
	expect(nav_has(app, "Device hub")).to_be(true)

	-- Usage: the rail's ring and its Settings page.
	flip(app, "Usage")
	expect(switch_of(app, "Usage")).to_contain("unchecked")
	expect(app:has("#rail-usage")).to_be(false)
	expect(nav_has(app, "Usage")).to_be(false)
	flip(app, "Usage")
	expect(app:has("#rail-usage")).to_be(true)
	expect(nav_has(app, "Usage")).to_be(true)

	-- Appshots: its Settings page.
	flip(app, "Appshots")
	expect(nav_has(app, "Appshots")).to_be(false)
	flip(app, "Appshots")
	expect(nav_has(app, "Appshots")).to_be(true)

	-- Voice: the composer's mic and Settings → Voice.
	flip(app, "Voice")
	expect(app:has('role:button "Hold to dictate"')).to_be(false)
	expect(nav_has(app, "Voice")).to_be(false)
	flip(app, "Voice")
	expect(app:has('role:button "Hold to dictate"')).to_be(true)
	expect(nav_has(app, "Voice")).to_be(true)

	-- Device: Settings → Device hub (the panel's Device row is tests/device_tab's).
	flip(app, "Device")
	expect(nav_has(app, "Device hub")).to_be(false)
	flip(app, "Device")
	expect(nav_has(app, "Device hub")).to_be(true)

	-- Keep awake: the engine's own key, through the host.
	flip(app, "Keep awake")
	expect(calls(app)).to_contain("keep_awake.set")
	expect(switch_of(app, "Keep awake")).to_contain("unchecked")

	-- Review, Home widgets and Device: their keys (Review's door is tests/review_plugin's, Home widgets has none yet, Device's above).
	for _, name in { "Review", "Home widgets", "Device" } do
		flip(app, name)
		expect(switch_of(app, name)).to_contain("unchecked")
	end
	expect(app:state("builtin_review")).to_be(false)
	expect(app:state("builtin_home_widgets")).to_be(false)
	expect(app:state("builtin_device")).to_be(false)

	-- Predictions: General's Predictions, and the search finds it no more.
	flip(app, "Predictions")
	app:click('#settings-nav >> role:tab "General"')
	app:frames(30)
	expect(app:tree("#settings-page")).never.to_contain("Show predictions")
	app:click('#settings-nav >> role:tab "Plugins"')
	app:frames(30)
	flip(app, "Predictions")
	app:click('#settings-nav >> role:tab "General"')
	app:frames(30)
	expect(app:tree("#settings-page")).to_contain("Show predictions")
end)

test("a switch turned off stays off at the next launch, its door with it", function()
	local app = open()
	settings(app, "Plugins")
	flip(app, "Usage")
	expect(app:has("#rail-usage")).to_be(false)
	local again = open()
	expect(again:has("#rail-usage")).to_be(false)
	settings(again, "Plugins")
	expect(switch_of(again, "Usage")).to_contain("unchecked")
	expect(switch_of(again, "Review")).to_contain("checked")
	expect(nav_has(again, "Usage")).to_be(false)
end)

test("Browser annotate off takes the browser bar's Annotate, its View menu item and its key away, and on brings them back", function()
	local app = mount("panel_fixture.luau")
	app:resize(900, 640)
	app:frames(10)
	expect(app:wait("#panel-empty", { timeout_ms = 15000 })).to_be(true)
	app:click("#panel-open-browser")
	expect(app:wait('role:combobox "Address"', { timeout_ms = 15000 })).to_be(true)
	app:type('role:combobox "Address"', "example.com")
	app:key("Enter")
	app:frames(5)
	local function started(): boolean
		return string.find(app:state("test_web_calls") or "", "run:start", 1, true) ~= nil
	end
	local function listed(): boolean
		for _, c in app:commands() do
			if c.id == "panel.annotate" then
				return c.menu == "View" and c.key == "cmd+shift+i"
			end
		end
		return false
	end
	expect(app:has('role:checkbox "Annotate"')).to_be(true)
	expect(listed()).to_be(true)
	app:set_state("builtin_browser_annotate", false)
	app:frames(10)
	expect(app:has('role:checkbox "Annotate"')).to_be(false)
	expect(listed()).to_be(false)
	app:command("panel.annotate")
	app:frames(5)
	expect(started()).to_be(false)
	app:set_state("builtin_browser_annotate", true)
	app:frames(10)
	expect(app:has('role:checkbox "Annotate"')).to_be(true)
	expect(listed()).to_be(true)
	app:command("panel.annotate")
	app:frames(5)
	expect(started()).to_be(true)
end)
