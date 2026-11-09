-- Settings → Device hub: gpui's page (crates/ui/src/settings/device_hub.rs)
-- over the engine's hub (`ListHubDevices`, `ConfigureDeviceHub`,
-- `TestDeviceHost` through `settings.engine`; its live state from
-- screens/device/api's watch): the hub's switch and its versions with Check
-- versions and Update, platform support with Refresh, agent device access,
-- and the device hosts: each with its status,
-- Test connection and Remove, and Add host's form with its checks.
-- Listed while the Device built-in is on (Settings → Plugins).

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local api = require("@app/screens/device/api")
local model = require("@app/screens/device/model")
local DevicePanel = require("@app/screens/device/panel")
local Spinner = require("@ds/status").Spinner
local Icon = require("@ds/icon")

local M = {}

M.ID = "device_hub"
M.TITLE = "Device hub"

local function t(key: string, args: { [string]: any }?): Message
	return ui.t(`settings.device_hub.{key}`, args)
end

search.index(M.ID, t("title"), t("terms"), {
	{ key = "hub", title = t("hub.label"), section = t("title"), terms = t("hub.terms") },
	{ key = "platforms", title = t("platforms.label"), section = t("title"), terms = t("platforms.terms") },
	{ key = "agent", title = t("agent.label"), section = t("title"), terms = t("agent.terms") },
	{ key = "hosts", title = t("hosts.label"), section = t("title"), terms = t("hosts.terms") },
})

-- The floating preview an agent's device shows in (gpui's `deviceAutoFloat`, on by default):
-- kept (and migrated), its switch not listed until zgx has a floating device window,
-- since an agent's device opens a tab either way.
M.auto_float = ui.persist("device_auto_float", true)

-- What's under way: "hub", "check", "agent", "update"; and what went wrong.
local pending = ui.state("settings_device_pending", nil :: string?)
local update_error = ui.state("settings_device_update_error", nil :: string?)
local unread = ui.state("settings_device_unread", nil :: string?)
-- Add host's form (nil: closed), each host's last check, and saving the list.
local editor = ui.state("settings_device_editor", nil :: any)
local checks = ui.state("settings_device_checks", {} :: { [string]: any })
local saving = ui.state("settings_device_saving", false)
local hosts_error = ui.state("settings_device_hosts_error", nil :: string?)

local function says(err: any): string
	local word = if type(err) == "table" then err.message else err
	if word == "outdated" then
		return ui.t("settings.engine.outdated"):get()
	elseif word == "offline" then
		return ui.t("settings.engine.offline"):get()
	end
	return if type(word) == "string" and word ~= "" then word else ui.t("common.something_went_wrong"):get()
end

local function engine(method: string, params: { [string]: any }?): any
	return host.call("settings.engine", { method = method, params = params or {} })
end

local function hub(): any
	return api.hub:get()
end

local function heard(v: any)
	if type(v) == "table" and v.settings then
		api.hub:set(v)
	end
end

local function call(what: string?, method: string, params: { [string]: any }, done: ((ok: boolean, v: any) -> ())?)
	if what then
		pending:set(what)
	end
	engine(method, params):after(function(v: any)
		if what and pending:peek() == what then
			pending:set(nil)
		end
		heard(v)
		unread:set(nil)
		if done then
			done(true, v)
		end
	end, function(err: any)
		if what and pending:peek() == what then
			pending:set(nil)
		end
		if what == "update" then
			update_error:set(t("versions.update_failed"):get())
		elseif what == nil then
			unread:set(says(err))
		end
		if done then
			done(false, says(err))
		end
	end)
end

local function load()
	call(nil, "ListHubDevices", { inspectOnly = true })
end
M.load = load

local function list(what: string, inspect_only: boolean)
	update_error:set(nil)
	call(what, "ListHubDevices", if inspect_only then { inspectOnly = true } else {})
end

local function saved_hosts(): { any }
	local h = api.hub:peek()
	return table.clone(((h or {}).settings or {}).deviceHosts or {})
end

