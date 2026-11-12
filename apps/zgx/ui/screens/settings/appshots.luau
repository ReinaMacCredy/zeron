-- Settings → Appshots: the shipping app's page (crates/ui settings/
-- appshots.rs, Synara's AppSnap panel): an intro card; Capture (Enable,
-- the hotkey chord, Destination, Capture sound); Quick capture's chord; and
-- macOS permissions (Input Monitoring, Screen Recording, Accessibility),
-- each with Grant, which asks macOS, opens its pane and unfolds three steps
-- while the page watches every 2 s for the grant; then Recheck. The
-- choices are the engine's ui-settings.json, shared with the shipping app
-- (screens/chat/appshots, src/appshots).

local ds = require("@ds")
local status_ds = require("@ds/status")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local Appshots = require("@app/screens/chat/appshots")

local M = {}

M.ID = "appshots"
M.TITLE = "Appshots"

local function t(key: string, args: { [string]: any }?): Message
	return ui.t(`settings.appshots.{key}`, args)
end

search.index(M.ID, t("title"), t("terms"), {
	{ key = "overview", title = t("intro.title"), terms = t("intro.terms") },
	{ key = "enabled", title = t("enabled.label"), section = t("capture_section"), terms = t("enabled.terms") },
	{ key = "hotkey", title = t("hotkey.label"), section = t("capture_section"), terms = t("hotkey.terms") },
	{ key = "destination", title = t("destination.label"), section = t("capture_section"), terms = t("destination.terms") },
	{ key = "sound", title = t("sound.label"), section = t("capture_section"), terms = t("sound.terms") },
	{ key = "quick", title = t("quick.label"), section = t("quick_section"), terms = t("quick.terms") },
	{ key = "input", title = t("input.label"), section = t("permissions"), terms = t("input.terms") },
	{ key = "capture", title = t("capture.label"), section = t("permissions"), terms = t("capture.terms") },
	{ key = "text", title = t("text.label"), section = t("permissions"), terms = t("text.terms") },
	{ key = "status", title = t("status_row.label"), section = t("permissions"), terms = t("status_row.terms") },
})

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

local function state(): any
	return Appshots.state:get() or {}
end
local function app(): string
	local a = state().app
	return if type(a) == "string" and a ~= "" then a else "Zeron"
end
local function mac(): boolean
	local p = state().platform
	return p == nil or p == "macos" or p == true
end
local function supported(): boolean
	local p = state().platform
	return p ~= false and p ~= "unsupported"
end
local function chord(combo: string): boolean
	return Appshots.glyph(combo) ~= nil
end

local function granted(pane: string): boolean
	return state()[pane] == "ready"
end

local open_pane = ui.state("settings_appshots_guide", nil :: string?)
local prompted = ui.state("settings_appshots_prompted", {} :: { [string]: boolean })
local notice = ui.state("settings_appshots_notice", nil :: string?)
local rechecking = ui.state("settings_appshots_rechecking", false)
local refused = ui.state("settings_appshots_refused", nil :: any)

local function prompt(pane: string)
	local p = table.clone(prompted:peek())
	p[pane] = true
	prompted:set(p)
end

local polling = 0
local function open_guide(pane: string)
	notice:set(nil)
	open_pane:set(pane)
	polling += 1
	local mine = polling
	-- (macOS says nothing when a grant changes: an open guide asks every 2 s.)
	ui.every(2000, function(): boolean?
		if polling ~= mine or open_pane:peek() ~= pane then
			return false
		end
		Appshots.refresh()
		return nil
	end)
end
local function close_guide()
	open_pane:set(nil)
	polling += 1
end

ui.effect(function()
	local pane = open_pane:get()
	if pane and granted(pane) then
		close_guide()
		notice:set(t("now_granted", { pane = t(`{pane}.label`):get() }):get())
	end
end)

local function enable_status(): string
	local st = state()
	if not supported() then
		return t("status.unsupported"):get()
	elseif not Appshots.enabled:get() then
		return t("status.off"):get()
	end
	local combo = Appshots.hotkey:get()
	if combo == Appshots.NO_HOTKEY then
		return t("status.no_hotkey"):get()
	elseif chord(combo) and st.input == "required" then
		return t("status.input"):get()
	elseif st.capture == "required" or st.capture == "missing" then
		return t("status.capture"):get()
	elseif st.shortcut_ready ~= true then
		return t("status.unavailable"):get()
	end
	return t("status.listening", { keys = Appshots.label(combo) }):get()
