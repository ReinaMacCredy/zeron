-- What the device hub's state says, read as gpui's device panel and Device
-- hub page read it (crates/ui/src/device_panel/{mod,setup}.rs,
-- settings/device_hub.rs, shell/device_tabs.rs): a hub is the engine's
-- DeviceHubState (camelCase, as src/device.rs hands it over).

local M = {}

M.LOCAL = "local"

local function s(key: string, args: { [string]: any }?): string
	return ui.t(`device.{key}`, args):get()
end
M.s = s

export type Device = { hostId: string, id: string, platform: string, name: string, version: string?, booted: boolean? }

function M.key(host_id: string?, device_id: string): string
	if host_id == nil or host_id == "" or host_id == M.LOCAL then
		return device_id
	end
	return `{host_id}/{device_id}`
end

function M.platform_name(platform: string): string
	return s(if platform == "android" then "platform.android" else "platform.ios")
end

function M.device(hub: any, host_id: string, device_id: string): Device?
	for _, d in (hub and hub.devices) or {} do
		if (d.hostId or M.LOCAL) == host_id and d.id == device_id then
			return d
		end
	end
	return nil
end

function M.session(hub: any, chat: string, host_id: string, device_id: string): any
	for _, x in (hub and hub.sessions) or {} do
		if x.chatId == chat and (x.hostId or M.LOCAL) == host_id and x.deviceId == device_id then
			return x
		end
	end
	return nil
end

local function host_of(hub: any, id: string): any
	for _, h in (hub and hub.hosts) or {} do
		if h.id == id then
			return h
		end
	end
	return nil
end

function M.description(hub: any, d: Device): string
	local h = host_of(hub, d.hostId or M.LOCAL)
	return s("description", { host = if h then h.label else s("host_fallback"), version = d.version or "" })
end

function M.busy(hub: any): boolean
	return hub ~= nil and (hub.hostStatus == "installing" or hub.hostStatus == "starting")
end

-- gpui's `platform_setup_status`: whether it's ready, and what to say.
function M.platform_status(hub: any, platform: string): (boolean, string)
	local found = nil
	for _, h in (hub and hub.hosts) or {} do
		for _, p in h.platforms or {} do
			if not found and p.platform == platform then
				found = p
			end
		end
	end
	if not (found and found.available) then
		return false, (found and found.reason) or s("platform.not_detected", { platform = M.platform_name(platform) })
	end
	if hub.hostStatus == "ready" then
		local any = false
		for _, d in hub.devices or {} do
			any = any or d.platform == platform
		end
		if not any then
			return false, s(if platform == "ios" then "platform.ios_no_sim" else "platform.android_no_avd")
		end
	end
	return true, s(if platform == "ios" then "platform.ios_ok" else "platform.android_ok")
end

function M.local_platforms_unavailable(hub: any): boolean
	for _, h in (hub and hub.hosts) or {} do
		if h.kind == "local" and #(h.platforms or {}) > 0 then
			local any = false
			for _, p in h.platforms do
				any = any or p.available == true
			end
			if not any then
				return true
			end
		end
	end
	return false
end

function M.hub_status(hub: any, pending: boolean, compact: boolean): string?
	if pending then
		local st = hub and hub.hostStatus
		if st == "installing" then
			return s(if compact then "status.installing" else "status.installing_hub")
		elseif st == "starting" then
			return s(if compact then "status.starting" else "status.starting_hub")
		end
		return s(if compact then "status.updating" else "status.updating_hub")
	end
	return if hub and hub.hostStatus == "ready" then s("status.ready") else nil
end

function M.agent_status(hub: any, compact: boolean): string
	local st = hub and hub.agentStatus
	if st == "installing" then
		return s(if compact then "status.installing" else "status.installing_agent")
	elseif st == "starting" then
		return s(if compact then "status.starting" else "status.starting_agent")
	end
	return s(if compact then "status.updating" else "status.updating_agent")
end

