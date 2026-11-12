-- Settings → Device hub (screens/settings/device_hub), on the shell in
-- ui/shell_fixture.luau over the hub of ui/device_stub.luau: gpui's rows
-- with the hub's values, the switch writing the engine and reading back,
-- versions with Check versions and Update, platform support and Refresh,
-- agent access, the floating preview's switch, and
-- device hosts: Add host's checks, Test connection's lines, Save and Remove.
-- The page leaves the list while the Device built-in is off.

local PAGE = "#settings-page"

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

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1400)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> "Device hub"')
	app:frames(30)
	return app
end
local function calls(app: any): string
	return table.concat(app:state("device_stub_calls") or {}, ",")
end
local function has(app: any, words: string): boolean
	return app:tree(PAGE):find(words, 1, true) ~= nil
end
local function checked(app: any, label: string): boolean
	for line in app:tree(PAGE):gmatch("[^\n]+") do
		if line:find(`switch "{label}`, 1, true) then
			return line:find("checked", 1, true) ~= nil and line:find("unchecked", 1, true) == nil
		end
	end
	error(`no switch {label}`)
end

test("device hub settings: gpui's rows, the hub's switch writes the engine and the page reads it back", function()
	local app = open()
	expect(calls(app)).to_contain("ListHubDevices")
	for _, w in { "Device hub", "Agent device access", "Device hosts", "No device hosts.", "Running", "Required", "Installed", "Not running", "0.12.1", "Check versions" } do
		expect(`{w}: {has(app, w)}`).to_be(`{w}: true`)
	end
	expect(has(app, "Platform support")).to_be(false)
	expect(checked(app, "Device hub")).to_be(false)
	app:click(`{PAGE} >> role:switch "Device hub"`)
	expect(eventually(app, function()
		return has(app, "Platform support")
	end)).to_be(true)
	expect(calls(app)).to_contain("ConfigureDeviceHub:enabled=true,onboardingCompleted=true")
	expect(checked(app, "Device hub")).to_be(true)
	expect(has(app, "Xcode and iOS Simulator are available.") or has(app, "Ready")).to_be(true)
	expect(has(app, "Update to v0.12.1")).to_be(true)
	app:click("#settings-device-hub-check")
	expect(eventually(app, function()
		return select(2, string.gsub(calls(app), "ListHubDevices", "")) >= 2
	end)).to_be(true)
	app:click("#settings-device-platforms-refresh")
	app:frames(5)
	app:click(`{PAGE} >> role:switch "Agent device access"`)
	expect(eventually(app, function()
		return checked(app, "Agent device access")
	end)).to_be(true)
	expect(calls(app)).to_contain("ConfigureDeviceHub:agentAccessEnabled=true")
	-- Off takes agent access with it, as gpui's does.
	app:click(`{PAGE} >> role:switch "Device hub"`)
	expect(eventually(app, function()
		return not has(app, "Platform support")
	end)).to_be(true)
	expect(calls(app)).to_contain("ConfigureDeviceHub:enabled=false,agentAccessEnabled=false")
	expect(checked(app, "Agent device access")).to_be(false)
end)

test("device hub settings: a failed update says so; Auto-show floating preview switches device_auto_float", function()
	local app = open()
	app:click(`{PAGE} >> role:switch "Device hub"`)
	expect(eventually(app, function()
		return has(app, "Update to v0.12.1")
	end)).to_be(true)
	app:command("device_stub.fail_next")
	app:click("#settings-device-hub-update")
	expect(eventually(app, function()
		return app:has("#settings-device-update-error")
	end)).to_be(true)
	expect(app:tree("#settings-device-update-error")).to_contain("Update failed. Check this host's network connection")
	expect(has(app, "Show the floating preview when an agent opens a device.")).to_be(true)
	expect(checked(app, "Auto-show floating preview")).to_be(true)
	app:click(`{PAGE} >> role:switch "Auto-show floating preview"`)
	expect(eventually(app, function()
		return app:state("device_auto_float") == false
	end)).to_be(true)
	expect(checked(app, "Auto-show floating preview")).to_be(false)
end)

test("device hosts: Add host's form checks what's typed, tests it, saves it; a host's Test connection and Remove", function()
	local app = open()
	app:click("#settings-device-host-add")
	expect(app:wait("#settings-device-host-editor", { timeout_ms = 3000 })).to_be(true)
	expect(has(app, "Hosts on the same machine are skipped.")).to_be(true)
	app:fill(`{PAGE} >> role:textbox "SSH target"`, "bad target")
	app:click("#settings-device-host-save")
	app:frames(3)
	expect(app:tree("#settings-device-host-error")).to_contain("The SSH target must be user@host or an SSH alias")
	app:fill(`{PAGE} >> role:textbox "SSH target"`, "me@mini")
	app:fill(`{PAGE} >> role:textbox "Port"`, "99999")
	app:click("#settings-device-host-save")
	app:frames(3)
	expect(app:tree("#settings-device-host-error")).to_contain("The SSH port must be between 1 and 65535.")
	app:fill(`{PAGE} >> role:textbox "Port"`, "")
	app:fill(`{PAGE} >> role:textbox "Identity file"`, "keys/id")
	app:click("#settings-device-host-save")
	app:frames(3)
	expect(app:tree("#settings-device-host-error")).to_contain("The identity file must be an absolute path.")
	app:fill(`{PAGE} >> role:textbox "Identity file"`, "")
	app:fill(`{PAGE} >> role:textbox "Name"`, "Mac mini")
	app:click("#settings-device-host-editor-test")
	expect(eventually(app, function()
		return app:has("#settings-device-host-editor >> #device-host-check")
	end)).to_be(true)
	expect(app:tree("#settings-device-host-editor")).to_contain("Xcode: Not installed")
	expect(calls(app)).to_contain("TestDeviceHost")
	app:click("#settings-device-host-save")
	expect(eventually(app, function()
		return not app:has("#settings-device-host-editor")
	end)).to_be(true)
	expect(calls(app)).to_contain("ConfigureDeviceHub:hosts=1")
	expect(has(app, "Mac mini")).to_be(true)
	expect(has(app, "me@mini")).to_be(true)
	expect(has(app, "Connection failed: Connection refused")).to_be(true)
	expect(has(app, "No device hosts.")).to_be(false)
	app:click(`{PAGE} >> role:button "Test connection"`)
	expect(eventually(app, function()
		return has(app, "SSH connection")
	end)).to_be(true)
	app:click(`{PAGE} >> role:button "Remove"`)
	expect(eventually(app, function()
		return has(app, "No device hosts.")
	end)).to_be(true)
	expect(calls(app)).to_contain("ConfigureDeviceHub:hosts=0")
end)

test("Device hub's Device panel switch is the Device built-in's; off, the page stays listed to turn it back on", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1000)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> role:tab "Device hub"')
	app:frames(30)
	local function switch(): string
		return app:tree("#settings-page"):match('switch "Device panel"[^\n]*') or ""
	end
	expect(switch()).to_contain("checked")
	expect(switch()).never.to_contain("unchecked")
	app:click('#settings-page >> role:switch "Device panel"')
	app:frames(20)
	expect(app:state("builtin_device")).to_be(false)
	expect(switch()).to_contain("unchecked")
	expect(app:has('#settings-nav >> "Device hub"')).to_be(true)
	app:click('#settings-page >> role:switch "Device panel"')
	app:frames(20)
	expect(app:state("builtin_device")).to_be(true)
end)
