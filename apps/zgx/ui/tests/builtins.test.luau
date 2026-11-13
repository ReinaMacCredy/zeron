-- Zeron's own features (screens/plugins/builtins): Review is the one built-in left on Settings → Plugins; the rest are switches on the Settings page that owns each, and off still takes the feature's doors away at once, with no reload; a launch after keeps a switch off. On ui/shell_fixture.luau (Settings, the rail, a new chat's composer) and ui/panel_fixture.luau (the browser).

local MOVED = { "Usage", "Home widgets", "Keep awake", "Appshots", "Predictions", "Browser annotate", "Device", "Voice" }

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1800)
	app:set_state("shell_fixture.plugins", true)
	app:frames(30)
	return app
end

local function settings(app: any, page: string)
	if not app:has("#settings-nav") then
		app:command("shell.settings")
		app:frames(10)
	end
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

test("Settings → Plugins lists Review alone of Zeron's own, Built-in and on; the rest aren't there", function()
	local app = open()
	settings(app, "Plugins")
	local tree = app:tree("#settings-page")
	expect(switch_of(app, "Review")).to_contain("checked")
	expect(app:has('#settings-page >> role:button "Review is part of Zeron"')).to_be(true)
	local _, tags = string.gsub(tree, '"Built%-in"', "")
	expect(tags).to_be(1)
	for _, name in MOVED do
		expect(tree).never.to_contain(`switch "{name}"`)
	end
end)

test("each feature's switch on its own page takes its doors away at once, and on brings them back; the page stays listed", function()
	local app = open()
	expect(app:has("#rail-usage")).to_be(true)
	expect(app:has('role:button "Hold to dictate"')).to_be(true)

	-- Usage: the rail's ring.
	settings(app, "Usage")
	flip(app, "Usage on the rail")
	expect(switch_of(app, "Usage on the rail")).to_contain("unchecked")
	expect(app:has("#rail-usage")).to_be(false)
	expect(nav_has(app, "Usage")).to_be(true)
	flip(app, "Usage on the rail")
	expect(app:has("#rail-usage")).to_be(true)

	-- Voice: the composer's mic.
	settings(app, "Voice")
	flip(app, "Voice")
	expect(app:has('role:button "Hold to dictate"')).to_be(false)
	expect(nav_has(app, "Voice")).to_be(true)
	flip(app, "Voice")
	expect(app:has('role:button "Hold to dictate"')).to_be(true)

	-- Keep awake: the engine's own key, through the host, on General.
	settings(app, "General")
	expect(calls(app)).to_contain("keep_awake.get")
	flip(app, "Keep this Mac awake while agents work")
	expect(calls(app)).to_contain("keep_awake.set")
	expect(switch_of(app, "Keep this Mac awake while agents work")).to_contain("unchecked")

	-- Home widgets (no door yet; Appearance's New chat, under Advanced), Browser annotate and Device: their keys.
	for _, it in { { "Appearance", "Home widgets", "builtin_home_widgets" }, { "Browsing", "Annotate", "builtin_browser_annotate" }, { "Device hub", "Device panel", "builtin_device" } } do
		settings(app, it[1])
		if it[1] == "Appearance" then
			app:set_state("settings_appearance_advanced", true)
			app:frames(20)
			app:scroll("#settings-page", 3000)
			app:frames(20)
		end
		flip(app, it[2])
		expect(switch_of(app, it[2])).to_contain("unchecked")
		expect(app:state(it[3])).to_be(false)
		expect(nav_has(app, it[1])).to_be(true)
	end

	-- Appshots: no built-in of its own any more (its Enable switch is the page's), so always listed.
	expect(nav_has(app, "Appshots")).to_be(true)
end)

test("a switch turned off stays off at the next launch, its door with it", function()
	local app = open()
	settings(app, "Usage")
	flip(app, "Usage on the rail")
	expect(app:has("#rail-usage")).to_be(false)
	local again = open()
	expect(again:has("#rail-usage")).to_be(false)
	settings(again, "Usage")
	expect(switch_of(again, "Usage on the rail")).to_contain("unchecked")
	expect(nav_has(again, "Usage")).to_be(true)
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
