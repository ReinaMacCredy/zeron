-- Settings → Computer use: the shipping app's (crates/ui/src/settings/
-- computer.rs). The engine keeps the settings (`GetComputerSettings`,
-- `SetComputerSettings` through `settings.engine`); the status line is
-- `settings.computer_status`'s, gpui's own words for what the engine
-- says of its desktop host. As gpui's, only a Mac edits them: elsewhere
-- they show what the engine has. Click sounds and the guide's Got it are
-- this app's own (gpui's ui-settings, carried). Each change shows at once,
-- and goes back with the engine's words when it refuses.

local host = require("@app/screens/settings/host")
local ds = require("@ds")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")

local M = {}

M.ID = "computer"
M.TITLE = "Computer use"

local function t(key: string): Message
	return ui.t(`settings.computer.{key}`)
end

search.index(M.ID, t("title"), t("terms"), {
	{ key = "control", title = t("control.title"), section = t("control.title"), terms = t("control.terms") },
	{ key = "cursor", title = t("cursor.label"), section = t("control.title"), terms = t("cursor.terms") },
	{ key = "preview", title = t("preview.label"), section = t("control.title"), terms = t("preview.terms") },
	{ key = "placement", title = t("placement.label"), section = t("control.title"), terms = t("placement.terms") },
	{ key = "sounds", title = t("sounds.label"), section = t("control.title"), terms = t("sounds.terms") },
	{ key = "developer", title = t("developer.title"), section = t("developer.title"), terms = t("developer.terms") },
	{ key = "allowed", title = t("allowed.title"), section = t("allowed.title"), terms = t("allowed.terms") },
	{ key = "guide", title = t("guide.title"), section = t("guide.title"), terms = t("guide.terms") },
	{ key = "driver", title = t("advanced.driver"), section = t("advanced.title"), terms = t("advanced.terms") },
})

-- The engine's, as last read or written (nil until read), and its desktop host's status.
local settings = ui.state("settings_computer", nil :: any)
local status = ui.state("settings_computer_status", nil :: any)
local unread = ui.state("settings_computer_unread", nil :: string?)
local refused = ui.state("settings_computer_refused", nil :: string?)
local guide_asked = ui.state("settings_computer_guide", false)
local advanced_open = ui.state("settings_computer_advanced", false)

-- gpui's `computer_click_sounds` ("foreground", "foreground-and-background", "off") and `computer_getting_started_acknowledged`.
M.click_sounds = ui.persist("click_sounds", "foreground")
M.acknowledged = ui.persist("guide_acknowledged", false)

local DEFAULTS = { autoOpenComputerPane = true, computerPreviewSize = "compact", computerPreviewPlacement = "rail" }

local function engine(method: string, params: { [string]: any }?): any
	return host.call("settings.engine", { method = method, params = params or {} })
end

local function says(err: any): string
	local word = if type(err) == "table" then err.message else err
	if word == "outdated" then
		return ui.t("settings.engine.outdated"):get()
	elseif word == "offline" then
		return ui.t("settings.engine.offline"):get()
	end
	return if type(word) == "string" and word ~= "" then word else ui.t("common.something_went_wrong"):get()
end

-- (A host without these, a preview's, says so as an unread page.)
local function ask(name: string, arg: any, ok: (v: any) -> (), bad: (err: any) -> ())
	local fine, p = pcall(host.call, name, arg)
	if fine and p ~= nil and p.after then
		p:after(ok, bad)
	else
		bad(if fine then "failed" else p)
	end
end

local function load()
	ask("settings.engine", { method = "GetComputerSettings", params = {} }, function(v: any)
		settings:set(v or {})
		unread:set(nil)
	end, function(err: any)
		unread:set(says(err))
	end)
	ask("settings.computer_status", {}, function(v: any)
		status:set(v)
	end, function() end)
end

M.load = load

-- (gpui's `editable`: the settings read, on the Mac that runs the engine.)
local function editable(): boolean
	local ok, platform = pcall(function(): any
		return ui.shared("platform", ""):get()
	end)
	return settings:get() ~= nil and ok and platform == "macos"
end

local function save(patch: { [string]: any })
	local before = settings:peek()
	if not before or not editable() then
		return
	end
	local next = table.clone(before)
	local changed = false
	for k, v in patch do
		if next[k] ~= v then
			changed = true
		end
		next[k] = v
	end
	if not changed then
		return
	end
	settings:set(next)
	engine("SetComputerSettings", next):after(function()
		refused:set(nil)
	end, function(err: any)
		settings:set(before)
		refused:set(says(err))
	end)
end

-- For the transcript's consent card (gpui's `approved_once`): its first-use words show until one is allowed, and while it isn't known.
local asked = false
function M.shows_disclosure(): boolean
	local s = settings:get()
	if s == nil and not asked then
		asked = true
		ui.after(0, load)
	end
	return s == nil or s.approvedOnce ~= true
end

function M.mark_approved()
	local s = settings:peek()
	if s and s.approvedOnce ~= true then
		local next = table.clone(s)
		next.approvedOnce = true
		settings:set(next)
	end
end

local function field(name: string): any
	local s = settings:get()
	return if s then s[name] else nil
end

-- A switch on one of the engine's: it follows the engine, a flip goes to it (and back, where it can't be changed).
local function engine_switch(label: Message, name: string): Node
	return ui.component(function(): Node
		local on = ui.use_state(field(name) == true)
		ui.effect(function()
			local v = field(name) == true
			if v ~= on:peek() then
				on:set(v)
			end
		end)
		return ds.Switch {
			value = on,
			label = label,
			on_change = function(v: boolean)
				if not editable() then
					on:set(not v)
					return
				end
				save({ [name] = v })
			end,
		}
	end)()
