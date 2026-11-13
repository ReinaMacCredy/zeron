-- Settings → Appearance → Pause animations in background, on ui/shell_fixture.luau.

local reachable = require("./settings_panel").reachable

local PAGE = "#settings-page"

local function open(): any
	local app = reachable(mount("shell_fixture.luau"))
	app:resize(1200, 2600)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> role:tab "Appearance"')
	-- (Its page's module loads before the next frame, and its states with it.)
	app:frames(1)
	app:set_state("settings_appearance_advanced", true)
	app:frames(30)
	return app
end

test("Pause animations in background: under Reduce motion, off at first, kept as set at the next launch", function()
	local app = open()
	expect(app:has(`{PAGE} >> role:switch "Pause animations in background"`)).to_be(true)
	expect(app:state("motion_pause_background")).to_be(false)
	app:click(`{PAGE} >> role:switch "Pause animations in background"`)
	app:frames(10)
	expect(app:state("motion_pause_background")).to_be(true)
	local next = open()
	expect(next:state("motion_pause_background")).to_be(true)
	expect(next:tree(`{PAGE} >> role:switch "Pause animations in background"`)).never.to_contain("unchecked")
end)

test("Settings' pages load as they're first shown, not with the shell, and what one follows stays followed once it's put away", function()
	local app = reachable(mount("shell_fixture.luau"))
	app:frames(30)
	-- (Appearance's own state: there once its module has loaded.)
	expect((pcall(function()
		return app:state("settings_reduce_motion")
	end))).to_be(false)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> role:tab "Appearance"')
	app:frames(1)
	expect(app:state("settings_reduce_motion")).to_be("system")
	app:click('#settings-nav >> role:tab "General"')
	app:frames(10)
	app:set_state("motion_reduce", true)
	app:frames(5)
	expect(app:state("settings_reduce_motion")).to_be("on")
end)

test("Smooth scrolling: on at first, kept as set at the next launch, and stepping while motion is reduced", function()
	local app = open()
	local SWITCH = `{PAGE} >> role:switch "Smooth scrolling"`
	expect(app:has(SWITCH)).to_be(true)
	expect(app:state("smooth_scrolling")).to_be(true)
	expect(app:tree("#setting-smooth_scrolling")).to_contain("Mouse wheels glide instead of jumping")
	app:set_state("motion_reduce", true)
	app:frames(10)
	expect(app:tree("#setting-smooth_scrolling")).to_contain("Mouse wheels step instantly while motion is reduced.")
	app:set_state("motion_reduce", false)
	app:click(SWITCH)
	app:frames(10)
	expect(app:state("smooth_scrolling")).to_be(false)
	local next = open()
	expect(next:state("smooth_scrolling")).to_be(false)
	expect(next:tree(SWITCH)).to_contain("unchecked")
end)
