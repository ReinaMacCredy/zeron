-- Settings → Appshots as the shipping app's AppSnap page (crates/ui
-- settings/appshots.rs), on ui/shell_fixture.luau with the host's state set
-- by hand: the intro, the Enable row's line, the chord menus and their
-- refusal, Destination, resets and Restore defaults, the permission guide
-- (Grant asks, unfolds the steps, folds once granted) and Recheck.

local PAGE = "#settings-page"
local function ready(over: { [string]: any }?): any
	local s = {
		platform = "macos",
		loaded = true,
		app = "Zeron",
		enabled = false,
		sound = true,
		hotkey = "command-command",
		quick_capture = "option-option",
		destination = "automatic",
		shortcut_ready = true,
		capture = "ready",
		text = "ready",
		input = "ready",
	}
	for k, v in over or {} do
		s[k] = v
	end
	return s
end

local function open(state: any): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1500)
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
local function line(app: any, id: string, words: string): boolean
	return app:tree(id):find(words, 1, true) ~= nil
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end
local function set(app: any, over: { [string]: any })
	app:set_state("appshots.state", ready(over))
	app:frames(10)
end

test("Appshots: the intro, and the Enable row's line says what the listener is doing", function()
	local app = open(ready())
	expect(shows(app, "Take an Appshot to show your agent another app's window")).to_be(true)
	expect(shows(app, "Capture another app's frontmost window")).to_be(true)
	expect(shows(app, "Appshots are available only in")).to_be(false)
	expect(line(app, "#setting-enabled", "Off")).to_be(true)
	app:click(`{PAGE} >> role:switch "Enable Appshots"`)
	app:frames(10)
	expect(app:state("appshots_enabled")).to_be(true)
	expect(calls(app)).to_contain("appshots.configure")
	expect(line(app, "#setting-enabled", "Listening: press ⌘ + ⌘ to snap")).to_be(true)
	set(app, { enabled = true, input = "required" })
	expect(line(app, "#setting-enabled", "Allow Input Monitoring in macOS")).to_be(true)
	set(app, { enabled = true, capture = "required" })
	expect(line(app, "#setting-enabled", "Allow Screen Recording in macOS")).to_be(true)
	set(app, { enabled = true, shortcut_ready = false })
	expect(line(app, "#setting-enabled", "This shortcut is unavailable.")).to_be(true)
	set(app, { enabled = true, hotkey = "none" })
	expect(line(app, "#setting-enabled", "No hotkey selected")).to_be(true)
	expect(shows(app, "No hotkey. Choose a chord")).to_be(true)
	set(app, { platform = "unsupported" })
	expect(shows(app, "Appshots are available only in the desktop app.")).to_be(true)
	expect(line(app, "#setting-enabled", "Available on macOS and Linux only")).to_be(true)
	expect(shows(app, "Input Monitoring")).to_be(false)
end)

test("Appshots: the hotkey is a chord from a menu, refusing the one Quick capture holds", function()
	local app = open(ready())
	expect(shows(app, "Press both ⌘ keys simultaneously")).to_be(true)
	expect(app:has("#appshots-hotkey-reset")).to_be(false)
	local function pick(button: string, item: string)
		app:click(`{PAGE} >> role:button "{button}"`)
		app:frames(10)
		expect(app:wait(`role:menuitem "{item}"`, { timeout_ms = 15000 })).to_be(true)
		app:click(`role:menuitem "{item}"`)
		app:frames(20)
	end
	pick("Appshot hotkey: ⌘ + ⌘", "⌥ + ⌥")
	expect(app:state("appshots_hotkey")).to_be("command-command")
	expect(line(app, "#setting-hotkey", "⌥ + ⌥ is already the Quick capture hotkey.")).to_be(true)
	pick("Appshot hotkey: ⌘ + ⌘", "⇧ + ⇧")
	expect(app:state("appshots_hotkey")).to_be("shift-shift")
	expect(line(app, "#setting-hotkey", "already the")).to_be(false)
	expect(shows(app, "Press both ⇧ keys simultaneously")).to_be(true)
	expect(app:has("#appshots-hotkey-reset")).to_be(true)
	pick("Quick capture hotkey: ⌥ + ⌥", "⇧ + ⇧")
	expect(app:state("appshots_quick_capture")).to_be("option-option")
	expect(line(app, "#setting-quick", "⇧ + ⇧ is already the Appshot hotkey.")).to_be(true)
	pick("Quick capture hotkey: ⌥ + ⌥", "None")
	expect(app:state("appshots_quick_capture")).to_be("none")
	expect(shows(app, "No hotkey. The + under Tasks")).to_be(true)
end)

