-- The app rail on the shell in ui/shell_fixture.luau: there whatever the
-- sidebar and Settings do, its entries where they lead, its ring from the
-- accounts the host says.

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	return app
end

local function rail_w(app: any): number?
	local line: string = app:tree("#app-rail"):match("[^\n]+") or ""
	return tonumber(line:match(" %-?[%d.]+,%-?[%d.]+ ([%d.]+)x"))
end

local PULLS = { { plugin = "github", id = "pulls", title = "Pull requests", icon = "pull-request" }, { plugin = "other", id = "page", title = "Other page" } }

local function accounts(windows: { any }): any
	return { [""] = { codex = { { id = "a", email = "a@b.c", plan = "Pro", active = true, windows = windows } } } }
end

test("the rail stays 46 wide with the sidebar collapsed and over Settings, and the fit counts it", function()
	local app = open()
	expect(rail_w(app)).to_be(46)
	expect(app:state("shell_viewport_w")).to_be(1200 - 46)
	app:command("shell.toggle_sidebar")
	app:frames(40)
	expect(rail_w(app)).to_be(46)
	app:command("shell.settings")
	app:frames(20)
	expect(app:has("#settings-nav")).to_be(true)
	expect(rail_w(app)).to_be(46)
end)

test("Settings' entry toggles, saying so", function()
	local app = open()
	expect(app:tree("#app-rail")).to_contain('#settings-trigger button "Settings"')
	app:click("#settings-trigger")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/settings/general")
	expect(app:tree("#app-rail")).to_contain('#settings-trigger button "Close settings"')
	app:click("#settings-trigger")
	app:frames(20)
	expect(app:has("#settings-nav")).to_be(false)
end)

test("with motion reduced (as a carried-over setting can be), a rail entry made then still shows", function()
	local app = open()
	app:set_state("ds/motion.luau::motion_reduce", true)
	app:frames(5)
	app:set_state("plugins.pages", PULLS)
	app:frames(10)
	expect(app:has("#open-pull-requests")).to_be(true)
	expect(app:tree("#app-rail")).to_contain('#settings-trigger button "Settings"')
end)

test("New chat leads home; Pull requests shows while its plugin's page does, and only the rail lists it", function()
	local app = open()
	expect(app:has("#open-pull-requests")).to_be(false)
	app:set_state("plugins.pages", PULLS)
	app:frames(10)
	expect(app:has("#open-pull-requests")).to_be(true)
	expect(app:has('#sidebar-actions >> "Pull requests"')).to_be(false)
	expect(app:has('#sidebar-actions >> "Other page"')).to_be(true)
	app:click("#open-pull-requests")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/plugin/github/pulls")
	app:click("#rail-new-chat")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/")
	app:set_state("plugins.pages", {})
	app:frames(10)
	expect(app:has("#open-pull-requests")).to_be(false)
end)

test("the ring says the tightest window, the limit with a badge, and opens the Usage page", function()
	local app = open()
	app:set_state("agents.on_device", { [""] = {} })
	app:frames(5)
	expect(app:tree("#app-rail")).to_contain('"No plan usage reported"')
	app:set_state("agents.on_device", accounts({ { label = "Session", used = 0.4 }, { label = "Week", used = 0.1 } }))
	app:frames(5)
	expect(app:tree("#app-rail")).to_contain('"Usage: Codex · 5h 40%, Weekly 10%"')
	expect(app:has("#rail-usage-limit")).to_be(false)
	app:set_state("agents.on_device", accounts({ { label = "Session", used = 1 } }))
	app:frames(5)
	expect(app:tree("#app-rail")).to_contain('"Usage: limit reached"')
	expect(app:has("#rail-usage-limit")).to_be(true)
	app:click("#rail-usage")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/settings/usage")
end)

test("Help's menu holds what opens something: Keyboard shortcuts and Star Zeron on GitHub", function()
	local app = open()
	app:click("#rail-help")
	app:frames(10)
	expect(app:has("#help-star")).to_be(true)
	app:click("#help-keybindings")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/settings/shortcuts")
end)

test("the engine's update is the rail's while it is ours or theirs; too old stays in the sidebar's foot", function()
	local app = open()
	expect(app:has("#update-rail-button")).to_be(false)
	app:set_state("engine.update", { stale = true, ours = false })
	app:frames(10)
	expect(app:tree("#app-rail")).to_contain("Zeron's engine is out of date")
	expect(app:has("#shell-sidebar >> #engine-update")).to_be(false)
	app:set_state("engine.update", { stale = false, ours = true })
	app:set_state("engine.interface", { supported = false })
	app:frames(10)
	expect(app:has("#update-rail-button")).to_be(false)
	expect(app:has("#shell-sidebar >> #engine-update")).to_be(true)
end)
