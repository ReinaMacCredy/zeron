-- Settings → Appshots' intro, status line, resets, Restore defaults and Preview, on ui/shell_fixture.luau.

local PAGE = "#settings-page"
local READY = { platform = true, enabled = true, shortcut_ready = true, capture = "ready", text = "ready" }

local function open(state: any): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("shell.settings")
	app:frames(20)
	app:set_state("appshots.state", state)
	app:click('#settings-nav >> role:tab "Appshots"')
	app:frames(30)
	expect(app:error()).to_be_nil()
	return app
end
local function shows(app: any, words: string): boolean
	return app:tree(PAGE):find(words, 1, true) ~= nil
end
local function enable_line(app: any, words: string): boolean
	return app:tree("#setting-enabled"):find(words, 1, true) ~= nil
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end

test("Appshots: the intro, and the Enable row's line says what the listener is doing", function()
	local app = open(READY)
	expect(shows(app, "Take an Appshot to show your agent another app's window")).to_be(true)
	expect(shows(app, "Appshots are available only in")).to_be(false)
	expect(app:state("appshots_enabled")).to_be(false)
	expect(enable_line(app, "Off")).to_be(true)
	app:click(`{PAGE} >> role:switch "Capture Appshots"`)
	app:frames(10)
	expect(enable_line(app, "Listening: press")).to_be(true)
	app:set_state("appshots.state", { platform = true, enabled = true, shortcut_ready = false, capture = "ready", text = "ready" })
	app:frames(10)
	expect(enable_line(app, "This shortcut is unavailable.")).to_be(true)
	app:set_state("appshots.state", { platform = true, enabled = true, shortcut_ready = true, capture = "missing", text = "ready" })
	app:frames(10)
	expect(enable_line(app, "Allow Screen Recording in macOS")).to_be(true)
	app:set_state("appshots.state", { platform = false })
	app:frames(10)
	expect(shows(app, "Appshots are available only in")).to_be(true)
	expect(enable_line(app, "Available on macOS and Linux only")).to_be(true)
end)

test("Appshots: a row off its default has a reset, and Restore defaults puts them all back, kept at the next launch", function()
	local app = open(READY)
	expect(app:tree("#appshots-restore-defaults")).to_contain("disabled")
	expect(app:has("#appshots-sound-reset")).to_be(false)
	app:click(`{PAGE} >> role:switch "Capture sound"`)
	app:frames(10)
	expect(app:state("appshots_sound")).to_be(false)
	expect(app:has("#appshots-sound-reset")).to_be(true)
	app:click("#appshots-sound-reset")
	app:frames(10)
	expect(app:state("appshots_sound")).to_be(true)
	expect(app:has("#appshots-sound-reset")).to_be(false)
	app:click(`{PAGE} >> role:switch "Capture Appshots"`)
	app:set_state("appshots_destination", "new")
	app:set_state("appshots_shortcut", "ctrl+shift+k")
	app:frames(10)
	expect(app:has("#appshots-enabled-reset")).to_be(true)
	expect(app:has("#appshots-destination-reset")).to_be(true)
	expect(app:has("#appshots-shortcut-reset")).to_be(true)
	expect(app:tree("#appshots-restore-defaults")).never.to_contain("disabled")
	local next = open(READY)
	expect(next:state("appshots_enabled")).to_be(true)
	expect(next:state("appshots_destination")).to_be("new")
	next:click("#appshots-restore-defaults")
	next:frames(10)
	expect(next:state("appshots_enabled")).to_be(false)
	expect(next:state("appshots_sound")).to_be(true)
	expect(next:state("appshots_destination")).to_be("automatic")
	expect(next:state("appshots_shortcut")).to_be("ctrl+alt+space")
	expect(next:has("#appshots-shortcut-reset")).to_be(false)
end)

test("Appshots: Preview plays the capture sound", function()
	local app = open(READY)
	app:click("#appshots-sound-preview")
	app:frames(5)
	expect(calls(app)).to_contain("sound.play:appshot")
end)