end

-- One of a few words, as gpui's chips (a segmented control here): it follows `read`, a pick goes to `write` (and back, where it can't be changed).
local function engine_choice(label: Message, read: () -> any, peek: () -> any, write: (v: any) -> (), options: { { value: any, label: any } }): Node
	return ui.component(function(): Node
		local chosen = ui.use_state(peek())
		ui.effect(function()
			local v = read()
			if v ~= chosen:peek() then
				chosen:set(v)
			end
		end)
		ui.effect(function()
			local v = chosen:get()
			if v ~= peek() then
				write(v)
				ui.after(0, function()
					if chosen:peek() ~= peek() then
						chosen:set(peek())
					end
				end)
			end
		end)
		return ds.Segmented { value = chosen, label = label, options = options :: any }
	end)()
end

local function peek_field(name: string): any
	local s = settings:peek()
	return if s then s[name] else nil
end

local function reset_button(id: string, label: Message, on_click: () -> ()): Node
	return ds.Button {
		ds.Icon { name = "restart", size = 12 },
		id = id,
		label = label,
		tone = "ghost",
		size = "xs",
		icon = true,
		on_click = on_click,
	}
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

-- gpui's `normalize_cursor_hex_color`: `#rrggbb` lowercased, else nothing.
function M.hex(value: string): string
	local v = string.lower(string.match(value, "^%s*(.-)%s*$") or "")
	return if string.match(v, "^#[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]$") then v else ""
end

local function cursor_field(id: string, label: Message, name: string): Node
	return ui.component(function(): Node
		local text = ui.use_state(field(name) or "")
		ui.effect(function()
			local v = field(name) or ""
			if v ~= text:peek() and M.hex(text:peek()) ~= v then
				text:set(v)
			end
		end)
		local function commit()
			local draft = string.match(text:peek(), "^%s*(.-)%s*$") or ""
			local v = if draft == "" then "" else M.hex(draft)
			if draft ~= "" and v == "" then
				return
			end
			save({ [name] = v })
		end
		return ui.row {
			gap = "$space.2",
			items = "center",
			ui.text { label, color = "$text.secondary", text_size = "$font.size.sm" },
			ui.div {
				w = 14,
				h = 14,
				rounded = "$radius.full",
				border = 1,
				border_color = "$border.subtle",
				bg = function(): string
					local v = M.hex(text:get())
					return if v ~= "" then v else "#00000000"
				end,
				aria_hidden = true,
			},
			ui.div {
				w = 112,
				ds.TextField {
					id = id,
					w = 112,
					value = text,
					name = label,
					placeholder = "#rrggbb",
					disabled = function(): boolean
						return not editable()
					end,
					on_commit = commit,
					on_focus_change = function(focused: boolean)
						if not focused then
							commit()
						end
					end,
				},
			},
		}
	end)()
end

local function status_line(): Node
	local function view(): any
		return status:get() or {}
	end
	return ui.row {
		id = "computer-status",
		w_full = true,
		gap = "$space.2",
		items = "center",
		px = "$space.2",
		text_size = "$font.size.sm",
		color = "$text.secondary",
		ui.div {
			w = 6,
			h = 6,
			shrink = 0,
			rounded = "$radius.full",
			bg = function(): string
				local k = view().kind
				return if k == "ready" then "$success" elseif k == "blocked" then "$danger" else "$warning"
			end,
			aria_hidden = true,
		},
		ui.text {
			function(): string
				local v = view()
				if not v.title then
					return t("checking"):get()
				end
				return `{v.title} · {v.line or ""}`
			end,
			wrap = true,
			min_w = 0,
		},
	}
end

local function attention(): boolean
	local v = status:get()
	return v ~= nil and v.attention == true
end