test("Appshots: a row off its default has a reset, and Restore defaults puts them all back", function()
	local app = open(ready())
	expect(app:tree("#appshots-restore-defaults")).to_contain("disabled")
	app:click(`{PAGE} >> role:switch "Capture sound"`)
	app:frames(10)
	expect(app:state("appshots_sound")).to_be(false)
	app:click("#appshots-sound-reset")
	app:frames(10)
	expect(app:state("appshots_sound")).to_be(true)
	app:click(`{PAGE} >> role:radio "New session"`)
	app:frames(10)
	expect(app:state("appshots_destination")).to_be("new-session")
	expect(shows(app, "Stage captures in a new-session composer")).to_be(true)
	expect(app:has("#appshots-destination-reset")).to_be(true)
	app:set_state("appshots_hotkey", "option-option")
	app:set_state("appshots_quick_capture", "shift-shift")
	app:frames(10)
	expect(app:tree("#appshots-restore-defaults")).never.to_contain("disabled")
	app:click("#appshots-restore-defaults")
	app:frames(10)
	expect(app:state("appshots_destination")).to_be("automatic")
	expect(app:state("appshots_hotkey")).to_be("command-command")
	expect(app:state("appshots_quick_capture")).to_be("option-option")
	expect(app:has("#appshots-destination-reset")).to_be(false)
	expect(shows(app, "Snaps join the session you interacted")).to_be(true)
end)

test("Appshots: Grant asks macOS, opens its pane and the steps, which fold once the grant lands", function()
	local app = open(ready({ input = "required", capture = "required" }))
	expect(line(app, "#setting-input", "Not requested yet")).to_be(true)
	expect(line(app, "#setting-text", "Granted")).to_be(true)
	expect(app:has("#appshots-grant-text")).to_be(false)
	app:click("#appshots-grant-capture")
	app:frames(10)
	expect(calls(app)).to_contain("appshots.request")
	expect(line(app, "#setting-capture", "Denied")).to_be(true)
	expect(app:has("#appshots-guide-capture")).to_be(true)
	expect(shows(app, "If this copy of Zeron is already listed")).to_be(true)
	expect(shows(app, "Watching for the change")).to_be(true)
	app:click("#appshots-guide-open-capture")
	app:frames(5)
	expect(calls(app)).to_contain("appshots.open_settings")
	set(app, { input = "required" })
	expect(app:has("#appshots-guide-capture")).to_be(false)
	expect(line(app, "#setting-status", "Permission granted. Screen Recording is ready")).to_be(true)
	-- Hide steps folds an open guide without asking again.
	app:click("#appshots-grant-input")
	app:frames(10)
	expect(app:has("#appshots-guide-input")).to_be(true)
	app:click("#appshots-grant-input")
	app:frames(10)
	expect(app:has("#appshots-guide-input")).to_be(false)
end)

test("Appshots: Recheck says when nothing changed", function()
	local app = open(ready({ capture = "required" }))
	app:click("#appshots-refresh-permissions")
	app:frames(40)
	app:wait('~"Permissions unchanged."', { timeout_ms = 2000 })
	expect(line(app, "#setting-status", "Permissions unchanged.")).to_be(true)
end)

test("Appshots: Enable asks for the first missing grant once and opens its guide", function()
	local app = open(ready({ input = "required", capture = "required" }))
	app:click(`{PAGE} >> role:switch "Enable Appshots"`)
	app:frames(10)
	expect(app:has("#appshots-guide-input")).to_be(true)
	expect(line(app, "#setting-input", "Denied")).to_be(true)
end)

test("Appshots: Preview plays the capture sound", function()
	local app = open(ready())
	app:click("#appshots-sound-preview")
	app:frames(5)
	expect(calls(app)).to_contain("sound.play:appshot")
end)