end

-- Enabling asks macOS once for the first grant missing (Input Monitoring
-- for a chord, then Screen Recording) and opens its guide.
local function set_enabled(on: boolean)
	Appshots.enabled:set(on)
	if not on then
		return
	end
	local st = state()
	local missing = if chord(Appshots.hotkey:peek()) and st.input == "required"
		then "input"
		elseif st.capture == "required" then "capture"
		else nil
	if missing then
		if not prompted:peek()[missing] then
			prompt(missing)
			Appshots.request(missing)
		end
		if state()[missing] ~= "ready" then
			open_guide(missing)
		end
	end
end

local function reset(id: string, what: string, shown: () -> boolean, back: () -> ()): Node
	return ui.show(shown, ds.IconButton {
		id = id,
		icon = "restart",
		size = "sm",
		tone = "quiet",
		label = function(): string
			return t("reset", { what = t(`reset_what.{what}`):get() }):get()
		end,
		on_click = back,
	})
end

local function customized(): boolean
	local d = Appshots.DEFAULTS
	return Appshots.enabled:get() ~= d.enabled
		or Appshots.sound:get() ~= d.sound
		or Appshots.hotkey:get() ~= d.hotkey
		or Appshots.quick_capture:get() ~= d.quick_capture
		or Appshots.destination:get() ~= d.destination
end

local function restore(): Node
	return ds.Button {
		id = "appshots-restore-defaults",
		tone = "ghost",
		size = "sm",
		label = t("restore_label"),
		disabled = function(): boolean
			return not customized()
		end,
		on_click = function()
			refused:set(nil)
			ui.batch(function()
				for k, v in Appshots.DEFAULTS :: { [string]: any } do
					(Appshots :: any)[k]:set(v)
				end
			end)
		end,
		ds.Icon { name = "restart", size = 14 },
		t("restore"),
	} :: any
end

local CHOICES = { "command-command", "option-option", "shift-shift", "none" }
local function hotkey_menu(which: "appshot" | "quick"): Node
	local mine: State<string> = if which == "appshot" then Appshots.hotkey else Appshots.quick_capture
	local other: State<string> = if which == "appshot" then Appshots.quick_capture else Appshots.hotkey
	local shown = ui.use_state(mine:peek())
	ui.effect(function()
		shown:set(mine:get())
	end)
	return ds.Select {
		value = shown,
		w = 124,
		name = function(): string
			local whose = if which == "appshot" then t("hotkey.appshot"):get() else t("quick.label"):get()
			return t("hotkey.name", { chord = whose, choice = Appshots.label(mine:get()) }):get()
		end,
		options = function(): { any }
			local out = {}
			for _, c in CHOICES do
				table.insert(out, { value = c, label = Appshots.label(c) })
			end
			return out
		end,
		on_change = function(v: any)
			if chord(v) and other:peek() == v then
				refused:set({
					which = which,
					text = t("hotkey.taken", {
						choice = Appshots.label(v),
						other = if which == "appshot" then t("quick.label"):get() else t("hotkey.appshot"):get(),
					}):get(),
				})
				shown:set(mine:peek())
				return
			end
			refused:set(nil)
			mine:set(v)
		end,
	} :: any
end
local function refusal(which: string): () -> string?
	return function(): string?
		local r = refused:get()
		return if r and r.which == which then r.text else nil
	end
end

local function badge(pane: string): Node
	return ui.row {
		gap = ui.raw(6, "the shipping app's badge: dot, then its word"),
		items = "center",
		ds.Dot {
			size = 6,
			color = function(): string
				if granted(pane) then
					return "$zeron.tone.success.ink"
				end
				return if prompted:get()[pane] then "$zeron.tone.danger.ink" else "$border"
			end :: any,
		},
		ui.text {
			function(): string
				if granted(pane) then
					return t("badge.granted"):get()
				end
				return if prompted:get()[pane] then t("badge.denied"):get() else t("badge.not_requested"):get()
			end,
			text_size = "$font.size.xs",
			weight = "$font.weight.medium",
			color = "$text.secondary",
		},
	}
end

local function step(n: number, body: Node): Node
	return ui.row {
		gap = "$space.2.5",
		items = "start",
		ui.div {
			w = 20,
			h = 20,
			mt = ui.raw(2, "the shipping app's step number, on the first line's middle"),
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = "$radius.full",
			border = 1,
			border_color = "$border",
			ui.text { tostring(n), text_size = "$font.size.xs", weight = "$font.weight.medium", color = "$text.secondary" },
		},
		ui.div { grow = 1, min_w = 0, min_h = 24, justify = "center", body },
	}