local function allowed_apps(): Node
	local apps = ui.computed(function(): { any }
		return field("alwaysAllowedApps") or {}
	end)
	return ui.column {
		w_full = true,
		gap = "$space.2",
		ui.show(function(): boolean
			return #apps:get() == 0
		end, ui.text { t("allowed.empty"), id = "computer-allowed-empty", wrap = true, color = "$text.secondary", text_size = "$font.size.sm", px = "$space.2" }),
		ds.Settings.Each(apps, function(app: any): Node
			local named = type(app.name) == "string" and string.match(app.name, "%S") ~= nil
			local title = if named then app.name else app.bundleId
			return ds.Settings.Row {
				label = title,
				description = if named then app.bundleId else nil,
				icon = "monitor",
				control = ds.Button {
					ds.Icon { name = "trash-bin-minimalistic", size = 14 },
					id = `computer-allowed-remove-{app.bundleId}`,
					label = ui.t("settings.computer.allowed.remove", { app = title }),
					tone = "ghost",
					size = "xs",
					icon = true,
					disabled = function(): boolean
						return not editable()
					end,
					on_click = function()
						local kept = {}
						for _, a in field("alwaysAllowedApps") or {} do
							if a.bundleId ~= app.bundleId then
								table.insert(kept, a)
							end
						end
						save({ alwaysAllowedApps = kept })
					end,
				},
			}
		end, { key = function(app: any): any
			return app.bundleId
		end }),
	}
end

local function guide(): Node
	local function open(): boolean
		return M.acknowledged:get() ~= true or guide_asked:get()
	end
	local steps = {}
	for i, key in { "ask", "approve", "follow" } do
		table.insert(steps, ds.Settings.Row { label = t(`guide.{key}.title`), description = t(`guide.{key}.body`) })
	end
	return ds.Settings.Section {
		title = t("guide.title"),
		trail = ds.Button {
			function(): string
				return t(if open() then "guide.hide" else "guide.show"):get()
			end,
			id = "computer-guide-toggle",
			tone = "ghost",
			size = "sm",
			on_click = function()
				if open() then
					guide_asked:set(false)
					M.acknowledged:set(true)
				else
					guide_asked:set(true)
				end
			end,
		},
		ui.show(open, ui.column {
			w_full = true,
			gap = "$space.2",
			search.anchor("guide", ds.Settings.Group(steps :: any)),
			ui.text { t("guide.appshots"), wrap = true, color = "$text.secondary", text_size = "$font.size.sm", px = "$space.2" },
			ui.show(function(): boolean
				return M.acknowledged:get() ~= true
			end, ui.row {
				px = "$space.2",
				ds.Button {
					t("guide.done"),
					id = "computer-guide-done",
					tone = "secondary",
					size = "sm",
					on_click = function()
						guide_asked:set(false)
						M.acknowledged:set(true)
					end,
				},
			}),
		}),
	}
end