function M.agent_failure(hub: any): string?
	if hub and hub.agentStatus == "failed" then
		return hub.agentStatusDetail or s("status.agent_failed")
	end
	return nil
end

-- gpui's `host_update`: the first host's label, what's happening, and whether it failed.
function M.host_update(hub: any): (string?, string?, boolean)
	if not hub then
		return nil, nil, false
	end
	local failed = hub.hostStatus == "failed"
	if not failed and not M.busy(hub) then
		return nil, nil, false
	end
	local first = (hub.hosts or {})[1]
	if not first then
		return nil, nil, false
	end
	local detail = hub.hostStatusDetail
		or s(if failed then "update.failed_detail" elseif hub.hostStatus == "installing" then "update.installing" else "update.starting")
	return first.label, detail, failed
end

-- The picker's groups: per host with devices, iOS then Android, running first then by name.
function M.picker_groups(hub: any): { { platform: string, title: string, host: string, devices: { Device } } }
	local hosts: { string } = {}
	local function has(id: string): boolean
		for _, d in (hub and hub.devices) or {} do
			if (d.hostId or M.LOCAL) == id then
				return true
			end
		end
		return false
	end
	local function add(id: string)
		if not table.find(hosts, id) and has(id) then
			table.insert(hosts, id)
		end
	end
	for _, h in (hub and hub.hosts) or {} do
		add(h.id)
	end
	for _, d in (hub and hub.devices) or {} do
		add(d.hostId or M.LOCAL)
	end
	local several = #hosts > 1
	local out = {}
	for _, id in hosts do
		for _, platform in { "ios", "android" } do
			local list: { Device } = {}
			for _, d in hub.devices or {} do
				if (d.hostId or M.LOCAL) == id and d.platform == platform then
					table.insert(list, d)
				end
			end
			if #list > 0 then
				table.sort(list, function(a: Device, b: Device): boolean
					if (a.booted == true) ~= (b.booted == true) then
						return a.booted == true
					end
					return a.name < b.name
				end)
				local title = s(if platform == "ios" then "picker.ios" else "picker.android")
				local h = host_of(hub, id)
				if several and h then
					title = s("picker.host_group", { host = h.label, title = title })
				end
				table.insert(out, { platform = platform, title = title, host = id, devices = list })
			end
		end
	end
	return out
end

function M.host_notes(hub: any): { { id: string, text: string } }
	local out = {}
	for _, h in (hub and hub.hosts) or {} do
		if h.kind == "ssh" then
			local st = (hub.hostStatuses or {})[h.id]
			local text = nil
			if st and st.status == "installing" then
				text = s("host_note.installing")
			elseif st and st.status == "starting" then
				text = st.detail or s("host_note.connecting")
			elseif st and st.status == "failed" then
				text = if st.detail then s("host_note.failed", { detail = st.detail }) else s("host_note.failed_plain")
			end
			if text then
				table.insert(out, { id = h.id, text = s("host_note.line", { host = h.label, text = text }) })
			end
		end
	end
	return out
end

function M.booting_banner(hub: any, chat: string): string?
	local names = {}
	for _, b in (hub and hub.booting) or {} do
		if b.chatId == chat then
			table.insert(names, b.name)
		end
	end
	return if #names > 0 then s("booting", { names = table.concat(names, ", ") }) else nil
end

export type Phase = { kind: string, name: string?, description: string?, message: string? }