end

local function words(m: Message | () -> string): Node
	return ui.text { m, wrap = true, text_size = "$font.size.sm", color = "$text.secondary" }
end

local function guide(pane: string): Node
	return ui.column {
		id = `appshots-guide-{pane}`,
		gap = "$space.3",
		p = "$space.4",
		mt = "$space.2",
		rounded = "$radius.md",
		border = 1,
		border_color = "$border",
		bg = "$background",
		ui.column {
			gap = "$space.2.5",
			step(
				1,
				ui.row {
					ds.Button {
						id = `appshots-guide-open-{pane}`,
						tone = "secondary",
						size = "sm",
						on_click = function()
							Appshots.open_settings(pane)
						end,
						t("guide.open", { pane = t(`{pane}.label`):get() }),
					} :: any,
				}
			),
			step(2, words(function(): string
				return t("guide.listed", { app = app() }):get()
			end)),
			step(3, words(t("guide.auth"))),
		},
		ui.row {
			gap = "$space.2",
			items = "center",
			pt = "$space.3",
			border = 1,
			border_sides = { 1, 0, 0, 0 },
			border_color = "$border",
			ui.show(function(): boolean
				return not granted(pane)
			end, status_ds.Spinner { size = 12 }),
			ui.text {
				function(): string
					return if granted(pane) then t("guide.granted"):get() else t("guide.watching"):get()
				end,
				text_size = "$font.size.sm",
				color = function(): string
					return if granted(pane) then "$zeron.tone.success.ink" else "$text.secondary"
				end,
			},
		},
		words(t("guide.stale")),
	}
end

local function permission(pane: string): Node
	return search.anchor(
		pane,
		ui.column {
			ds.Settings.Row {
				label = t(`{pane}.label`),
				description = function(): string
					return t(`{pane}.description`, { app = app() }):get()
				end,
				control = ui.row {
					gap = "$space.2",
					items = "center",
					badge(pane),
					ui.show(
						function(): boolean
							return not granted(pane)
						end,
						ds.Button {
							id = `appshots-grant-{pane}`,
							tone = "secondary",
							size = "sm",
							label = function(): string
								local action = if open_pane:get() == pane then t("hide_steps"):get() else t("grant"):get()
								return t("grant_label", { action = action, pane = t(`{pane}.label`):get() }):get()
							end,
							on_click = function()
								if open_pane:peek() == pane then
									close_guide()
									return
								end
								prompt(pane)
								-- (Asking first lists this copy in the pane; then the pane opens.)
								Appshots.request(pane, true)
								open_guide(pane)
							end,
							ui.text(function(): string
								return if open_pane:get() == pane then t("hide_steps"):get() else t("grant"):get()
							end),
						} :: any
					),
				},
			},
			ui.show(function(): boolean
				return open_pane:get() == pane
			end, ui.div { px = ds.Settings.inset(), pb = "$space.3", guide(pane) }),
		}
	)
end

local function recheck()
	if rechecking:peek() then
		return
	end
	notice:set(nil)
	rechecking:set(true)
	-- (The check is at once; the short hold keeps "Rechecking…" readable.)
	ui.after(350, function()
		Appshots.refresh()
		rechecking:set(false)
		local missing = false
		for _, pane in { "input", "capture" } do
			if (pane ~= "input" or chord(Appshots.hotkey:peek()) or chord(Appshots.quick_capture:peek())) and not granted(pane) then
				missing = true
			end
		end
		notice:set(if missing then t("unchanged"):get() else nil)
	end)
end