local function save_hosts(next: { any }, close: boolean)
	hosts_error:set(nil)
	saving:set(true)
	call(nil, "ConfigureDeviceHub", { deviceHosts = next }, function(ok: boolean, v: any)
		saving:set(false)
		if ok then
			if close then
				editor:set(nil)
			end
		else
			hosts_error:set(v)
		end
	end)
end

local function test_host(h: any)
	local all = table.clone(checks:peek())
	all[h.id] = { pending = true }
	checks:set(all)
	engine("TestDeviceHost", h):after(function(v: any)
		local next = table.clone(checks:peek())
		next[h.id] = { result = v }
		checks:set(next)
	end, function(err: any)
		local next = table.clone(checks:peek())
		next[h.id] = { failed = says(err) }
		checks:set(next)
	end)
end

-- The form's fields (one form at a time).
local ed = {
	name = ui.state("settings_device_ed_name", ""),
	target = ui.state("settings_device_ed_target", ""),
	identity = ui.state("settings_device_ed_identity", ""),
	port = ui.state("settings_device_ed_port", ""),
	error = ui.state("settings_device_ed_error", nil :: string?),
}
local seq = 0
local function open_editor()
	seq += 1
	local id = string.format("host-%06x%06x", os.time() % 0xffffff, (seq * 7919) % 0xffffff)
	for _, k in { "name", "target", "identity", "port" } do
		(ed :: any)[k]:set("")
	end
	ed.error:set(nil)
	editor:set({ id = id })
end

local function draft(e: any): (any, string?)
	return model.draft(e.id, ed.name:peek(), ed.target:peek(), ed.identity:peek(), ed.port:peek())
end

local function small(text: any, danger: boolean?, id: string?): Node
	return ui.text { text, id = id, wrap = true, text_size = "$font.size.sm", color = if danger then "$zeron.tone.danger.ink" else "$text.secondary" }
end

local function button(id: string, label: any, enabled: () -> boolean, run: () -> ()): Node
	return ds.Button {
		id = id,
		tone = "secondary",
		size = "sm",
		label = label,
		disabled = function(): boolean
			return not enabled()
		end,
		on_click = run,
		label,
	}
end

-- A host's check, line by line (gpui's `render_check`).
local function check_lines(c: any): Node
	if c.pending then
		return small(t("hosts.checking"), false, "device-host-check")
	end
	local lines: { Node } = {}
	local function line(ok: boolean, text: string)
		table.insert(lines, ui.row {
			gap = "$space.1",
			items = "start",
			Icon { name = if ok then "check" else "close", size = 12, color = if ok then "$text.secondary" else "$zeron.tone.danger.ink" },
			small(text, not ok),
		})
	end
	if c.failed then
		line(false, c.failed)
	elseif c.result and c.result["local"] then
		line(true, t("hosts.local"):get())
	elseif c.result then
		for _, x in c.result.checks or {} do
			line(x.ok == true, if x.detail then t("hosts.check_line", { name = x.name, detail = x.detail }):get() else x.name)
		end
	end
	return ui.column { id = "device-host-check", role = "status", gap = "$space.1", table.unpack(lines) }
end

