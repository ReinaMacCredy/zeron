-- The device hub, answered here as src/device.rs's mock hub answers a mock
-- Dev (no engine in a test host): screens/device/api's calls, its watch
-- and its frames, and Settings' `settings.engine` for the hub's methods.
-- Required by ui/settings_stub.luau (the shell's fixture) and
-- ui/panel_fixture.luau; tests/device_hub.test.luau and
-- tests/device_tab.test.luau drive it.
--
-- What was asked, in order, is `device_stub_calls` (a state). `ui.command`s
-- change the hub as the engine would on its own:
--   device_stub.fail_next    the next call fails ("The device hub failed to start.")
--   device_stub.old_engine   the engine has no devices ({ outdated = true })
--   device_stub.frame        the next frame lands on every stream
--   device_stub.installing   the hub is installing (busy)
--   device_stub.ready        the hub is ready again

local api = require("@app/screens/device/api") :: any
local settings_host = require("@app/screens/settings/host") :: any

local calls = ui.state("device_stub_calls", {} :: { string })
local fail = ui.state("device_stub_fail", false)
local function log(entry: string)
	calls:push(entry)
end

local function settled(value: any, word: string?): any
	local p = { done = true }
	function p.after(_self: any, ok: ((any) -> ())?, bad: ((any) -> ())?)
		if word then
			if bad then
				bad({ message = word })
			end
		elseif ok then
			ok(value)
		end
	end
	return p
end

local DEVICES = {
	{ id = "mock-iphone", platform = "ios", name = "iPhone 17", version = "iOS 27.0" },
	{ id = "mock-ipad", platform = "ios", name = "iPad Air", version = "iOS 27.0" },
	{ id = "Pixel_9", platform = "android", name = "Pixel 9", version = "Android 16" },
}
local hub = {
	revision = 1,
	enabled = false,
	agent = false,
	onboarded = false,
	status = nil :: string?,
	hosts = {} :: { any },
	booted = { ["mock-iphone"] = true } :: { [string]: boolean },
	sessions = {} :: { any },
	outdated = false,
}

local function state(): any
	if hub.outdated then
		return { outdated = true }
	end
	local devices = {}
	if hub.enabled then
		for _, d in DEVICES do
			table.insert(devices, { hostId = "local", id = d.id, platform = d.platform, name = d.name, version = d.version, booted = hub.booted[d.id] == true, physical = false })
		end
	end
	local hosts = {
		{ id = "local", kind = "local", label = "This machine", hubInstalled = true,
			platforms = { { platform = "ios", available = true }, { platform = "android", available = true } },
			tools = { hub = { required = "0.12.1", installed = { "0.12.0" }, running = if hub.enabled then "0.12.0" else nil } } },
	}
	local statuses = {}
	for _, h in hub.hosts do
		table.insert(hosts, { id = h.id, kind = "ssh", label = h.label, platforms = { { platform = "ios", available = true } }, hubInstalled = false })
		statuses[h.id] = { status = "failed", detail = "Connection refused" }
	end
	local screens, streams = {}, {}
	for _, x in hub.sessions do
		screens[x.deviceId] = { width = 196, height = 426, orientation = "portrait" }
		streams[x.deviceId] = { video = "live", input = "live", subscribers = 1 }
	end
	return {
		revision = hub.revision,
		settings = { enabled = hub.enabled, agentAccessEnabled = hub.agent, onboardingCompleted = hub.onboarded, deviceHosts = table.clone(hub.hosts) },
		hosts = hosts,
		hostStatus = hub.status or (if hub.enabled then "ready" else "disabled"),
		hostStatuses = statuses,
		devices = devices,
		sessions = table.clone(hub.sessions),
		booting = {},
		streams = streams,
		screens = screens,
		agentStatus = if hub.agent then "ready" else "disabled",
	}
end

local watching = 0
local function publish()
	if watching > 0 then
		api.hub:set(state())
	end
end

local function answer(method: string, p: any): any
	hub.revision += 1
	if fail:peek() then
		fail:set(false)
		return settled(nil, "The device hub failed to start.")
	end
	if hub.outdated then
		return settled(nil, "outdated")
	end
	if method == "ListHubDevices" then
		return settled(state())
	elseif method == "ConfigureDeviceHub" then
		if p.deviceHosts ~= nil then
			hub.hosts = table.clone(p.deviceHosts)
		end
		if p.enabled ~= nil then
			hub.enabled = p.enabled == true
			if not hub.enabled then
				hub.sessions = {}
			end
		end
		if p.agentAccessEnabled ~= nil then
			hub.agent = p.agentAccessEnabled == true and hub.enabled
		end
		if p.onboardingCompleted ~= nil then
			hub.onboarded = p.onboardingCompleted == true
		end
		return settled(state())
	elseif method == "TestDeviceHost" then
		return settled({ ["local"] = false, checks = { { name = "SSH connection", ok = true }, { name = "Xcode", ok = false, detail = "Not installed" } } })
	elseif method == "OpenHubDevice" then
		if not hub.enabled then
			return settled(nil, "The device hub is off.")
		end
		hub.booted[p.deviceId] = true
		local s = { chatId = p.chatId, hostId = p.hostId or "local", deviceId = p.deviceId, platform = p.platform, openedAt = "2026-11-09T00:00:00Z", openedBy = "user" }
		table.insert(hub.sessions, s)
		return settled(s)
	elseif method == "CloseHubDevice" then
		local keep = {}
		for _, x in hub.sessions do
			if not (x.chatId == p.chatId and (p.deviceId == nil or x.deviceId == p.deviceId)) then
				table.insert(keep, x)
			end
		end
		hub.sessions = keep
		if p.shutdown and p.deviceId then
			hub.booted[p.deviceId] = nil
		end
		return settled({})
	elseif method == "HubDeviceScreenshot" then
		return settled({ path = "/tmp/zeron-mock-device-screenshot.png" })
	elseif method == "SendHubDeviceInput" then
		return settled({})
	end
	return settled(nil, "refused")
end

local function detail(method: string, p: any): string
	if method == "SendHubDeviceInput" and type(p.input) == "table" then
		for kind, v in p.input do
			if kind == "touch" then
				return `{method}:touch:{v.phase}:{string.format("%.2f", v.x)}:{string.format("%.2f", v.y)}`
			elseif kind == "button" then
				return `{method}:button:{v.button}`
			elseif kind == "key" then
				return `{method}:key:{v.phase}:{v.usage}`
			elseif kind == "text" then
				return `{method}:text:{v.text}`
			elseif kind == "keycode" then
				return `{method}:keycode:{v.keycode}`
			end
		end
	end
	if method == "ConfigureDeviceHub" then
		local parts = {}
		for _, k in { "enabled", "agentAccessEnabled", "onboardingCompleted" } do
			if p[k] ~= nil then
				table.insert(parts, `{k}={tostring(p[k])}`)
			end
		end
		if p.deviceHosts ~= nil then
			table.insert(parts, `hosts={#p.deviceHosts}`)
		end
		return `{method}:{table.concat(parts, ",")}`
	end
	if p.deviceId then
		return `{method}:{p.deviceId}{if p.shutdown then ":shutdown" else ""}`
	end
	return method
end

local function hub_call(method: string, params: any?): any
	local p = params or {}
	log(detail(method, p))
	local r = answer(method, p)
	publish()
	return r
end

api.call = hub_call
api.watch = function(): () -> ()
	watching += 1
	log("watch")
	api.hub:set(state())
	local done = false
	return function()
		if not done then
			done = true
			watching -= 1
		end
	end
end
api.stream = function(key: string, _host: string, device: string)
	log(`stream:{device}`)
	local all = table.clone(api.frames:peek())
	all[key] = { seq = 1, w = 196, h = 426 }
	api.frames:set(all)
end
api.unstream = function(key: string)
	local all = table.clone(api.frames:peek())
	if all[key] then
		log("unstream")
		all[key] = nil
		api.frames:set(all)
	end
end
api.frame_src = function(key: string): string
	local f = api.frames:get()[key]
	local n = if f then ((f.seq - 1) % 3) + 1 else 1
	local src = `assets/device/frame-{n}.png`
	log(`src:{src}`)
	return src
end

local HUB_METHODS = { ListHubDevices = true, ConfigureDeviceHub = true, TestDeviceHost = true }
local before = settings_host.call
settings_host.call = function(name: string, arg: any?): any
	if name == "settings.engine" and type(arg) == "table" and HUB_METHODS[arg.method] then
		return hub_call(arg.method, arg.params)
	end
	return before(name, arg)
end

ui.command {
	id = "device_stub.fail_next",
	title = "Fail the next device call",
	run = function()
		fail:set(true)
	end,
}
ui.command {
	id = "device_stub.old_engine",
	title = "An engine without devices",
	run = function()
		hub.outdated = true
		api.hub:set(state())
	end,
}
ui.command {
	id = "device_stub.frame",
	title = "The next frame",
	run = function()
		local all = table.clone(api.frames:peek())
		for k, f in all do
			all[k] = { seq = f.seq + 1, w = f.w, h = f.h }
		end
		api.frames:set(all)
	end,
}
ui.command {
	id = "device_stub.installing",
	title = "The hub is installing",
	run = function()
		hub.status = "installing"
		api.hub:set(state())
	end,
}
ui.command {
	id = "device_stub.ready",
	title = "The hub is ready",
	run = function()
		hub.status = nil
		api.hub:set(state())
	end,
}
-- The Device built-in (Settings → Plugins), as its switch sets it.
for _, which in { "on", "off" } do
	ui.command {
		id = `device_stub.builtin_{which}`,
		title = `Device built-in {which}`,
		run = function()
			require("@app/screens/plugins/builtins").set("device", which == "on")
		end,
	}
end

return {}
