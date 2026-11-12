-- The device hub's host calls (src/device.rs), through one place so a
-- fixture can answer them as the host would (ui/device_stub.luau).
--
--   api.hub                 the hub's state (the engine's DeviceHubState,
--                           camelCase), nil until heard; `{ outdated = true }`
--                           from an engine without devices
--   api.frames              { [key] = { seq?, w?, h?, failed? } }: each stream's
--                           last frame, and why the engine ended it
--   api.call(method, params) the host's promise of a hub method's answer
--   api.watch()             the hub followed while what asked is on screen:
--                           what it gives back lets go
--   api.stream(key, host, device, platform, orientation) / api.unstream(key)
--   api.orient(key, orientation)  the device turned: its picture turns too
--   api.frame_src(key)      the picture `ui.image` shows for a stream
--   api.three_d(key, on, device)  the stream drawn in 3D (gpui's ThreeD) or flat;
--                           `device` = { platform, name, host, id }
--   api.view3d(key)         the 3D view `ui.gpu_view` shows
--   api.screen3d / pointer3d (the touches the device is to be sent),
--   api.wheel3d / pinch3d (taken), api.restore3d, api.pose3d
--   api.folds               { [key] = { fold_error? } }: a hinge the hub refused

local M = {}

M.hub = ui.shared("device_hub", nil :: any)
M.frames = ui.shared("device_frames", {} :: { [string]: any })
M.folds = ui.shared("device_3d", {} :: { [string]: any })

local watchers = 0

function M.call(method: string, params: { [string]: any }?): any
	return ui.call("device.call", { method = method, params = params or {} })
end

function M.watch(): () -> ()
	watchers += 1
	if watchers == 1 then
		pcall(ui.invoke, "device.watch", true)
	end
	local done = false
	return function()
		if done then
			return
		end
		done = true
		watchers -= 1
		if watchers == 0 then
			pcall(ui.invoke, "device.watch", false)
		end
	end
end

function M.stream(key: string, host_id: string, device_id: string, platform: string?, orientation: string?)
	pcall(ui.invoke, "device.stream", { key = key, host_id = host_id, device_id = device_id, platform = platform, orientation = orientation })
end

function M.orient(key: string, orientation: string)
	pcall(ui.invoke, "device.orient", { key = key, orientation = orientation })
end

function M.unstream(key: string)
	pcall(ui.invoke, "device.unstream", key)
end

function M.frame_src(key: string): string
	return `host:device/{key}`
end

function M.three_d(key: string, on: boolean, device: { platform: string?, name: string?, host: string?, id: string? }?)
	pcall(ui.invoke, "device.three_d", {
		key = key,
		on = on,
		platform = device and device.platform,
		name = device and device.name,
		host_id = device and device.host,
		device_id = device and device.id,
	})
end

function M.view3d(key: string): string
	return `device3d/{key}`
end

local function touches(name: string, args: { [string]: any }): { any }
	local ok, out = pcall(ui.call_sync, name, args)
	return if ok and type(out) == "table" then out else {}
end

function M.screen3d(key: string, screen: any): { any }
	return touches("device.screen3d", { key = key, screen = screen })
end

function M.pointer3d(key: string, phase: string, x: number, y: number, orbit: boolean?): { any }
	return touches("device.pointer3d", { key = key, phase = phase, x = x, y = y, orbit = orbit })
end

function M.wheel3d(key: string, e: { dx: number, dy: number, x: number?, y: number?, ctrl: boolean? }): boolean
	local ok, out = pcall(ui.call_sync, "device.wheel3d", { key = key, dx = e.dx, dy = e.dy, x = e.x, y = e.y, ctrl = e.ctrl })
	return ok and out == true
end

function M.pinch3d(key: string, scale: number, x: number, y: number): boolean
	local ok, out = pcall(ui.call_sync, "device.pinch3d", { key = key, scale = scale, x = x, y = y })
	return ok and out == true
end

function M.restore3d(key: string)
	pcall(ui.invoke, "device.restore3d", key)
end

function M.pose3d(key: string, pose: string)
	pcall(ui.invoke, "device.pose3d", { key = key, pose = pose })
end

return M
