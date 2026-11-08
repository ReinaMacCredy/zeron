-- Settings → Appearance → Pause animations in background, on ui/shell_fixture.luau.

local PAGE = "#settings-page"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 2600)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> role:tab "Appearance"')
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