local function host_row(h: any): Node
	local function summary(): any
		for _, x in (hub() or {}).hosts or {} do
			if x.id == h.id then
				return x
			end
		end
		return nil
	end
	local function check(): any
		return checks:get()[h.id]
	end
	local function status(): any
		return ((hub() or {}).hostStatuses or {})[h.id]
	end
	return ui.column {
		id = `settings-device-host-{h.id}`,
		w_full = true,
		gap = "$space.1",
		ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			ui.column {
				grow = 1,
				min_w = 0,
				ui.row {
					gap = "$space.2",
					items = "center",
					ui.text { h.label, text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
					ui.text {
						function(): string
							local marks = {}
							for _, p in (summary() or {}).platforms or {} do
								if p.available then
									table.insert(marks, t("hosts.available", { platform = model.platform_name(p.platform) }):get())
								end
							end
							return table.concat(marks, "  ")
						end,
						text_size = "$font.size.xs",
						color = "$text.secondary",
					},
				},
				ds.FadeText { h.target, text_size = "$font.size.sm", color = "$text.secondary" },
			},
			small(function(): string
				local c = check()
				local st = status()
				if c and c.pending then
					return t("hosts.checking"):get()
				elseif st and st.status == "installing" then
					return t("hosts.installing"):get()
				elseif st and st.status == "starting" then
					return st.detail or t("hosts.connecting"):get()
				end
				return ""
			end),
			button(`settings-device-host-test-{h.id}`, t("hosts.test"), function(): boolean
				local c = check()
				return not saving:get() and not (c and c.pending)
			end, function()
				test_host(h)
			end),
			button(`settings-device-host-remove-{h.id}`, t("hosts.remove"), function(): boolean
				return not saving:get()
			end, function()
				local next = {}
				for _, x in saved_hosts() do
					if x.id ~= h.id then
						table.insert(next, x)
					end
				end
				local all = table.clone(checks:peek())
				all[h.id] = nil
				checks:set(all)
				save_hosts(next, false)
			end),
		},
		ui.show(function(): boolean
			local st = status()
			return st ~= nil and st.status == "failed" and st.detail ~= nil
		end, small(function(): string
			local st = status()
			return if st and st.detail then t("hosts.failed", { detail = st.detail }):get() else ""
		end, true)),
		ui.presence(function(): string
			local c = check()
			if not c or c.pending then
				return ""
			end
			return if c.failed then `f:{c.failed}` else `r:{#((c.result or {}).checks or {})}:{tostring((c.result or {})["local"])}`
		end, function(which: string): Node
			local c = checks:peek()[h.id]
			if which == "" or not c then
				return ui.div {}
			end
			return check_lines(c)
		end),
	}
end

local function field(label: Message, value: State<string>, placeholder: Message, id: string): Node
	return ui.column {
		gap = "$space.1",
		w_full = true,
		small(label),
		ds.TextField { id = id, value = value, label = label, placeholder = placeholder },
	}
end

local function editor_view(e: any): Node
	local function checking(): boolean
		local c = checks:get()[e.id]
		return c ~= nil and c.pending == true
	end
	return ui.column {
		id = "settings-device-host-editor",
		w_full = true,
		gap = "$space.3",
		p = "$space.3",
		rounded = "$radius.lg",
		border = 1,
		border_color = "$border",
		small(t("hosts.same_machine")),
		field(t("editor.name"), ed.name, t("editor.name_ph"), "settings-device-host-name"),
		field(t("editor.target"), ed.target, t("editor.target_ph"), "settings-device-host-target"),
		ui.row {
			w_full = true,
			gap = "$space.3",
			ui.column { grow = 1, field(t("editor.identity"), ed.identity, t("editor.identity_ph"), "settings-device-host-identity") },
			ui.column { w = 110, shrink = 0, field(t("editor.port"), ed.port, t("editor.port_ph"), "settings-device-host-port") },
		},
		ui.show(function(): boolean
			return ed.error:get() ~= nil
		end, small(function(): string
			return ed.error:get() or ""
		end, true, "settings-device-host-error")),
		ui.presence(function(): string
			local c = checks:get()[e.id]
			if not c then
				return ""
			end
			return if c.pending then "p" elseif c.failed then `f:{c.failed}` else `r:{#((c.result or {}).checks or {})}`
		end, function(which: string): Node
			local c = checks:peek()[e.id]
			return if which == "" or not c then ui.div {} else check_lines(c)
		end),
		ui.row {
			w_full = true,
			gap = "$space.2",
			justify = "end",
			button("settings-device-host-cancel", ui.t("common.cancel"), function(): boolean
				return not saving:get()
			end, function()
				editor:set(nil)
			end),
			button("settings-device-host-editor-test", function(): string
				return t(if checking() then "editor.checking" else "editor.test"):get()
			end, function(): boolean
				return not saving:get() and not checking()
			end, function()
				local h, why = draft(e)
				ed.error:set(why)
				if h then
					test_host(h)
				end
			end),
			button("settings-device-host-save", function(): string
				return t(if saving:get() then "editor.saving" else "editor.save"):get()
			end, function(): boolean
				return not saving:get() and not checking()
			end, function()
				local h, why = draft(e)
				ed.error:set(why)
				if not h then
					return
				end
				local next = {}
				for _, x in saved_hosts() do
					if x.id ~= h.id then
						table.insert(next, x)
					end
				end
				table.insert(next, h)
				save_hosts(next, true)
			end),
		},
	}
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

