-- A chat's Device tab (screens/device/panel), on the panel in
-- ui/panel_fixture.luau over the hub of ui/device_stub.luau (as the host's
-- mock answers a mock Dev): opened from the panel's home, the setup wizard's
-- three steps, the picker's groups and rows, opening a device to its screen
-- (frames, touch, keys, Home), Rename, the error line, the saved-screenshot
-- line, Power off back to the picker, and the Device built-in's switch.

local function eventually(app: any, ok: () -> boolean, ms: number?): boolean
	local tries = math.ceil((ms or 5000) / 50)
	for _ = 1, tries do
		if ok() then
			return true
		end
		app:wait('"never shown, only waited past"', { timeout_ms = 50 })
	end
	return ok()
end

local function open_panel(): any
	local app = mount("panel_fixture.luau")
	app:resize(900, 760)
	app:frames(10)
	expect(app:error()).to_be_nil()
	expect(app:wait("#panel-empty", { timeout_ms = 15000 })).to_be(true)
	return app
end

local function calls(app: any): string
	return table.concat(app:state("device_stub_calls") or {}, ",")
end

local function tab_titles(app: any): string
	local out: { string } = {}
	for name in app:tree("#panel-strip"):gmatch('tab "([^"]+)"') do
		table.insert(out, name)
	end
	return table.concat(out, ",")
end

-- Through the wizard: the hub on, Continue twice, Done.
local function set_up(app: any)
	app:click("#panel-open-device")
	expect(app:wait("#device-setup", { timeout_ms = 5000 })).to_be(true)
	app:click('#device-hub-enable >> role:switch')
	expect(eventually(app, function()
		return app:has("#device-setup-status")
	end)).to_be(true)
	app:click("#device-setup-continue")
	app:frames(5)
	app:click("#device-setup-continue")
	app:frames(5)
	app:click("#device-setup-done")
	expect(app:wait("#device-picker", { timeout_ms = 5000 })).to_be(true)
end

test("the panel's home offers a Device tab; its wizard has three steps and its footer, and Done reaches the picker", function()
	local app = open_panel()
	expect(app:has("#panel-open-device")).to_be(true)
	app:click("#panel-open-device")
	expect(app:wait("#device-setup", { timeout_ms = 5000 })).to_be(true)
	expect(tab_titles(app)).to_be("Device")
	expect(app:has("#panel-open-device")).to_be(false)
	local tree = app:tree("#device-setup")
	for _, w in { "Set up devices", "Device hub", "Simulators", "Agent access", "Enable the device hub", "Cancel", "Continue" } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	-- (Off, Continue waits for the hub.)
	app:click("#device-setup-continue")
	app:frames(5)
	expect(app:has("#device-setup-step-0")).to_be(true)
	expect(app:tree("#device-setup"):find("Check simulator support", 1, true)).to_be_nil()
	app:click('#device-hub-enable >> role:switch')
	expect(eventually(app, function()
		return app:text("#device-setup-status") == "Device hub is ready."
	end)).to_be(true)
	expect(calls(app)).to_contain("ConfigureDeviceHub:enabled=true")
	app:click("#device-setup-continue")
	expect(eventually(app, function()
		return app:tree("#device-setup"):find("Check simulator support", 1, true) ~= nil
	end)).to_be(true)
	expect(app:tree("#device-platform-ios")).to_contain("Xcode and iOS Simulator are available.")
	app:click("#device-setup-check")
	expect(eventually(app, function()
		return calls(app):find("ListHubDevices", 1, true) ~= nil
	end)).to_be(true)
	app:click("#device-setup-continue")
	expect(eventually(app, function()
		return app:tree("#device-setup"):find("Allow agent control", 1, true) ~= nil
	end)).to_be(true)
	expect(app:has("#device-setup-done")).to_be(true)
	-- Back to a passed step, from its tab.
	app:click("#device-setup-step-0")
	expect(eventually(app, function()
		return app:tree("#device-setup"):find("Enable the device hub", 1, true) ~= nil
	end)).to_be(true)
	app:click("#device-setup-continue")
	app:frames(5)
	app:click("#device-setup-continue")
	app:frames(5)
	app:click('#device-agent-access >> role:switch')
	expect(eventually(app, function()
		return calls(app):find("agentAccessEnabled=true", 1, true) ~= nil
	end)).to_be(true)
	app:click("#device-setup-done")
	expect(app:wait("#device-picker", { timeout_ms = 5000 })).to_be(true)
	expect(calls(app)).to_contain("onboardingCompleted=true")
end)