-- gpui's `panel_phase`: setup, loading (with what it says), the stream, or the picker.
function M.phase(hub: any, chat: string, target: { string }?, opening: string?): Phase
	local function loading(name: string, description: string, message: string): Phase
		return { kind = "loading", name = name, description = description, message = message }
	end
	if not hub then
		return loading(s("loading.devices"), "", s("loading.finding"))
	end
	local settings = hub.settings or {}
	if not settings.onboardingCompleted or hub.hostStatus == "disabled" or hub.hostStatus == nil then
		return { kind = "setup" }
	end
	if target and M.session(hub, chat, target[1], target[2]) then
		return { kind = "stream" }
	end
	if opening then
		for _, d in hub.devices or {} do
			if M.key(d.hostId, d.id) == opening then
				return loading(d.name, M.description(hub, d), s(if d.booted then "loading.opening" else "loading.starting"))
			end
		end
		return loading(s("loading.devices"), "", s("loading.opening"))
	end
	if hub.hostStatus == "installing" then
		return loading(s("loading.devices"), "", hub.hostStatusDetail or s("loading.installing"))
	elseif hub.hostStatus == "starting" then
		return loading(s("loading.devices"), "", s("loading.finding"))
	end
	return { kind = "picker" }
end

-- Settings → Device hub's versions (the local host's hub tool), and the version to update to.
function M.version_rows(hub: any): { { label: string, value: string } }?
	local tool = nil
	for _, h in (hub and hub.hosts) or {} do
		if h.kind == "local" and h.tools and h.tools.hub then
			tool = h.tools.hub
			break
		end
	end
	if not tool then
		return nil
	end
	local installed = if #(tool.installed or {}) == 0 then ui.t("settings.device_hub.versions.none"):get() else table.concat(tool.installed, ", ")
	return {
		{ label = ui.t("settings.device_hub.versions.running"):get(), value = tool.running or ui.t("settings.device_hub.versions.not_running"):get() },
		{ label = ui.t("settings.device_hub.versions.required"):get(), value = tool.required or "" },
		{ label = ui.t("settings.device_hub.versions.installed"):get(), value = installed },
	}
end

function M.needs_update(hub: any): string?
	for _, h in (hub and hub.hosts) or {} do
		if h.kind == "local" and h.tools and h.tools.hub then
			local tool = h.tools.hub
			return if table.find(tool.installed or {}, tool.required) then nil else tool.required
		end
	end
	return nil
end

-- An SSH host as the Add host form has it, checked as the engine checks one
-- (zeron_proto `SshDeviceHostConfig::validate`): the host, or what's wrong.
function M.draft(id: string, name: string, target: string, identity: string, port: string): (any, string?)
	local function e(key: string): string
		return ui.t(`settings.device_hub.errors.{key}`):get()
	end
	name, target, identity, port = string.gsub(name, "^%s*(.-)%s*$", "%1"), string.gsub(target, "^%s*(.-)%s*$", "%1"), string.gsub(identity, "^%s*(.-)%s*$", "%1"), string.gsub(port, "^%s*(.-)%s*$", "%1")
	local n = nil
	if port ~= "" then
		n = tonumber(port)
		if not n or n % 1 ~= 0 or n < 1 or n > 65535 or not string.match(port, "^%d+$") then
			return nil, e("port")
		end
	end
	local host = { id = id, label = if name == "" then target else name, target = target, identityFile = if identity ~= "" then identity else nil, port = n }
	if #id > 128 or not string.match(id, "^[%w][%w_%-]*$") then
		return nil, e("id")
	end
	if id == M.LOCAL then
		return nil, e("reserved")
	end
	if host.label == "" or string.find(host.label, "%c") then
		return nil, e("name")
	end
	if target == "" or string.find(target, "[%s%c]") or string.sub(target, 1, 1) == "-" then
		return nil, e("target")
	end
	if host.identityFile and (string.sub(host.identityFile, 1, 1) ~= "/" or string.find(host.identityFile, "%c")) then
		return nil, e("identity")
	end
	return host, nil
end

-- gpui's `device_surface_unavailable`: why a Device tab can't open here, or nil.
function M.unavailable(chat: any, hub: any): string?
	if chat == nil or chat == "" then
		return s("unavailable.thread")
	end
	if type(chat) == "table" and chat.device and chat.here == false then
		return s("unavailable.computer")
	end
	if hub and hub.outdated then
		return s("unavailable.engine")
	end
	return nil
end

return M