local function synced(id: string, read: () -> boolean): State<boolean>
	local s = ui.state(`settings_device_switch_{id}`, read())
	ui.effect(function()
		local v = read()
		if s:peek() ~= v then
			s:set(v)
		end
	end)
	return s
end

local function gated(enabled: () -> boolean, node: Node): Node
	return ui.div {
		shrink = 0,
		opacity = function(): number
			return if enabled() then 1 else 0.5
		end,
		pointer_events = function(): string
			return if enabled() then "auto" else "none"
		end,
		node,
	}
end

function M.view(): Node
	visits.once("device_hub", load)
	local stop = api.watch()
	ui.on_cleanup(stop)
	local function loaded(): boolean
		local h = hub()
		return h ~= nil and h.settings ~= nil
	end
	local function on(): boolean
		return loaded() and hub().settings.enabled == true
	end
	local function busy(): boolean
		return model.busy(hub())
	end
	local function idle(): boolean
		return pending:get() == nil
	end
	local hub_switch = synced("hub", on)
	local agent_switch = synced("agent", function(): boolean
		return loaded() and hub().settings.agentAccessEnabled == true
	end)
	local versions = ui.column {
		w_full = true,
		gap = "$space.2",
		ui.presence(function(): string
			local rows = model.version_rows(hub())
			if not rows then
				return ""
			end
			local parts = {}
			for _, r in rows do
				table.insert(parts, `{r.label}={r.value}`)
			end
			return table.concat(parts, "\n")
		end, function(which: string): Node
			if which == "" then
				return small(t("versions.unchecked"), false, "settings-device-versions")
			end
			local lines: { Node } = {}
			for _, r in model.version_rows(api.hub:peek()) or {} do
				table.insert(lines, ui.row {
					w_full = true,
					justify = "between",
					gap = "$space.6",
					ui.text { r.label, text_size = "$font.size.sm", color = "$text.secondary" },
					ui.text { r.value, literal = true, font = "$font.mono", text_size = "$font.size.sm", color = "$text.primary" },
				})
			end
			return ui.column { id = "settings-device-versions", max_w = 280, w_full = true, gap = "$space.1", table.unpack(lines) }
		end),
		ui.row {
			gap = "$space.2",
			ui.show(function(): boolean
				return model.needs_update(hub()) ~= nil
			end, button("settings-device-hub-update", function(): string
				if pending:get() == "update" then
					return t("versions.updating"):get()
				end
				return t("versions.update", { version = model.needs_update(hub()) or "" }):get()
			end, function(): boolean
				return on() and idle() and not busy()
			end, function()
				list("update", false)
			end)),
			button("settings-device-hub-check", function(): string
				return t(if pending:get() == "check" then "versions.checking" else "versions.check"):get()
			end, function(): boolean
				return idle() and not busy()
			end, function()
				list("check", true)
			end),
		},
		ui.show(function(): boolean
			return update_error:get() ~= nil
		end, small(function(): string
			return update_error:get() or ""
		end, true, "settings-device-update-error")),
	}
	local hub_control = ui.row {
		gap = "$space.2",
		items = "center",
		ui.show(function(): boolean
			return pending:get() == "hub" and model.hub_status(hub(), true, true) ~= nil
		end, ui.row { gap = "$space.2", items = "center", Spinner { size = 12 }, small(function(): string
			return model.hub_status(hub(), true, true) or ""
		end) }),
		gated(function(): boolean
			return loaded() and not busy() and idle()
		end, ds.Switch {
			value = hub_switch,
			label = t("hub.label"),
			on_change = function(v: boolean)
				call("hub", "ConfigureDeviceHub", if v then { enabled = true, onboardingCompleted = true } else { enabled = false, agentAccessEnabled = false })
			end,
		}),
	}
	local agent_control = ui.row {
		gap = "$space.2",
		items = "center",
		ui.show(function(): boolean
			return pending:get() == "agent"
		end, ui.row { gap = "$space.2", items = "center", Spinner { size = 12 }, small(function(): string
			return model.agent_status(hub(), true)
		end) }),
		gated(function(): boolean
			return loaded() and on() and not busy() and idle()
		end, ds.Switch {
			value = agent_switch,
			label = t("agent.label"),
			on_change = function(v: boolean)
				call("agent", "ConfigureDeviceHub", { agentAccessEnabled = v })
			end,
		}),
	}
	local hosts_list = ui.column {
		w_full = true,
		gap = "$space.3",
		ui.show(function(): boolean
			return #(((hub() or {}).settings or {}).deviceHosts or {}) == 0 and editor:get() == nil
		end, small(t("hosts.none"), false, "settings-device-hosts-none")),
		ui.each(ui.computed(function(): { any }
			return ((hub() or {}).settings or {}).deviceHosts or {}
		end), function(h: any): Node
			return host_row(h)
		end, {
			key = function(h: any): string
				return `{h.id}:{h.label}:{h.target}`
			end,
		}),
		ui.presence(function(): string
			local e = editor:get()
			return if e then e.id else ""
		end, function(which: string): Node
			local e = editor:peek()
			return if which == "" or not e then ui.div {} else editor_view(e)
		end),
		ui.show(function(): boolean
			return hosts_error:get() ~= nil
		end, small(function(): string
			return hosts_error:get() or ""
		end, true, "settings-device-hosts-error")),
	}
	return ds.Settings.Page {
		title = t("title"),
		description = t("subtitle"),
		ui.show(function(): boolean
			return unread:get() ~= nil
		end, ds.Callout {
			tone = "warning",
			title = t("unread"),
			body = function(): string
				return unread:get() or ""
			end,
			action = {
				label = ui.t("common.retry") :: any,
				on_click = function()
					visits.again("device_hub", load)
				end,
			},
		}),
		ui.show(function(): boolean
			return not loaded() and unread:get() == nil
		end, small(t("loading"))),
		ui.show(loaded, ui.column {
			w_full = true,
			gap = "$space.6",
			ds.Settings.Section {
				title = t("title"),
				ds.Settings.Group {
					ui.column {
						w_full = true,
						row("hub", { label = t("hub.label"), description = ui.t("device.setup.hub_body"), control = hub_control }),
						ui.column { w_full = true, px = "$space.4", pb = "$space.3", versions },
						ui.show(on, ui.column {
							w_full = true,
							ds.Settings.Rule(),
							row("platforms", {
								label = t("platforms.label"),
								control = button("settings-device-platforms-refresh", function(): string
									return t(if pending:get() == "check" then "versions.checking" else "platforms.refresh"):get()
								end, function(): boolean
									return on() and not busy() and idle()
								end, function()
									list("check", false)
								end),
							}),
							ui.row {
								w_full = true,
								px = "$space.4",
								pb = "$space.3",
								gap = "$space.5",
								ui.column { grow = 1, min_w = 0, DevicePanel.platform_card(function(): any
									return hub() or {}
								end, "ios", true) },
								ui.column { grow = 1, min_w = 0, DevicePanel.platform_card(function(): any
									return hub() or {}
								end, "android", true) },
							},
						}),
						ds.Settings.Rule(),
						row("agent", {
							label = t("agent.label"),
							description = ui.t("device.setup.agent_body"),
							detail = function(): string?
								return model.agent_failure(hub())
							end,
							control = agent_control,
						}),
						ds.Settings.Rule(),
						row("hosts", {
							label = t("hosts.label"),
							description = t("hosts.hint"),
							control = button("settings-device-host-add", t("hosts.add"), function(): boolean
								return loaded() and editor:get() == nil and not saving:get()
							end, open_editor),
						}),
						ui.column { w_full = true, px = "$space.4", pb = "$space.3", hosts_list },
					},
				},
			},
			DevicePanel.host_update(function(): any
				return hub() or {}
			end, function()
				list("check", false)
			end),
		}),
	}
end

return M