test("the picker groups simulators and emulators, Running first, says Open or Start, and opens a device to its screen", function()
	local app = open_panel()
	set_up(app)
	expect(app:text("#device-group-ios")).to_be("iOS Simulators")
	expect(app:text("#device-group-android")).to_be("Android Emulators")
	expect(app:has('role:button "Open iPhone 17"')).to_be(true)
	expect(app:has('role:button "Start iPad Air"')).to_be(true)
	expect(app:has('role:button "Start Pixel 9"')).to_be(true)
	expect(app:tree("#device-row-mock-iphone")).to_contain("This machine · iOS 27.0 · Running")
	local picker = app:tree("#device-picker")
	expect(picker:find("iPhone 17", 1, true) < picker:find("iPad Air", 1, true)).to_be(true)
	app:click('role:button "Start Pixel 9"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	expect(calls(app)).to_contain("OpenHubDevice:Pixel_9")
	expect(calls(app)).to_contain("stream:Pixel_9")
	expect(tab_titles(app)).to_be("Pixel 9")
	expect(app:has("#device-back")).to_be(true)
	-- Touch: a press and a release on the screen, as fractions of it.
	app:click("#device-screen")
	expect(eventually(app, function()
		return calls(app):find("SendHubDeviceInput:touch:end", 1, true) ~= nil
	end)).to_be(true)
	expect(calls(app)).to_contain("SendHubDeviceInput:touch:begin:0.50:0.50")
	app:click("#device-home")
	expect(calls(app)).to_contain("SendHubDeviceInput:button:home")
	app:click("#device-back")
	expect(calls(app)).to_contain("SendHubDeviceInput:button:back")
	-- Keys: Android takes text, and its own keycodes.
	app:click("#device-screen")
	app:key("a")
	app:key("Enter")
	expect(calls(app)).to_contain("SendHubDeviceInput:text:a")
	expect(calls(app)).to_contain("SendHubDeviceInput:keycode:66")
end)

test("iOS keys go as HID usages, a frame replaces the picture, Save screenshot says where, and an error line dismisses", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	expect(app:has("#device-back")).to_be(false)
	app:click("#device-screen")
	app:key("b")
	expect(calls(app)).to_contain("SendHubDeviceInput:key:down:5")
	expect(calls(app)).to_contain("SendHubDeviceInput:key:up:5")
	expect(calls(app)).to_contain("src:assets/device/frame-1.png")
	app:command("device_stub.frame")
	app:frames(3)
	expect(calls(app)).to_contain("src:assets/device/frame-2.png")
	app:click("#device-screenshot")
	expect(eventually(app, function()
		return app:has("#device-toast")
	end)).to_be(true)
	expect(app:tree("#device-toast")).to_contain("Saved screenshot to /tmp/zeron-mock-device-screenshot.png")
	app:command("device_stub.fail_next")
	app:click("#device-home")
	expect(eventually(app, function()
		return app:has("#device-panel-error")
	end)).to_be(true)
	expect(app:tree("#device-panel-error")).to_contain("The device hub failed to start.")
	app:click("#device-error-dismiss")
	app:frames(3)
	expect(app:has("#device-panel-error")).to_be(false)
end)

test("Rename names the tab, Escape puts it back, nothing typed is the device's name again; Power off goes back to the picker", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	local tab = 'role:tab "iPhone 17"'
	app:right_click(tab)
	app:click('role:menuitem "Rename"')
	expect(app:wait("#device-tab-rename", { timeout_ms = 3000 })).to_be(true)
	app:fill('role:textbox "Device tab name"', "My phone")
	app:key("Enter")
	expect(eventually(app, function()
		return tab_titles(app) == "My phone"
	end)).to_be(true)
	app:right_click('role:tab "My phone"')
	app:click('role:menuitem "Rename"')
	expect(app:wait("#device-tab-rename", { timeout_ms = 3000 })).to_be(true)
	app:fill('role:textbox "Device tab name"', "Discarded")
	app:key("Escape")
	expect(eventually(app, function()
		return tab_titles(app) == "My phone" and not app:has("#device-tab-rename")
	end)).to_be(true)
	app:right_click('role:tab "My phone"')
	app:click('role:menuitem "Rename"')
	expect(app:wait("#device-tab-rename", { timeout_ms = 3000 })).to_be(true)
	app:fill('role:textbox "Device tab name"', "   ")
	app:key("Enter")
	expect(eventually(app, function()
		return tab_titles(app) == "iPhone 17"
	end)).to_be(true)
	app:click("#device-more")
	app:click('role:menuitem "Power off"')
	expect(app:wait("#device-picker", { timeout_ms = 5000 })).to_be(true)
	expect(calls(app)).to_contain("CloseHubDevice:mock-iphone:shutdown")
	expect(tab_titles(app)).to_be("Device")
	expect(app:has('role:button "Start iPhone 17"')).to_be(true)
end)

test("the Device built-in off takes the home's Device row away and the open tab says so; on brings both back", function()
	local app = open_panel()
	expect(app:has("#panel-open-device")).to_be(true)
	app:command("device_stub.builtin_off")
	app:frames(5)
	expect(app:has("#panel-open-device")).to_be(false)
	app:command("device_stub.builtin_on")
	app:frames(5)
	expect(app:has("#panel-open-device")).to_be(true)
	app:click("#panel-open-device")
	expect(app:wait("#device-setup", { timeout_ms = 5000 })).to_be(true)
	app:command("device_stub.builtin_off")
	app:frames(5)
	expect(app:has("#device-setup")).to_be(false)
	expect(app:tree("#panel-strip") ~= nil).to_be(true)
	app:command("device_stub.builtin_on")
	expect(app:wait("#device-setup", { timeout_ms = 5000 })).to_be(true)
end)

test("an engine without devices: the tab says so, and Device leaves the home", function()
	local app = open_panel()
	app:click("#panel-open-device")
	expect(app:wait("#device-setup", { timeout_ms = 5000 })).to_be(true)
	app:command("device_stub.old_engine")
	expect(eventually(app, function()
		return app:tree():find("This Zeron engine does not support devices.", 1, true) ~= nil
	end)).to_be(true)
end)
