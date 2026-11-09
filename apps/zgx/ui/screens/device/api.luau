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
--   api.stream(key, host, device) / api.unstream(key)
--   api.frame_src(key)      the picture `ui.image` shows for a stream

local M = {}

M.hub = ui.shared("device_hub", nil :: any)
M.frames = ui.shared("device_frames", {} :: { [string]: any })

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

function M.stream(key: string, host_id: string, device_id: string)
	pcall(ui.invoke, "device.stream", { key = key, host_id = host_id, device_id = device_id })
end

function M.unstream(key: string)
	pcall(ui.invoke, "device.unstream", key)
end

function M.frame_src(key: string): string
	return `host:device/{key}`
end

return M
