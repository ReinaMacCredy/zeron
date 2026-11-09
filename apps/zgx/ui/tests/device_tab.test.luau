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
	-- Its title's tooltip: the name and the device's line.
	app:hover('#panel-strip >> role:tab "Pixel 9"')
	expect(app:wait('~"This machine · Android 16"', { timeout_ms = 5000 })).to_be(true)
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

test("the picker's states: a host's note, the empty and failed lines, the starting banner, the loading steps while one opens", function()
	local app = open_panel()
	set_up(app)
	app:command("device_stub.ssh_host")
	expect(app:wait('#device-host-note-mini >> "Mac mini · Connection failed: Connection refused"', { timeout_ms = 3000 })).to_be(true)
	app:command("device_stub.booting")
	expect(app:wait('#device-booting >> "Starting iPad Air... This can take a minute."', { timeout_ms = 3000 })).to_be(true)
	-- Opening: the device's name, Starting, and the first of two steps, until the hub answers.
	app:command("device_stub.hold_open")
	app:click('role:button "Start Pixel 9"')
	expect(app:wait("#device-loading", { timeout_ms = 3000 })).to_be(true)
	local shown = app:tree("#device-loading")
	for _, w in { "Pixel 9", "This machine · Android 16", "Starting device...", "Step 1 of 2: open device" } do
		expect(`{w}: {shown:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	app:command("device_stub.let_open")
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	app:click("#device-more")
	app:click('role:menuitem "Power off"')
	expect(app:wait("#device-picker", { timeout_ms = 5000 })).to_be(true)
	app:command("device_stub.no_devices")
	expect(app:wait('#device-picker-empty >> "No simulators or emulators were found on this environment."', { timeout_ms = 3000 })).to_be(true)
	-- Failed: the line says why, and the host's card offers Retry.
	app:command("device_stub.failed")
	expect(app:wait('#device-picker-empty >> "Device support could not start: node exited (1)."', { timeout_ms = 3000 })).to_be(true)
	expect(app:has("#device-host-update >> #device-retry")).to_be(true)
	app:click("#device-retry")
	expect(calls(app)).to_contain("ListHubDevices")
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

test("a device the chat's agent opens comes up in a Device tab, once: a tab on its picker takes it, one closed isn't opened again", function()
	local app = open_panel()
	-- (Auto-show floating preview off: a tab, as gpui's.)
	app:set_state("device_auto_float", false)
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	app:frames(5)
	expect(tab_titles(app)).to_be("iPhone 17")
	app:command("device_stub.agent_open")
	expect(eventually(app, function()
		return tab_titles(app) == "iPhone 17,Pixel 9"
	end)).to_be(true)
	expect(calls(app)).to_contain("stream:Pixel_9")
	-- (Heard again, it's shown: no second tab.)
	app:command("device_stub.ready")
	app:frames(10)
	expect(tab_titles(app)).to_be("iPhone 17,Pixel 9")
	-- Closed: the agent opening it again opens no tab.
	app:click("#device-more")
	app:click('role:menuitem "Close panel"')
	expect(eventually(app, function()
		return tab_titles(app) == "iPhone 17"
	end)).to_be(true)
	app:command("device_stub.agent_close")
	app:command("device_stub.agent_open")
	app:frames(10)
	expect(tab_titles(app)).to_be("iPhone 17")
	-- A tab on its picker takes the next one.
	app:click("#device-more")
	app:click('role:menuitem "Power off"')
	expect(app:wait("#device-picker", { timeout_ms = 5000 })).to_be(true)
	expect(tab_titles(app)).to_be("Device")
	app:command("device_stub.agent_open_ipad")
	expect(eventually(app, function()
		return tab_titles(app) == "iPad Air"
	end)).to_be(true)
	-- Device off: an agent's device opens nothing.
	app:command("device_stub.builtin_off")
	app:frames(5)
	app:command("device_stub.agent_open_iphone")
	app:frames(10)
	app:command("device_stub.builtin_on")
	app:frames(10)
	expect(tab_titles(app)).to_be("iPad Air")
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
	expect(app:wait('"Device is off in Settings → Plugins."', { timeout_ms = 3000 })).to_be(true)
	expect(tab_titles(app)).to_be("Device")
	app:command("device_stub.builtin_on")
	expect(app:wait("#device-setup", { timeout_ms = 5000 })).to_be(true)
	expect(app:has('"Device is off in Settings → Plugins."')).to_be(false)
end)

test("with Device off, a device's tab offers no Rename", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	app:right_click('role:tab "iPhone 17"')
	expect(app:wait('role:menuitem "Rename"', { timeout_ms = 3000 })).to_be(true)
	app:key("Escape")
	app:frames(10)
	app:command("device_stub.builtin_off")
	app:frames(5)
	app:right_click('#panel-strip >> role:tab')
	app:frames(10)
	expect(app:has('role:menuitem "Close"') or app:has('role:menuitem ~"Close"')).to_be(true)
	expect(app:has('role:menuitem "Rename"')).to_be(false)
end)

test("a Device tab streams only while the panel shows it, and again once it shows", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	local function streams(): number
		local n = 0
		for _, c in app:state("device_stub_calls") or {} do
			if c == "stream:mock-iphone" then
				n += 1
			end
		end
		return n
	end
	expect(streams()).to_be(1)
	app:set_state("test_panel_visible", false)
	expect(eventually(app, function()
		return calls(app):find("unstream", 1, true) ~= nil
	end)).to_be(true)
	expect(next(app:state("device_frames") or {})).to_be_nil()
	app:set_state("test_panel_visible", true)
	expect(eventually(app, function()
		return streams() == 2
	end)).to_be(true)
end)

test("device input goes one call at a time, in order, a drag's moves merged into the newest", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Start Pixel 9"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	app:command("device_stub.hold_input")
	local function inputs(): { string }
		local out = {}
		for _, c in app:state("device_stub_calls") or {} do
			if string.find(c, "SendHubDeviceInput:", 1, true) then
				table.insert(out, (string.gsub(c, "^SendHubDeviceInput:", "")))
			end
		end
		return out
	end
	app:drag("#device-screen", "#device-screen")
	app:frames(5)
	expect(#inputs()).to_be(1)
	expect(inputs()[1]).to_contain("touch:begin")
	app:command("device_stub.let_input")
	app:frames(3)
	expect(#inputs()).to_be(2)
	expect(inputs()[2]).to_contain("touch:move")
	app:command("device_stub.let_input")
	app:frames(3)
	expect(#inputs()).to_be(3)
	expect(inputs()[3]).to_contain("touch:end")
	-- Keys behind a call in flight keep their order.
	app:click("#device-screen")
	app:frames(3)
	for _ = 1, 3 do
		app:command("device_stub.let_input")
		app:frames(3)
	end
	local before = #inputs()
	app:key("a")
	app:key("Enter")
	app:frames(3)
	app:command("device_stub.let_input")
	app:frames(3)
	app:command("device_stub.let_input")
	app:frames(3)
	local all = inputs()
	expect(#all).to_be(before + 2)
	expect(`{all[before + 1]},{all[before + 2]}`).to_be("text:a,keycode:66")
end)

test("a device the engine opened before the hub lists it stays bound, not back to the picker", function()
	local app = open_panel()
	set_up(app)
	app:command("device_stub.reply_first")
	app:click('role:button "Start Pixel 9"')
	app:frames(10)
	expect(tab_titles(app)).to_be("Pixel 9")
	app:command("device_stub.late_push")
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	expect(tab_titles(app)).to_be("Pixel 9")
	expect(calls(app)).never.to_contain("unstream")
end)

test("a stream the engine ends says so, still, with Try again", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	app:command("device_stub.stream_fails")
	expect(app:wait("#device-stream-failed", { timeout_ms = 3000 })).to_be(true)
	expect(app:tree("#device-stream-failed")).to_contain("The device stopped.")
	expect(app:has("#device-loading")).to_be(false)
	app:click("#device-stream-retry")
	expect(eventually(app, function()
		return not app:has("#device-stream-failed")
	end)).to_be(true)
	local n = 0
	for _, c in app:state("device_stub_calls") or {} do
		if c == "stream:mock-iphone" then
			n += 1
		end
	end
	expect(n).to_be(2)
end)

test("an engine without devices: the tab says so, and Device leaves the home", function()
	local app = open_panel()
	app:click("#panel-open-device")
	expect(app:wait("#device-setup", { timeout_ms = 5000 })).to_be(true)
	app:command("device_stub.old_engine")
	expect(eventually(app, function()
		return app:tree():find("This Zeron engine does not support devices.", 1, true) ~= nil
	end)).to_be(true)
	-- The tab closed, the home has no Device row on this engine.
	app:command("test.close_active_tab")
	expect(app:wait("#panel-empty", { timeout_ms = 5000 })).to_be(true)
	app:frames(5)
	expect(app:has("#panel-open-device")).to_be(false)
end)

test("Rotate turns the device: the picture and its box turn, and a touch lands where it shows on the turned screen", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	local function box(): (number, number)
		local w, h = string.match(app:tree("#device-screen"), " (%d+)x(%d+)")
		return tonumber(w) or 0, tonumber(h) or 0
	end
	local w, h = box()
	expect(h > w).to_be(true)
	app:click('role:button "Rotate device"')
	expect(eventually(app, function()
		return calls(app):find("orient:landscape_left", 1, true) ~= nil
	end)).to_be(true)
	expect(calls(app)).to_contain("SendHubDeviceInput:orientation:landscape_left")
	app:frames(5)
	w, h = box()
	expect(w > h).to_be(true)
	-- (Dragged off the right edge: on the screen turned to the left, that edge is the device's top.)
	app:drag("#device-screen", "#device-rotate")
	expect(eventually(app, function()
		return calls(app):find("SendHubDeviceInput:touch:end", 1, true) ~= nil
	end)).to_be(true)
	expect(string.match(calls(app), "SendHubDeviceInput:touch:end:[%d.]+:([%d.]+)")).to_be("0.00")
	app:click('role:button "Rotate device"')
	expect(eventually(app, function()
		return calls(app):find("orient:portrait_upside_down", 1, true) ~= nil
	end)).to_be(true)
	app:frames(5)
	w, h = box()
	expect(h > w).to_be(true)
end)
