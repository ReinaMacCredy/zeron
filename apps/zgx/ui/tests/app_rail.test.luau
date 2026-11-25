-- The app rail on the shell in ui/shell_fixture.luau: there whatever the
-- sidebar and Settings do, its entries where they lead.

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
	-- (Under the panel's scrim now: ⌘, again closes it.)
	app:command("shell.settings")
	expect(app:wait("#settings-nav", { gone = true, timeout_ms = 1000 })).to_be(true)
end)

test("with motion reduced (as a carried-over setting can be), a rail entry made then still shows", function()
	local app = open()
	app:set_state("ds/motion.luau::motion_reduce", true)
	app:frames(5)
	app:set_state("plugins.pages", PULLS)
	app:frames(10)
	app:click("#rail-more")
	app:frames(10)
	expect(app:has("#open-pull-requests")).to_be(true)
	expect(app:tree("#app-rail")).to_contain('#settings-trigger button "Settings"')
end)

test("Home leads home; More lists Pull requests while its plugin's page does, and only the rail lists it", function()
	local app = open()
	app:click("#rail-more")
	app:frames(10)
	expect(app:has("#rail-inbox")).to_be(true)
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
end)

test("the rail is Home, Bots, Search and More, then Help and Settings: no Usage ring; Search opens the palette", function()
	local app = open()
	local tree = app:tree("#app-rail")
	for _, id in { "#rail-new-chat", "#rail-bots", "#rail-search", "#rail-more", "#rail-help", "#settings-trigger" } do
		expect(app:has(id)).to_be(true)
	end
	expect(app:has("#rail-usage")).to_be(false)
	expect(tree).to_contain('#rail-new-chat button "Home"')
	app:click("#rail-bots")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/plugin/zeron/bots")
	app:click("#rail-search")
	app:frames(10)
	expect(app:state("shell_palette_open")).to_be(true)
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

-- A node's box, from the tree's first line: x, y, w, h.
local function box(app: any, target: string): (number, number, number, number)
	local x, y, w, h = (app:tree(target):match("[^\n]+") or ""):match(" (%-?[%d.]+),(%-?[%d.]+) ([%d.]+)x([%d.]+)")
	return tonumber(x) :: number, tonumber(y) :: number, tonumber(w) :: number, tonumber(h) :: number
end

test("you are on the rail, right under Settings, and open your profile; the sidebar's foot no longer shows you", function()
	local app = open()
	expect(app:tree("#app-rail")).to_contain('#rail-account button "Local"')
	expect(app:has('#shell-sidebar >> "Local"')).to_be(false)
	local _, ay, _, ah = box(app, "#rail-account")
	local _, sy = box(app, "#settings-trigger")
	local _, _, _, sh = box(app, "#settings-trigger")
	expect(ay - (sy + sh)).to_be(6)
	app:click("#rail-account")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/settings/profile")
end)

local function about(app: any, fields: { [string]: any })
	local now: { [string]: any } = { app_version = "0.2.131", available = true, latest = "0.2.140", self_updating = true, flow = "idle" }
	for k, v in fields do
		now[k] = v
	end
	app:set_state("about", now)
	app:frames(10)
end

local function asked(app: any, entry: string): number
	local n = 0
	for _, c in (app:state("settings_stub_calls") :: any) or {} do
		if c == entry then
			n += 1
		end
	end
	return n
end

test("the app's own update is the rail's in each of its phases, its motion only where it moves, and wins over the engine's", function()
	local app = open()
	expect(app:has("#update-rail-button")).to_be(false)
	about(app, {})
	expect(app:tree("#app-rail")).to_contain('#update-rail-button button "Version 0.2.140 available"')
	expect(app:has("#update-glyph-arc")).to_be(false)
	expect(app:has("#update-glyph-halo-1")).to_be(false)
	app:click("#update-rail-button")
	app:frames(10)
	expect(asked(app, "about.apply")).to_be(1)

	about(app, { flow = "downloading", flow_version = "0.2.140" })
	expect(app:tree("#app-rail")).to_contain('#update-rail-button button "Downloading…"')
	expect(app:has("#update-glyph-arc")).to_be(true)
	expect(app:has("#update-glyph-arrow")).to_be(true)
	expect(app:has("#update-glyph-halo-1")).to_be(false)
	-- (The arrow falls: its loop started. It starts in step with the clock, so
	-- anywhere in its fall, and a fall's ends move under a point in 8 frames;
	-- half a second on, it is always somewhere else.)
	local _, fell = box(app, "#update-glyph-arrow")
	app:frames(30)
	local _, now = box(app, "#update-glyph-arrow")
	expect(now ~= fell).to_be(true)

	about(app, { flow = "ready", flow_version = "0.2.140" })
	expect(app:tree("#app-rail")).to_contain('#update-rail-button button "0.2.140 is ready. Restart to finish"')
	expect(app:has("#update-glyph-halo-1")).to_be(true)
	expect(app:has("#update-glyph-halo-2")).to_be(true)
	expect(app:has("#update-glyph-arc")).to_be(false)

	about(app, { flow = "failed", flow_version = "0.2.140", flow_error = "no space" })
	expect(app:tree("#app-rail")).to_contain('#update-rail-button button "Update failed"')
	expect(app:has("#update-glyph-halo-1")).to_be(false)

	app:set_state("engine.update", { stale = true, ours = false })
	about(app, { flow = "idle" })
	expect(app:tree("#app-rail")).to_contain('#update-rail-button button "Version 0.2.140 available"')
	expect(app:tree("#app-rail")).never.to_contain("Zeron's engine is out of date")
	about(app, { available = false })
	expect(app:tree("#app-rail")).to_contain("Zeron's engine is out of date")
end)

test("with motion reduced the update rests in every phase: no arc, arrow or halo", function()
	local app = open()
	app:set_state("ds/motion.luau::motion_reduce", true)
	app:frames(5)
	about(app, { flow = "downloading", flow_version = "0.2.140" })
	expect(app:has("#update-rail-button")).to_be(true)
	expect(app:has("#update-glyph-arc")).to_be(false)
	expect(app:has("#update-glyph-arrow")).to_be(false)
	about(app, { flow = "ready", flow_version = "0.2.140" })
	expect(app:has("#update-glyph-halo-1")).to_be(false)
end)

test("an install that can't update itself explains why from the rail; a development build's engine update isn't the rail's", function()
	local app = open()
	about(app, { self_updating = false, blocker = "Zeron is on a read-only volume" })
	expect(app:tree("#app-rail")).to_contain('#update-rail-button button "Version 0.2.140 available"')
	app:click("#update-rail-button")
	app:frames(20)
	expect(app:has("role:dialog")).to_be(true)
	expect(asked(app, "about.apply")).to_be(0)
	about(app, { self_updating = false, blocker = nil :: any })
	expect(app:has("#update-rail-button")).to_be(false)
end)