M.view = ui.component(function(_: {}): Node
	ui.on_mount(Appshots.refresh)
	ui.on_cleanup(close_guide)
	local function destination_description(): string
		local d = Appshots.destination:get()
		if d == "last-session" then
			return t("destination.last_description"):get()
		elseif d == "new-session" then
			return t("destination.new_description"):get()
		end
		return t("destination.automatic_description", { app = app() }):get()
	end
	local d = Appshots.DEFAULTS
	return ds.Settings.Page {
		title = t("title"),
		description = t("description"),
		trail = restore(),
		search.anchor(
			"overview",
			ds.Settings.Group {
				ds.Settings.Row {
					lead = ui.div {
						w = 32,
						h = 32,
						shrink = 0,
						items = "center",
						justify = "center",
						rounded = "$radius.md",
						border = 1,
						border_color = "$border",
						color = "$text.secondary",
						ds.Icon { name = "screen-capture", size = 16 },
					},
					label = t("intro.title"),
					description = function(): string
						return t("intro.body", { app = app() }):get()
					end,
					detail = function(): string?
						return if supported() then nil else t("intro.desktop_only"):get()
					end,
				},
			}
		),
		ds.Settings.Section {
			title = t("capture_section"),
			ds.Settings.Group {
				row("enabled", {
					label = t("enabled.label"),
					description = function(): string
						return t("enabled.description", { app = app() }):get()
					end,
					detail = enable_status,
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-enabled-reset", "enabled", function(): boolean
							return Appshots.enabled:get() ~= d.enabled
						end, function()
							Appshots.enabled:set(false)
						end),
						ds.Switch {
							value = Appshots.enabled,
							label = t("enabled.label"),
							disabled = function(): boolean
								return not supported()
							end,
							on_change = function(on: boolean)
								set_enabled(on)
							end,
						},
					},
				}),
				row("hotkey", {
					label = t("hotkey.label"),
					description = function(): string
						local g = Appshots.glyph(Appshots.hotkey:get())
						return if g then t("hotkey.description", { glyph = g }):get() else t("hotkey.none_description"):get()
					end,
					detail = refusal("appshot"),
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-hotkey-reset", "hotkey", function(): boolean
							return Appshots.hotkey:get() ~= d.hotkey
						end, function()
							refused:set(nil)
							Appshots.hotkey:set(d.hotkey)
						end),
						hotkey_menu("appshot"),
					},
				}),
				row("destination", {
					label = t("destination.label"),
					-- (Beside a three-line description the chips overlap it: Row's text keeps 350 and the row doesn't wrap them under.)
					stacked = true,
					description = destination_description,
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-destination-reset", "destination", function(): boolean
							return Appshots.destination:get() ~= d.destination
						end, function()
							Appshots.destination:set(d.destination)
						end),
						ds.Segmented {
							id = "appshots-destination",
							value = Appshots.destination,
							label = t("destination.label"),
							options = {
								{ value = "automatic", label = t("destination.automatic") },
								{ value = "last-session", label = t("destination.last") },
								{ value = "new-session", label = t("destination.new") },
							},
						},
					},
				}),
				row("sound", {
					label = t("sound.label"),
					description = t("sound.description"),
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-sound-reset", "sound", function(): boolean
							return Appshots.sound:get() ~= d.sound
						end, function()
							Appshots.sound:set(true)
						end),
						ds.Button {
							id = "appshots-sound-preview",
							tone = "secondary",
							size = "sm",
							label = t("preview_label"),
							on_click = function()
								pcall(host.invoke, "sound.play", "appshot")
							end,
							t("preview"),
						} :: any,
						ds.Switch { value = Appshots.sound, label = t("sound.label") },
					},
				}),
			},
		},
		ds.Settings.Section {
			title = t("quick_section"),
			ds.Settings.Group {
				row("quick", {
					label = t("quick.label"),
					description = function(): string
						local g = Appshots.glyph(Appshots.quick_capture:get())
						return if g then t("quick.description", { glyph = g }):get() else t("quick.none_description"):get()
					end,
					detail = function(): string?
						local r = refusal("quick")()
						if r then
							return r
						end
						return if chord(Appshots.quick_capture:get()) and state().input == "required" then t("quick.input"):get() else nil
					end,
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-quick-reset", "quick", function(): boolean
							return Appshots.quick_capture:get() ~= d.quick_capture
						end, function()
							refused:set(nil)
							Appshots.quick_capture:set(d.quick_capture)
						end),
						hotkey_menu("quick"),
					},
				}),
			},
		},
		ui.show(
			supported,
			ds.Settings.Section {
				title = t("permissions"),
				ds.Settings.Group {
					ui.show(mac, permission("input")),
					permission("capture"),
					permission("text"),
					row("status", {
						label = t("status_row.label"),
						description = t("status_row.description"),
						detail = function(): string?
							return notice:get()
						end,
						control = ds.Button {
							id = "appshots-refresh-permissions",
							tone = "secondary",
							size = "sm",
							busy = rechecking,
							label = t("recheck"),
							on_click = recheck,
							ui.text(function(): string
								return if rechecking:get() then t("rechecking"):get() else t("recheck"):get()
							end),
						} :: any,
					}),
				},
			}
		),
	}
end)

return M