function M.view(): Node
	visits.once("computer", load)
	local function ready(): boolean
		return settings:get() ~= nil
	end
	local function custom(): boolean
		return field("agentCursorColorMode") == "custom"
	end
	local function cursor_dirty(): boolean
		return (field("agentCursorColorMode") or "stock") ~= "stock" or (field("agentCursorFillColor") or "") ~= "" or (field("agentCursorRimColor") or "") ~= ""
	end
	local function preview_dirty(): boolean
		return (field("autoOpenComputerPane") ~= false) ~= DEFAULTS.autoOpenComputerPane or (field("computerPreviewSize") or "compact") ~= DEFAULTS.computerPreviewSize
	end
	return ds.Settings.Page {
		title = t("title"),
		description = t("note"),
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
					visits.again("computer", load)
				end,
			},
		}),
		ui.show(function(): boolean
			return not ready() and unread:get() == nil
		end, ui.text { t("loading"), color = "$text.secondary", text_size = "$font.size.sm" }),
		ui.show(ready, ui.column {
			w_full = true,
			gap = "$space.6",
			ds.Settings.Section {
				title = t("control.title"),
				description = function(): string
					return t(if editable() then "control.description" else "control.elsewhere"):get()
				end :: any,
				trail = search.anchor("control", engine_switch(t("control.label"), "computerControlEnabled")),
				status_line(),
				ui.show(function(): boolean
					return refused:get() ~= nil
				end, ui.text {
					function(): string
						return refused:get() or ""
					end,
					id = "computer-refused",
					role = "alert",
					wrap = true,
					px = "$space.4",
					text_size = "$font.size.sm",
					color = "$danger",
				}),
				ds.Settings.Group {
					ui.column {
						w_full = true,
					ui.show(attention, ui.column { w_full = true, ds.Settings.Row {
						label = function(): string
							return (status:get() or {}).title or ""
						end,
						description = function(): string
							return (status:get() or {}).description or ""
						end,
						detail = function(): string?
							return (status:get() or {}).reconnects
						end,
					}, ds.Settings.Rule() }),
					row("cursor", {
						label = t("cursor.label"),
						description = t("cursor.description"),
						control = ui.row {
							gap = "$space.2",
							items = "center",
							ui.show(function(): boolean
								return cursor_dirty() and editable()
							end, reset_button("computer-cursor-reset", t("cursor.reset"), function()
								save({ agentCursorColorMode = "stock", agentCursorFillColor = "", agentCursorRimColor = "" })
							end)),
							engine_choice(t("cursor.label"), function(): any
								return field("agentCursorColorMode") or "stock"
							end, function(): any
								return peek_field("agentCursorColorMode") or "stock"
							end, function(v: any)
								save({ agentCursorColorMode = v })
							end, { { value = "stock", label = t("cursor.stock") }, { value = "custom", label = t("cursor.custom") } }),
						},
					}),
					ui.show(custom, ui.row {
						w_full = true,
						px = ds.Settings.inset(),
						pb = "$space.3",
						gap = "$space.4",
						flex_wrap = true,
						cursor_field("computer-cursor-fill", t("cursor.fill"), "agentCursorFillColor"),
						cursor_field("computer-cursor-rim", t("cursor.rim"), "agentCursorRimColor"),
					}),
					},
					row("preview", {
						label = t("preview.label"),
						description = t("preview.description"),
						control = ui.row {
							gap = "$space.3",
							items = "center",
							ui.show(function(): boolean
								return preview_dirty() and editable()
							end, reset_button("computer-preview-reset", t("preview.reset"), function()
								save({ autoOpenComputerPane = DEFAULTS.autoOpenComputerPane, computerPreviewSize = DEFAULTS.computerPreviewSize })
							end)),
							engine_switch(t("preview.auto"), "autoOpenComputerPane"),
							engine_choice(t("preview.label"), function(): any
								return field("computerPreviewSize") or "compact"
							end, function(): any
								return peek_field("computerPreviewSize") or "compact"
							end, function(v: any)
								save({ computerPreviewSize = v })
							end, { { value = "compact", label = t("preview.compact") }, { value = "large", label = t("preview.large") } }),
						},
					}),
					row("placement", {
						label = t("placement.label"),
						description = t("placement.description"),
						control = ui.row {
							gap = "$space.2",
							items = "center",
							ui.show(function(): boolean
								return (field("computerPreviewPlacement") or "rail") ~= DEFAULTS.computerPreviewPlacement and editable()
							end, reset_button("computer-placement-reset", t("placement.reset"), function()
								save({ computerPreviewPlacement = DEFAULTS.computerPreviewPlacement })
							end)),
							engine_choice(t("placement.label"), function(): any
								return field("computerPreviewPlacement") or "rail"
							end, function(): any
								return peek_field("computerPreviewPlacement") or "rail"
							end, function(v: any)
								save({ computerPreviewPlacement = v })
							end, { { value = "card", label = t("placement.card") }, { value = "rail", label = t("placement.rail") } }),
						},
					}),
					row("sounds", {
						label = t("sounds.label"),
						description = t("sounds.description"),
						stacked = true,
						control = engine_choice(t("sounds.label"), function(): any
							return M.click_sounds:get()
						end, function(): any
							return M.click_sounds:peek()
						end, function(v: any)
							M.click_sounds:set(v)
						end, {
							{ value = "foreground", label = t("sounds.foreground") },
							{ value = "foreground-and-background", label = t("sounds.all") },
							{ value = "off", label = t("sounds.off") },
						}),
					}),
				},
			},
			ds.Settings.Section {
				title = t("developer.title"),
				description = t("developer.description"),
				trail = search.anchor("developer", engine_switch(t("developer.label"), "browserDeveloperAccess")),
			},
			ds.Settings.Section {
				title = t("allowed.title"),
				description = t("allowed.description"),
				search.anchor("allowed", allowed_apps()),
			},
			guide(),
			ds.Settings.Section {
				title = t("advanced.title"),
				trail = ds.Button {
					function(): string
						return t(if advanced_open:get() then "advanced.hide" else "advanced.show"):get()
					end,
					id = "computer-advanced-toggle",
					tone = "ghost",
					size = "sm",
					on_click = function()
						advanced_open:set(not advanced_open:peek())
					end,
				},
				ui.show(function(): boolean
					return advanced_open:get()
				end, ds.Settings.Group {
					row("driver", {
						label = t("advanced.driver"),
						description = function(): string
							return (status:get() or {}).driver or ""
						end,
						detail = function(): string?
							return (status:get() or {}).host
						end,
					}),
				}),
			},
		}),
	}
end

return M
