-- Frames are gpui's float.rs numbers for the stub's 196x426 screen in the fixture's 600x600 chat column.

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

local function box(app: any, sel: string): string?
	return string.match(app:tree(sel), "^[^\n]-(%-?%d+,%-?%d+ %d+x%d+)")
end

local function floating(app: any): boolean
	return app:has("#device-float")
end

local function streams(app: any, device: string): number
	local n = 0
	for _, c in app:state("device_stub_calls") or {} do
		if c == `stream:{device}` then
			n += 1
		end
	end
	return n
end

local function drag_to(app: any, from: string, x: number, y: number)
	app:set_state("test_float_to", { x = x, y = y })
	app:frames(2)
	app:drag(from, "#float-to")
	app:frames(3)
end

local function tab_titles(app: any): string
	local out: { string } = {}
	for name in app:tree("#panel-strip"):gmatch('tab "([^"]+)"') do
		table.insert(out, name)
	end
	return table.concat(out, ",")
end

test("an agent's device floats over its chat with Auto-show floating preview on: gpui's corner and size, no tab, its own stream and touch", function()
	local app = open_panel()
	set_up(app)
	app:command("device_stub.agent_open")
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	expect(box(app, "#device-float")).to_be("348,12 240x522")
	expect(tab_titles(app)).to_be("Device")
	expect(app:has("#device-picker")).to_be(true)
	expect(streams(app, "Pixel_9")).to_be(1)
	app:click("#device-float-screen")
	expect(eventually(app, function()
		return calls(app):find("SendHubDeviceInput:touch:end", 1, true) ~= nil
	end)).to_be(true)
	expect(calls(app)).to_contain("SendHubDeviceInput:touch:begin:0.50:0.50")
	-- (The agent closing it closes the float.)
	app:command("device_stub.agent_close")
	expect(eventually(app, function()
		return not floating(app)
	end)).to_be(true)
	expect(next(app:state("device_floats") or {})).to_be_nil()
end)

test("with Auto-show floating preview off, an agent's device opens a tab and no float", function()
	local app = open_panel()
	app:set_state("device_auto_float", false)
	set_up(app)
	app:command("device_stub.agent_open")
	expect(eventually(app, function()
		return tab_titles(app) == "Pixel 9"
	end)).to_be(true)
	app:frames(5)
	expect(floating(app)).to_be(false)
	expect(next(app:state("device_floats") or {})).to_be_nil()
end)

test("the device float hides while the panel shows the same device or the chat isn't shown, streaming only while it shows", function()
	local app = open_panel()
	set_up(app)
	app:click('role:button "Open iPhone 17"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	-- More → Float device over chat: recorded, hidden while the panel shows that device.
	app:click("#device-more")
	app:click('role:menuitem "Float device over chat"')
	expect(eventually(app, function()
		return (app:state("device_floats") or {})["chat-1"] ~= nil
	end)).to_be(true)
	expect(floating(app)).to_be(false)
	expect(tab_titles(app)).to_be("iPhone 17")
	app:set_state("test_panel_visible", false)
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	expect(streams(app, "mock-iphone")).to_be(2)
	app:set_state("test_float_chat_shown", false)
	expect(eventually(app, function()
		return not floating(app)
	end)).to_be(true)
	expect(next(app:state("device_frames") or {})).to_be_nil()
	app:set_state("test_float_chat_shown", true)
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	expect(streams(app, "mock-iphone")).to_be(3)
end)

test("dragging the device float's handle moves it and an edge resizes it, in the column with the 12-point gap, at the device's aspect; the last gesture is kept", function()
	local app = open_panel()
	set_up(app)
	app:command("device_stub.agent_open")
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	expect(box(app, "#device-float")).to_be("348,12 240x522")
	-- The bottom edge down: it grows till the column's height, pushed left to stay in.
	drag_to(app, "#device-float-resize-bottom", 360, 750)
	expect(box(app, "#device-float")).to_be("323,12 265x576")
	-- Up past the least: 240 wide.
	drag_to(app, "#device-float-resize-bottom", 335, 100)
	expect(box(app, "#device-float")).to_be("323,12 240x522")
	-- The handle far down and left: clamped in the corner.
	drag_to(app, "#device-float-grip", 0, 750)
	expect(box(app, "#device-float")).to_be("12,66 240x522")
	local kept = app:state("device_float_frame")
	expect(`{kept.x},{kept.y},{kept.width},{kept.height}`).to_be("12,66,240,522")
	-- A narrow column fits it; a move there keeps the size kept.
	app:set_state("test_float_column", { w = 250, h = 600 })
	expect(eventually(app, function()
		return box(app, "#device-float") == "12,66 226x491"
	end)).to_be(true)
	drag_to(app, "#device-float-grip", 0, 0)
	expect(box(app, "#device-float")).to_be("12,12 226x491")
	kept = app:state("device_float_frame")
	expect(`{kept.x},{kept.y},{kept.width},{kept.height}`).to_be("12,12,240,522")
end)

test("the device float's Close takes it away; Open in right panel puts it in a Device tab, the picker's", function()
	local app = open_panel()
	set_up(app)
	app:command("device_stub.agent_open")
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	app:hover("#device-float-handle")
	app:frames(3)
	app:click("#device-float-close")
	expect(eventually(app, function()
		return not floating(app)
	end)).to_be(true)
	expect(next(app:state("device_floats") or {})).to_be_nil()
	expect(tab_titles(app)).to_be("Device")
	-- The agent opening another floats it; Open in right panel binds the picker's tab to it.
	app:command("device_stub.agent_open_ipad")
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	app:hover("#device-float-handle")
	app:frames(3)
	app:click("#device-float-open")
	expect(eventually(app, function()
		return tab_titles(app) == "iPad Air"
	end)).to_be(true)
	expect(floating(app)).to_be(false)
	expect(next(app:state("device_floats") or {})).to_be_nil()
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
end)

test("the pane's toggle closing it floats the device its active Device tab shows", function()
	local app = open_panel()
	set_up(app)
	app:command("test.float_active_device_tab")
	app:frames(3)
	expect(next(app:state("device_floats") or {})).to_be_nil()
	app:click('role:button "Start Pixel 9"')
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	app:command("test.float_active_device_tab")
	app:frames(3)
	local f = (app:state("device_floats") or {})["chat-1"]
	expect(f and `{f.host}/{f.id}`).to_be("local/Pixel_9")
	app:set_state("test_panel_visible", false)
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	expect(tab_titles(app)).to_be("Pixel 9")
end)

test("the device float's Open in right panel opens a tab for a device whose tab was closed before", function()
	local app = open_panel()
	app:set_state("device_auto_float", false)
	set_up(app)
	app:command("device_stub.agent_open")
	expect(eventually(app, function()
		return tab_titles(app) == "Pixel 9"
	end)).to_be(true)
	expect(app:wait("#device-screen", { timeout_ms = 5000 })).to_be(true)
	app:click("#device-more")
	app:click('role:menuitem "Close panel"')
	expect(eventually(app, function()
		return tab_titles(app) == ""
	end)).to_be(true)
	app:command("device_stub.agent_close")
	app:set_state("device_auto_float", true)
	app:frames(3)
	app:command("device_stub.agent_open")
	expect(eventually(app, function()
		return floating(app)
	end)).to_be(true)
	expect(tab_titles(app)).to_be("")
	app:hover("#device-float-handle")
	app:frames(3)
	app:click("#device-float-open")
	expect(eventually(app, function()
		return tab_titles(app) == "Pixel 9"
	end)).to_be(true)
	expect(floating(app)).to_be(false)
end)
