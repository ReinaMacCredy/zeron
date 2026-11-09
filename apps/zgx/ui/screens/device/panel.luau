-- A chat's Device tab: gpui's device panel (crates/ui/src/device_panel)
-- over the engine's hub (screens/device/api): the setup wizard until the
-- hub is on and set up, the picker of simulators and emulators (by host and
-- platform, Running or Stopped, Open or Start), the loading steps while one
-- opens, then its screen: the device's frames as a picture, touch, keys,
-- Home (Back on Android), Save screenshot and More (Close panel, Power off).
--
--   Device.Tab { panel_key, tab, visible? }   a device tab's body; its
--                                   screen streams only while `visible()`
--                                   (the panel open), as gpui's `set_visible`
--   Device.title(tab)               its name: Rename's, the device's, "Device"
--   Device.icon(tab)                apple / android / smartphone
--   Device.detail(tab)              its tooltip: the name and the device's line
--   Device.unavailable(session)     why a Device tab can't open here, or nil
--   Device.close(tab, panel_key?)   its stream let go (the device stays open,
--                                   as gpui's Close panel leaves it)
--   Device.shows(tab, host, id)     whether a tab shows that device (or opens it)
--   Device.idle(tab)                on its picker, opening nothing
--   Device.dismissed(chat, host, id) its tab closed: not opened again by itself
--   Device.follow(open)             devices opened in a chat elsewhere, to open
--                                   a tab for (gpui's `auto_open_devices`)
--
-- The tab keeps its device and its name (screens/panel/state's tab), so a
-- reload shows the same. What it opens, tries and fails is its own while it
-- shows. Idle on a still screen: no frame asks for another.

local ds = require("@ds")
local api = require("@app/screens/device/api")
local model = require("@app/screens/device/model")
local Screen = require("@app/screens/device/screen")
local Float = require("@app/screens/device/float")
local state = require("@app/screens/panel/state")
local Icon = require("@ds/icon")
local IconButton = require("@ds/icon_button").IconButton
local ContextMenu = require("@ds/context_menu")
local Spinner = require("@ds/status").Spinner

local M = {}

local s = model.s
local TOAST_MS = 4000

local function chat_of(panel_key: string): string
	return if string.sub(panel_key, 1, 5) == "chat:" then string.sub(panel_key, 6) else ""
end

local function target_of(tab: any): { string }?
	local t = tab and tab.device_target
	return if t and t.id then { t.host or model.LOCAL, t.id } else nil
end

-- The tab as it is now (its device, its name), followed: the strip holds the one it was made with.
local function live(tab: any): any
	for _, sess in state.sessions:get() do
		for _, x in sess.tabs do
			if x.id == tab.id then
				return x
			end
		end
	end
	return tab
end

local function device_of(tab: any): model.Device?
	local t = target_of(live(tab))
	return if t then model.device(api.hub:get(), t[1], t[2]) else nil
end

-- Whether the tab shows a device now (Rename is for one that does).
function M.bound(tab: any): boolean
	return target_of(live(tab)) ~= nil
end

function M.title(tab: any): string
	tab = live(tab)
	if tab.title and tab.title ~= "" then
		return tab.title
	end
	local d = device_of(tab)
	return if d then d.name else s("tab")
end

-- (gpui's Apple and Android glyphs: zgx's set has neither, so the phone for all.)
function M.icon(_tab: any): string
	return "smartphone"
end

function M.detail(tab: any): string?
	local d = device_of(tab)
	return if d then `{M.title(tab)}\n{model.description(api.hub:get(), d)}` else nil
end

-- Why a Device tab can't open for this chat (its id), or nil: gpui's
-- `device_surface_unavailable`. Its computer is the one in the sidebar's
-- list; a chat whose computer isn't known yet counts as this one's.
function M.unavailable(session: any): string?
	local sid = if type(session) == "table" then session.id else session
	if type(sid) ~= "string" or sid == "" then
		return model.unavailable(nil, api.hub:get())
	end
	local chat: any = sid
	local ok, sessions = pcall(require, "@account/sessions")
	local now = if ok then (sessions :: any).now:get() else nil
	if now and now.here then
		for _, c in now.chats or {} do
			if c.id == sid then
				chat = { id = sid, device = c.device, here = c.device == nil or c.device == now.here }
			end
		end
	end
	return model.unavailable(chat, api.hub:get())
end

-- Each open tab's runtime: what it's opening, its step, its words; gone with the tab.
type Run = {
	opening: State<string?>,
	pending: State<string?>,
	step: State<number>,
	powering_off: State<boolean>,
	error: State<string?>,
	toast: State<string?>,
	shooting: State<boolean>,
	-- The device it was bound to, once the hub listed it open (gpui's `seen_live`).
	seen_live: string?,
}
local runs: { [string]: Run } = {}
local function run_of(id: string): Run
	local r = runs[id]
	if not r then
		r = {
			opening = ui.state(`device_tab_opening_{id}`, nil :: string?),
			pending = ui.state(`device_tab_pending_{id}`, nil :: string?),
			step = ui.state(`device_tab_step_{id}`, 0),
			powering_off = ui.state(`device_tab_off_{id}`, false),
			error = ui.state(`device_tab_error_{id}`, nil :: string?),
			toast = ui.state(`device_tab_toast_{id}`, nil :: string?),
			shooting = ui.state(`device_tab_shooting_{id}`, false),
		}
		runs[id] = r
	end
	return r
end

-- Whether a tab shows this device, or is opening it.
function M.shows(tab: any, host: string, id: string): boolean
	local t = target_of(live(tab))
	if t and t[1] == host and t[2] == id then
		return true
	end
	local r = runs[tab.id]
	return r ~= nil and r.opening:peek() == model.key(host, id)
end

-- A device opened in a chat elsewhere (an agent's, another window's), followed
-- while the Device built-in is on, as gpui's `auto_open_devices`: each one
-- new since the hub was first heard is handed to `open` (chat, host, device).
function M.follow(open: (chat: string, host: string, id: string) -> ())
	local Builtins = require("@app/screens/plugins/builtins")
	local stop: (() -> ())? = nil
	local seen: { [string]: boolean }? = nil
	ui.effect(function()
		local on = Builtins.on("device")
		if on and not stop then
			stop = api.watch()
		elseif not on and stop then
			stop()
			stop = nil
			seen = nil
		end
	end)
	ui.effect(function()
		local hub = api.hub:get()
		if stop == nil or type(hub) ~= "table" or hub.outdated then
			return
		end
		local now: { [string]: boolean } = {}
		local fresh = {}
		for _, x in hub.sessions or {} do
			local host = x.hostId or model.LOCAL
			if type(x.chatId) == "string" and model.device(hub, host, x.deviceId) then
				local k = `{x.chatId}\0{model.key(host, x.deviceId)}`
				now[k] = true
				if seen and not seen[k] then
					table.insert(fresh, { x.chatId, host, x.deviceId })
				end
			end
		end
		local first = seen == nil
		seen = now
		if not first and #fresh > 0 then
			-- (Out of this effect: what opening reads isn't what it follows.)
			ui.after(0, function()
				for _, f in fresh do
					open(f[1], f[2], f[3])
				end
			end)
		end
	end)
end

local words, call, target_params = Screen.words, Screen.call, Screen.target_params

-- Devices whose tab was closed, by chat: an agent opening one again opens no tab for it (gpui's `dismissed`).
local dismissed: { [string]: boolean } = {}
local function dismissal(chat: string, host: string, id: string): string
	return `{chat}\0{model.key(host, id)}`
end

function M.close(tab: any, panel_key: string?)
	local t = target_of(live(tab))
	if t then
		api.unstream(tab.id)
		if panel_key then
			dismissed[dismissal(chat_of(panel_key), t[1], t[2])] = true
		end
	end
	runs[tab.id] = nil
end

function M.dismissed(chat: string, host: string, id: string): boolean
	return dismissed[dismissal(chat, host, id)] == true
end

function M.undismiss(chat: string, host: string, id: string)
	dismissed[dismissal(chat, host, id)] = nil
end

-- A Device tab on its picker, opening nothing: where a device opened from elsewhere goes.
function M.idle(tab: any): boolean
	local r = runs[tab.id]
	return target_of(live(tab)) == nil and not (r and r.opening:peek() ~= nil)
end

-- Small words, in the quiet ink (or the danger's).
local function small(text: any, danger: boolean?, id: string?): Node
	return ui.text {
		text,
		id = id,
		wrap = true,
		text_size = "$font.size.sm",
		color = if danger then "$zeron.tone.danger.ink" else "$text.secondary",
	}
end

local function heading(text: any): Node
	return ui.text { text, text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" }
end

local function platform_card(hub: () -> any, platform: string, compact: boolean): Node
	return ui.row {
		id = `device-platform-{platform}`,
		gap = "$space.2",
		items = "start",
		w_full = true,
		p = if compact then nil else "$space.2",
		rounded = if compact then nil else "$radius.md",
		border = if compact then nil else 1,
		border_color = if compact then nil else "$border",
		ui.presence(function(): string
			local ready = model.platform_status(hub(), platform)
			return if ready then "ready" else "not"
		end, function(which: string): Node
			return Icon { name = if which == "ready" then "check-circle" else "info-circle", size = 16, color = if which == "ready" then "$success" else "$text.secondary" }
		end, { shrink = 0 }),
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text { model.platform_name(platform), text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
			small(function(): string
				local ready, why = model.platform_status(hub(), platform)
				return if compact and ready then s("platform.ready") else why
			end),
		},
	}
end
M.platform_card = platform_card

local function button(id: string, label: any, enabled: () -> boolean, run: () -> (), tone: string?): Node
	return ds.Button {
		id = id,
		tone = (tone or "secondary") :: any,
		size = "sm",
		label = label,
		disabled = function(): boolean
			return not enabled()
		end,
		on_click = run,
		label,
	}
end

-- The host's update or failure, with Retry once it failed (gpui's `host_update_banner`).
local function host_update(hub: () -> any, retry: () -> ()): Node
	return ui.show(function(): boolean
		local label = model.host_update(hub())
		return label ~= nil
	end, ui.row {
		id = "device-host-update",
		role = "status",
		w_full = true,
		gap = "$space.3",
		items = "start",
		rounded = "$radius.md",
		border = 1,
		border_color = "$border",
		p = "$space.2",
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text {
				function(): string
					local label = model.host_update(hub())
					return label or ""
				end,
				text_size = "$font.size.sm",
				weight = "$font.weight.medium",
				color = "$text.primary",
			},
			small(function(): string
				local _, detail = model.host_update(hub())
				return detail or ""
			end),
			ui.show(function(): boolean
				local _, _, failed = model.host_update(hub())
				return failed
			end, small(ui.t("device.update.failed_note"))),
		},
		ui.show(function(): boolean
			local _, _, failed = model.host_update(hub())
			return failed
		end, button("device-retry", ui.t("device.update.retry"), function(): boolean
			return true
		end, retry)),
	})
end
M.host_update = host_update

local function refresh(r: Run)
	call("ListHubDevices", {}, nil, function(why: string)
		r.error:set(why)
	end)
end

-- gpui's setup wizard: three steps, each reachable back once passed, its footer's Cancel or Back and Continue or Done.
local function setup(props: { tab: any, panel_key: string, r: Run }): Node
	local r = props.r
	local function hub(): any
		return api.hub:get() or {}
	end
	local function enabled(): boolean
		return hub().hostStatus ~= nil and hub().hostStatus ~= "disabled"
	end
	local function idle(): boolean
		return r.pending:get() == nil
	end
	local function configure(pending: string, params: { [string]: any })
		r.pending:set(pending)
		r.error:set(nil)
		call("ConfigureDeviceHub", params, function()
			r.pending:set(nil)
		end, function(why: string)
			r.pending:set(nil)
			r.error:set(why)
		end)
	end
	local hub_on = ui.state(`device_setup_hub_{props.tab.id}`, enabled())
	local agent_on = ui.state(`device_setup_agent_{props.tab.id}`, (hub().settings or {}).agentAccessEnabled == true)
	ui.effect(function()
		local on = enabled()
		if hub_on:peek() ~= on then
			hub_on:set(on)
		end
		local a = (hub().settings or {}).agentAccessEnabled == true
		if agent_on:peek() ~= a then
			agent_on:set(a)
		end
	end)
	local labels = { ui.t("device.setup.step.hub"), ui.t("device.setup.step.simulators"), ui.t("device.setup.step.agent") }
	local steps: { Node } = {}
	for i, label in labels do
		local index = i - 1
		table.insert(steps, ui.column {
			id = `device-setup-step-{index}`,
			role = "tab",
			label = label,
			selected = function(): boolean
				return r.step:get() == index
			end,
			grow = 1,
			gap = "$space.1",
			cursor = function(): string
				return if index < r.step:get() and idle() and not model.busy(hub()) then "pointer" else "default"
			end,
			on_click = function()
				if index < r.step:peek() and idle() and not model.busy(hub()) then
					r.step:set(index)
				end
			end,
			ui.div {
				h = 3,
				w_full = true,
				rounded = "$radius.full",
				bg = function(): string
					return if index <= r.step:get() then "$text.secondary" else "$border"
				end,
			},
			ui.text {
				label,
				text_size = "$font.size.xs",
				color = function(): string
					return if r.step:get() == index then "$text.primary" else "$text.secondary"
				end,
			},
		})
	end
	local step0 = ui.column {
		gap = "$space.3",
		w_full = true,
		heading(ui.t("device.setup.hub_heading")),
		ui.row {
			gap = "$space.4",
			items = "start",
			w_full = true,
			ui.column { grow = 1, min_w = 0, small(ui.t("device.setup.hub_body")) },
			ui.div {
				id = "device-hub-enable",
				opacity = function(): number
					return if idle() and not model.busy(hub()) then 1 else 0.5
				end,
				pointer_events = function(): string
					return if idle() and not model.busy(hub()) then "auto" else "none"
				end,
				ds.Switch {
					value = hub_on,
					label = ui.t("device.setup.hub_switch"),
					on_change = function(on: boolean)
						configure("hub", if on then { enabled = true } else { enabled = false, agentAccessEnabled = false })
					end,
				},
			},
		},
		ui.presence(function(): string
			local pending = r.pending:get() == "hub" or (model.busy(hub()) and r.pending:get() ~= "agent")
			return model.hub_status(hub(), pending, false) or ""
		end, function(text: string): Node
			if text == "" then
				return ui.div {}
			end
			local spinning = r.pending:peek() == "hub" or model.busy(api.hub:peek())
			return ui.row {
				id = "device-setup-status",
				role = "status",
				gap = "$space.2",
				items = "center",
				if spinning then Spinner { size = 12 } else Icon { name = "check", size = 12, color = "$success" },
				small(text),
			}
		end),
	}
	local check = ui.column {
		gap = "$space.3",
		w_full = true,
		heading(ui.t("device.setup.check_heading")),
		platform_card(hub, "ios", false),
		platform_card(hub, "android", false),
		small(ui.t("device.setup.check_note")),
		ui.row {
			button("device-setup-check", function(): string
				return s(if r.pending:get() == "check" then "setup.checking" else "setup.check_again")
			end, function(): boolean
				return enabled() and idle() and not model.busy(hub())
			end, function()
				r.pending:set("check")
				call("ListHubDevices", {}, function()
					r.pending:set(nil)
				end, function(why: string)
					r.pending:set(nil)
					r.error:set(why)
				end)
			end),
		},
	}
	local step2 = ui.column {
		gap = "$space.3",
		w_full = true,
		heading(ui.t("device.setup.agent_heading")),
		ui.row {
			gap = "$space.4",
			items = "start",
			w_full = true,
			ui.column { grow = 1, min_w = 0, small(ui.t("device.setup.agent_body")) },
			ui.div {
				id = "device-agent-access",
				opacity = function(): number
					return if enabled() and idle() and not model.busy(hub()) then 1 else 0.5
				end,
				pointer_events = function(): string
					return if enabled() and idle() and not model.busy(hub()) then "auto" else "none"
				end,
				ds.Switch {
					value = agent_on,
					label = ui.t("device.setup.agent_switch"),
					on_change = function(on: boolean)
						configure("agent", { agentAccessEnabled = on })
					end,
				},
			},
		},
		ui.show(function(): boolean
			return r.pending:get() == "agent"
		end, ui.row { gap = "$space.2", items = "center", Spinner { size = 12 }, small(function(): string
			return model.agent_status(hub(), false)
		end) }),
		ui.show(function(): boolean
			return model.agent_failure(hub()) ~= nil
		end, small(function(): string
			return model.agent_failure(hub()) or ""
		end, true, "device-agent-failed")),
		small(ui.t("device.setup.agent_note")),
	}
	local function ready(): boolean
		return hub().hostStatus == "ready"
	end
	return ui.scroll {
		id = "device-setup",
		w_full = true,
		grow = 1,
		min_h = 0,
		items = "center",
		ui.column {
			id = "device-setup-card",
			w_full = true,
			max_w = 440,
			my = "$space.4",
			mx = "$space.4",
			rounded = "$radius.lg",
			border = 1,
			border_color = "$border",
			bg = "$surface.raised",
			ui.column {
				p = "$space.4",
				gap = "$space.2",
				w_full = true,
				ui.text { ui.t("device.setup.title"), text_size = "$font.size.lg", weight = "$font.weight.semibold", color = "$text.primary" },
				small(ui.t("device.setup.body")),
				ui.row { id = "device-setup-steps", role = "tablist", label = ui.t("device.setup.steps_label"), mt = "$space.2", gap = "$space.2", w_full = true, table.unpack(steps) },
			},
			ui.column {
				px = "$space.4",
				pb = "$space.4",
				gap = "$space.3",
				w_full = true,
				host_update(hub, function()
					refresh(r)
				end),
				ui.show(function(): boolean
					return r.step:get() == 0
				end, step0),
				ui.show(function(): boolean
					return r.step:get() == 1 or (r.step:get() == 0 and enabled() and model.local_platforms_unavailable(hub()))
				end, check),
				ui.show(function(): boolean
					return r.step:get() == 2
				end, step2),
				ui.show(function(): boolean
					return hub().hostStatus == "failed" and hub().hostStatusDetail ~= nil
				end, small(function(): string
					return hub().hostStatusDetail or ""
				end, true, "device-setup-failed")),
			},
			ui.row {
				border_sides = { 1, 0, 0, 0 },
				border_color = "$border",
				px = "$space.4",
				py = "$space.3",
				gap = "$space.2",
				justify = "end",
				w_full = true,
				ui.presence(function(): string
					return if r.step:get() == 0 then "cancel" else "back"
				end, function(which: string): Node
					if which == "cancel" then
						return button("device-setup-cancel", ui.t("device.setup.cancel"), function(): boolean
							return true
						end, function()
							state.close(props.panel_key, props.tab.id)
							M.close(props.tab, props.panel_key)
						end)
					end
					return button("device-setup-back", ui.t("device.setup.back"), function(): boolean
						return idle() and not model.busy(hub())
					end, function()
						r.step:set(math.max(0, r.step:peek() - 1))
					end)
				end),
				ui.presence(function(): string
					return if r.step:get() < 2 then "continue" else "done"
				end, function(which: string): Node
					if which == "continue" then
						return button("device-setup-continue", ui.t("device.setup.continue"), function(): boolean
							return ready() and idle()
						end, function()
							r.step:set(r.step:peek() + 1)
						end, "primary")
					end
					return button("device-setup-done", function(): string
						return s(if r.pending:get() == "complete" then "setup.saving" else "setup.done")
					end, function(): boolean
						return ready() and idle()
					end, function()
						configure("complete", { onboardingCompleted = true })
					end, "primary")
				end),
			},
		},
	}
end

-- gpui's loading screen: the device's tile, its name and line, what's happening, two steps.
local function loading(phase: () -> model.Phase, stage: () -> number): Node
	local function segment(n: number): Node
		return ui.div {
			h = 4,
			grow = 1,
			rounded = "$radius.full",
			bg = function(): string
				return if stage() >= n then "$text.secondary" else "$border"
			end,
		}
	end
	return ui.column {
		id = "device-loading",
		role = "status",
		w_full = true,
		grow = 1,
		items = "center",
		justify = "center",
		px = "$space.6",
		ui.column {
			max_w = 384,
			w_full = true,
			items = "center",
			gap = "$space.3",
			ui.div {
				w = 48,
				h = 48,
				rounded = "$radius.lg",
				border = 1,
				border_color = "$border",
				items = "center",
				justify = "center",
				Icon { name = "smartphone", size = 24, color = "$text.secondary" },
			},
			ui.text { function(): string
				return phase().name or ""
			end, text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
			ui.show(function(): boolean
				return (phase().description or "") ~= ""
			end, small(function(): string
				return phase().description or ""
			end)),
			ui.row { gap = "$space.2", items = "center", Spinner { size = 12 }, small(function(): string
				return phase().message or ""
			end) },
			ui.row { w = 160, gap = "$space.1", segment(1), segment(2) },
			small(function(): string
				return s(if stage() >= 2 then "loading.step2" else "loading.step1")
			end),
		},
	}
end

local function picker(props: { tab: any, panel_key: string, r: Run }): Node
	local r = props.r
	local chat = chat_of(props.panel_key)
	local function hub(): any
		return api.hub:get() or {}
	end
	local function ready(): boolean
		local h = hub()
		if h.hostStatus == "ready" then
			return true
		end
		for _, st in h.hostStatuses or {} do
			if st.status == "ready" then
				return true
			end
		end
		return false
	end
	local function open(d: model.Device)
		if r.opening:peek() then
			return
		end
		r.opening:set(model.key(d.hostId, d.id))
		r.error:set(nil)
		call("OpenHubDevice", { chatId = chat, hostId = d.hostId or model.LOCAL, deviceId = d.id, platform = d.platform }, function(session: any)
			r.opening:set(nil)
			local host = (session and session.hostId) or d.hostId or model.LOCAL
			local id = (session and session.deviceId) or d.id
			dismissed[dismissal(chat, host, id)] = nil
			state.patch(props.panel_key, props.tab.id, { device_target = { host = host, id = id }, title = false })
		end, function(why: string)
			r.opening:set(nil)
			r.error:set(why)
		end)
	end
	local groups = ui.computed(function(): { any }
		return if ready() then model.picker_groups(hub()) else {}
	end)
	local function row(d: model.Device): Node
		local key = model.key(d.hostId, d.id)
		local action = s(if d.booted then "picker.open" else "picker.start")
		return ui.row {
			id = `device-row-{key}`,
			role = "button",
			label = s("picker.row_label", { action = action, name = d.name }),
			w_full = true,
			gap = "$space.3",
			items = "center",
			px = "$space.2",
			py = "$space.2",
			rounded = "$radius.md",
			cursor = "pointer",
			hover = { bg = "$hover" },
			focusable = true,
			on_click = function()
				open(d)
			end,
			on_key = function(e): boolean
				if e.key == "Enter" or e.key == " " then
					open(d)
					return true
				end
				return false
			end,
			Icon { name = "smartphone", size = 15, color = "$text.secondary" },
			ui.column {
				grow = 1,
				min_w = 0,
				ds.FadeText { d.name, text_size = "$font.size.md", color = "$text.primary" },
				ds.FadeText {
					s("picker.detail", { description = model.description(hub(), d), status = s(if d.booted then "picker.running" else "picker.stopped") }),
					text_size = "$font.size.sm",
					color = "$text.secondary",
				},
			},
			ui.text {
				function(): string
					return if r.opening:get() == key then s("picker.opening") else action
				end,
				shrink = 0,
				text_size = "$font.size.sm",
				color = "$text.secondary",
			},
		}
	end
	return ui.scroll {
		id = "device-picker",
		w_full = true,
		grow = 1,
		min_h = 0,
		ui.column {
			w_full = true,
			p = "$space.4",
			gap = "$space.1",
			ui.show(function(): boolean
				return #groups:get() == 0
			end, small(function(): string
				local h = hub()
				if h.hostStatus == "failed" then
					return h.hostStatusDetail or s("picker.failed")
				end
				return s("picker.empty")
			end, false, "device-picker-empty")),
			ui.each(groups, function(g: any): Node
				local rows: { Node } = {
					ui.text {
						g.title,
						id = if g.host == model.LOCAL then `device-group-{g.platform}` else `device-group-{g.host}-{g.platform}`,
						mt = "$space.2",
						text_size = "$font.size.xs",
						weight = "$font.weight.medium",
						color = "$text.tertiary",
					},
				}
				for _, d in g.devices do
					table.insert(rows, row(d))
				end
				return ui.column { w_full = true, gap = "$space.1", table.unpack(rows) }
			end, {
				key = function(g: any): string
					local parts = { g.host, g.platform }
					for _, d in g.devices do
						table.insert(parts, `{d.id}:{d.booted == true}`)
					end
					return table.concat(parts, "/")
				end,
			}),
			ui.each(ui.computed(function(): { any }
				return model.host_notes(hub())
			end), function(n: any): Node
				return small(n.text, false, `device-host-note-{n.id}`)
			end, {
				key = function(n: any): string
					return `{n.id}:{n.text}`
				end,
			}),
			ui.show(function(): boolean
				if not ready() then
					return false
				end
				local h = hub()
				for _, d in h.devices or {} do
					if d.platform == "android" then
						return false
					end
				end
				for _, host in h.hosts or {} do
					for _, p in host.platforms or {} do
						if p.platform == "android" and not p.available then
							return false
						end
					end
				end
				return true
			end, small(ui.t("device.picker.android_hint"))),
			ui.row {
				mt = "$space.1",
				ds.Button {
					id = "device-refresh",
					tone = "ghost",
					size = "sm",
					label = ui.t("device.picker.refresh"),
					on_click = function()
						refresh(r)
					end,
					ui.t("device.picker.refresh"),
				},
			},
		},
	}
end

local NEXT_ORIENTATION = {
	portrait = "landscape_left",
	landscape_left = "portrait_upside_down",
	portrait_upside_down = "landscape_right",
	landscape_right = "portrait",
}
function M.next_orientation(orientation: string?): string
	return NEXT_ORIENTATION[orientation or "portrait"] or "landscape_left"
end

local function stream(props: { tab: any, panel_key: string, r: Run, visible: (() -> boolean)? }): Node
	local r = props.r
	local chat = chat_of(props.panel_key)
	local t = target_of(props.tab) :: { string }
	local host, id = t[1], t[2]
	local key = props.tab.id
	local screen = Screen.make {
		key = key,
		host = host,
		id = id,
		visible = props.visible,
		on_error = function(why: string)
			r.error:set(why)
		end,
	}
	local send, device, screen_config = screen.send, screen.device, screen.screen_config
	local function rotate()
		local sc = screen_config()
		send({ orientation = { orientation = M.next_orientation(if sc then sc.orientation else nil) } })
	end
	local more_open = ui.state(`device_more_{key}`, false)
	local function power_off()
		if r.powering_off:peek() then
			return
		end
		r.powering_off:set(true)
		r.error:set(nil)
		local p = target_params(host, id)
		p.chatId = chat
		p.hostId = host
		p.shutdown = true
		call("CloseHubDevice", p, function()
			r.powering_off:set(false)
			api.unstream(key)
			state.patch(props.panel_key, key, { device_target = false, title = false })
		end, function(why: string)
			r.powering_off:set(false)
			r.error:set(why)
		end)
	end
	local function screenshot()
		if r.shooting:peek() then
			return
		end
		r.shooting:set(true)
		r.error:set(nil)
		call("HubDeviceScreenshot", { hostId = host, deviceId = id }, function(shot: any)
			r.shooting:set(false)
			local text = s("saved", { path = (shot and shot.path) or "" })
			r.toast:set(text)
			ui.after(TOAST_MS, function()
				if r.toast:peek() == text then
					r.toast:set(nil)
				end
			end)
		end, function(why: string)
			r.shooting:set(false)
			r.error:set(why)
		end)
	end
	local rail = ui.column {
		id = "device-rail",
		w = 40,
		shrink = 0,
		gap = "$space.1",
		items = "center",
		py = "$space.2",
		IconButton {
			id = "device-home",
			label = ui.t("device.rail.home"),
			tone = "quiet",
			on_click = function()
				send({ button = { button = "home" } })
			end,
			Icon { name = "home", size = 16 },
		},
		ui.show(function(): boolean
			local d = device()
			return d ~= nil and d.platform == "android"
		end, IconButton {
			id = "device-back",
			label = ui.t("device.rail.back"),
			tone = "quiet",
			on_click = function()
				send({ button = { button = "back" } })
			end,
			Icon { name = "arrow-left", size = 16 },
		}),
		IconButton {
			id = "device-rotate",
			label = ui.t("device.rail.rotate"),
			tone = "quiet",
			on_click = rotate,
			Icon { name = "rotate", size = 16 },
		},
		IconButton {
			id = "device-screenshot",
			label = function(): string
				return s(if r.shooting:get() then "rail.capturing" else "rail.screenshot")
			end,
			tone = "quiet",
			on_click = screenshot,
			Icon { name = "gallery", size = 16 },
		},
		ContextMenu {
			open = more_open,
			label = ui.t("device.rail.more"),
			shrink = 0,
			items = function(): { any }
				return {
					{ label = s("rail.float"), icon = "widget", on_select = function()
						Float.float_tab(chat, host, id)
					end },
					{ label = s("rail.close"), icon = "close", on_select = function()
						state.close(props.panel_key, key)
						M.close(props.tab, props.panel_key)
					end },
					{ separator = true },
					{ label = s("rail.power_off"), icon = "stop", danger = true, on_select = power_off },
				}
			end,
			IconButton {
				id = "device-more",
				label = ui.t("device.rail.more"),
				tone = "quiet",
				on_click = function()
					more_open:set(not more_open:peek())
				end,
				Icon { name = "more-horizontal", size = 16 },
			},
		},
	}
	local view = ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		items = "center",
		justify = "center",
		p = "$space.3",
		relative = true,
		screen.node,
		ui.show(function(): boolean
			return r.toast:get() ~= nil
		end, ui.row {
			id = "device-toast",
			role = "status",
			absolute = true,
			top = 8,
			left = 0,
			right = 0,
			justify = "center",
			ui.div {
				px = "$space.3",
				py = "$space.1",
				rounded = "$radius.full",
				bg = "$surface.raised",
				border = 1,
				border_color = "$border",
				small(function(): string
					return r.toast:get() or ""
				end),
			},
		}),
	}
	return ui.row {
		id = "device-screen-area",
		w_full = true,
		grow = 1,
		min_h = 0,
		view,
		rail,
	}
end

function M.Tab(props: { tab: any, panel_key: string, visible: (() -> boolean)? }): Node
	local id = props.tab.id
	local r = run_of(id)
	local chat = chat_of(props.panel_key)
	local stop = api.watch()
	ui.on_cleanup(stop)
	-- (Followed: its device can be set from outside the tab, an agent's opened for it.)
	local function tab(): any
		return live(props.tab)
	end
	local function hub(): any
		return api.hub:get()
	end
	local phase = ui.computed(function(): model.Phase
		return model.phase(hub(), chat, target_of(tab()), r.opening:get())
	end)
	-- (A device that's gone from the hub's sessions, once it was listed there:
	-- the tab goes back to the picker. One the engine has opened but the hub
	-- hasn't listed yet is kept: its push comes after the open's answer.)
	ui.effect(function()
		local h = hub()
		local t = target_of(tab())
		if not (h and not h.outdated and t and r.opening:peek() == nil and (h.settings or {}).onboardingCompleted and h.hostStatus == "ready") then
			return
		end
		local k = model.key(t[1], t[2])
		if model.session(h, chat, t[1], t[2]) then
			r.seen_live = k
		elseif r.seen_live == k then
			r.seen_live = nil
			api.unstream(id)
			state.patch(props.panel_key, id, { device_target = false, title = false })
		end
	end)
	local function body_kind(): string
		local h = hub()
		if h and h.outdated then
			return "outdated"
		end
		if r.powering_off:get() then
			return "off"
		end
		local p = phase:get()
		if p.kind == "stream" then
			local t = target_of(tab())
			return if t then `stream:{t[1]}/{t[2]}` else "picker"
		end
		return p.kind
	end
	local banners = ui.column {
		w_full = true,
		gap = "$space.1",
		px = "$space.3",
		ui.show(function(): boolean
			local h = hub()
			return h ~= nil and h.hostStatus == "ready" and phase:get().kind ~= "stream" and phase:get().kind ~= "setup" and h.hostStatusDetail ~= nil
		end, small(function(): string
			local h = hub()
			return (h and h.hostStatusDetail) or ""
		end, false, "device-host-detail")),
		ui.show(function(): boolean
			return phase:get().kind ~= "setup"
		end, host_update(function(): any
			return hub() or {}
		end, function()
			refresh(r)
		end)),
		ui.show(function(): boolean
			return phase:get().kind ~= "setup" and model.booting_banner(hub(), chat) ~= nil
		end, small(function(): string
			return model.booting_banner(hub(), chat) or ""
		end, false, "device-booting")),
	}
	local error_line = ui.show(function(): boolean
		return r.error:get() ~= nil
	end, ui.row {
		id = "device-panel-error",
		role = "alert",
		w_full = true,
		gap = "$space.2",
		items = "start",
		px = "$space.3",
		py = "$space.2",
		ui.column { grow = 1, min_w = 0, small(function(): string
			return r.error:get() or ""
		end, true) },
		IconButton {
			id = "device-error-dismiss",
			label = ui.t("device.dismiss_error"),
			tone = "quiet",
			size = "sm",
			on_click = function()
				r.error:set(nil)
			end,
			Icon { name = "close", size = 12 },
		},
	})
	return ui.column {
		id = `device-tab-{id}`,
		w_full = true,
		grow = 1,
		min_h = 0,
		banners,
		error_line,
		ui.presence(body_kind, function(kind: string): Node
			if kind == "outdated" then
				return ds.EmptyState { icon = "smartphone", size = "sm", fill = true, title = ui.t("device.tab") :: any, body = ui.t("device.unavailable.engine") :: any }
			elseif kind == "off" then
				return ui.row { grow = 1, w_full = true, items = "center", justify = "center", gap = "$space.2", Spinner { size = 12 }, small(ui.t("device.powering_off")) }
			elseif kind == "setup" then
				return setup { tab = props.tab, panel_key = props.panel_key, r = r }
			elseif kind == "loading" then
				return loading(function(): model.Phase
					return phase:get()
				end, function(): number
					return 1
				end)
			elseif kind == "picker" then
				return picker { tab = props.tab, panel_key = props.panel_key, r = r }
			end
			-- (Its screen once, the loading's second step over it until the first frame lands.)
			local current = tab()
			local d = device_of(current)
			return ui.column {
				w_full = true,
				grow = 1,
				min_h = 0,
				relative = true,
				stream { tab = current, panel_key = props.panel_key, r = r, visible = props.visible },
				ui.show(function(): boolean
					local f = api.frames:get()[id]
					return f == nil or (f.seq == nil and f.failed == nil)
				end, ui.column {
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					bottom = 0,
					bg = "$background",
					loading(function(): model.Phase
						return { kind = "loading", name = if d then d.name else s("tab"), description = if d then model.description(hub(), d) else "", message = s("stream.connecting") }
					end, function(): number
						return 2
					end),
				}),
				-- (The engine ended the stream: said, still, with Try again; no spinner.)
				ui.show(function(): boolean
					local f = api.frames:get()[id]
					return f ~= nil and f.failed ~= nil
				end, ui.column {
					id = "device-stream-failed",
					role = "alert",
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					bottom = 0,
					bg = "$background",
					items = "center",
					justify = "center",
					gap = "$space.3",
					px = "$space.6",
					Icon { name = "smartphone", size = 24, color = "$text.secondary" },
					heading(ui.t("device.stream.failed")),
					small(function(): string
						local f = api.frames:get()[id]
						local why = f and f.failed
						return if why == "done" then s("stream.ended") else words(why)
					end),
					button("device-stream-retry", ui.t("device.stream.retry"), function(): boolean
						return true
					end, function()
						local t = target_of(tab())
						if t then
							api.unstream(id)
							api.stream(id, t[1], t[2])
						end
					end),
				}),
			}
		end, { w_full = true, grow = 1, min_h = 0 }),
	}
end

return M
