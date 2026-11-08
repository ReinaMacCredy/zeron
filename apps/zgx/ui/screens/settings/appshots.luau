-- Settings → Appshots: the shipping app's page (crates/ui settings/
-- appshots.rs), its rows and words: Appshots on or off, their sound, the
-- global shortcut (recorded here), where a capture lands, and the two
-- permissions, each with its state and the way to grant it. The host checks
-- the permissions (src/appshots); "Check again" asks it afresh after a trip
-- to System Settings.

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local Appshots = require("@app/screens/chat/appshots")

local M = {}

M.ID = "appshots"
M.TITLE = "Appshots"

search.index(M.ID, ui.t("settings.appshots.title"), ui.t("settings.appshots.terms"), {
	{ key = "enabled", title = ui.t("settings.appshots.enabled.label"), terms = ui.t("settings.appshots.enabled.terms") },
	{ key = "sound", title = ui.t("settings.appshots.sound.label"), terms = ui.t("settings.appshots.sound.terms") },
	{ key = "shortcut", title = ui.t("settings.appshots.shortcut.label"), terms = ui.t("settings.appshots.shortcut.terms") },
	{ key = "destination", title = ui.t("settings.appshots.destination.label"), terms = ui.t("settings.appshots.destination.terms") },
	{ key = "capture", title = ui.t("settings.appshots.capture.label"), terms = ui.t("settings.appshots.capture.terms") },
	{ key = "text", title = ui.t("settings.appshots.text.label"), terms = ui.t("settings.appshots.text.terms") },
})

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

local function state(): any
	return Appshots.state:get() or {}
end

-- A capability's badge: "Ready", or "Required" / "Set up".
local function badge(ready: () -> boolean, missing: Message): Node
	return ui.show(ready, ds.Badge { ui.t("settings.appshots.ready"), tone = "success" }, ds.Badge { missing, tone = "warning" })
end

-- The shortcut, as its keys show it ("⌃⌥Space").
local function shown(combo: string): string
	return ui.shortcut_text(combo)
end

-- gpui's defaults (settings.rs, appshots.rs): off, sound on, its hotkey, Automatic.
local DEFAULTS = { enabled = false, sound = true, shortcut = "ctrl+alt+space", destination = "automatic" }

local function customized(): boolean
	return Appshots.enabled:get() ~= DEFAULTS.enabled
		or Appshots.sound:get() ~= DEFAULTS.sound
		or Appshots.shortcut:get() ~= DEFAULTS.shortcut
		or Appshots.destination:get() ~= DEFAULTS.destination
end

-- A row's reset glyph, while its setting is off its default.
local function reset(id: string, what: Message, setting: State<any>, default: any): Node
	return ui.show(function(): boolean
		return setting:get() ~= default
	end, ds.IconButton {
		id = id,
		icon = "restart",
		size = "sm",
		tone = "quiet",
		label = function(): string
			return ui.t("settings.appshots.reset", { what = what:get() }):get()
		end,
		on_click = function()
			setting:set(default)
		end,
	})
end

local function restore(): Node
	return ds.Button {
		id = "appshots-restore-defaults",
		tone = "ghost",
		size = "sm",
		disabled = function(): boolean
			return not customized()
		end,
		on_click = function()
			for k, v in DEFAULTS do
				(Appshots :: any)[k]:set(v)
			end
		end,
		ds.Icon { name = "restart", size = 14 },
		ui.t("settings.appshots.restore"),
	} :: any
end

-- The Enable row's line under it (gpui's `enable_status`).
local function status(): string
	local st = state()
	if st.platform == false then
		return ui.t("settings.appshots.status.unsupported"):get()
	elseif not Appshots.enabled:get() then
		return ui.t("settings.appshots.status.off"):get()
	elseif st.capture == "missing" then
		return ui.t("settings.appshots.status.capture"):get()
	elseif st.shortcut_ready ~= true then
		return ui.t("settings.appshots.status.unavailable"):get()
	end
	return ui.t("settings.appshots.status.listening", { keys = shown(Appshots.shortcut:get()) }):get()
end

local function preview(): Node
	return ds.Button {
		id = "appshots-sound-preview",
		tone = "secondary",
		size = "sm",
		on_click = function()
			pcall(host.invoke, "sound.play", "appshot")
		end,
		ui.t("settings.appshots.preview"),
	} :: any
end

-- A combination from a key pressed, or nil (as the shipping app takes them:
-- a letter, a digit, Space, Tab, Enter, an arrow, Home/End/Page, F1–F20,
-- with Control, Option or Command held).
local NAMED: { [string]: string } = {
	[" "] = "space",
	Tab = "tab",
	Enter = "enter",
	ArrowUp = "up",
	ArrowDown = "down",
	ArrowLeft = "left",
	ArrowRight = "right",
	Home = "home",
	End = "end",
	PageUp = "pageup",
	PageDown = "pagedown",
}
local function combo_of(e: KeyEvent): string?
	if not (e.ctrl or e.alt or e.meta) then
		return nil
	end
	local key = NAMED[e.key]
	if not key and string.match(e.key, "^F%d+$") then
		local n = tonumber(string.sub(e.key, 2)) or 0
		key = if n >= 1 and n <= 20 then string.lower(e.key) else nil
	end
	if not key and e.code then
		-- (The physical key, so ⌥ doesn't turn a letter into a symbol.)
		local letter = string.match(e.code, "^Key(%a)$") or string.match(e.code, "^Digit(%d)$")
		key = if letter then string.lower(letter) else nil
	end
	if not key and string.match(e.key, "^%w$") then
		key = string.lower(e.key)
	end
	if not key then
		return nil
	end
	local parts = {}
	if e.ctrl then
		table.insert(parts, "ctrl")
	end
	if e.alt then
		table.insert(parts, "alt")
	end
	if e.shift then
		table.insert(parts, "shift")
	end
	if e.meta then
		table.insert(parts, "cmd")
	end
	table.insert(parts, key)
	return table.concat(parts, "+")
end

-- The shortcut's recorder: its keys; pressed, it listens for a new
-- combination (the shortcut let go meanwhile, so pressing it doesn't
-- capture); Escape puts it back as it was.
-- (gpui's refusal for a key with no modifier: said in place of "Press a shortcut…".)
local BARE = { Shift = true, Control = true, Alt = true, Meta = true, CapsLock = true }
local Recorder = ui.component(function(_: {}): Node
	local listening = ui.use_state(false)
	local refused = ui.use_state(false)
	local function stop()
		listening:set(false)
		Appshots.set_recording(false)
	end
	return ds.Button {
		tone = "secondary",
		size = "sm",
		label = function(): string
			return if listening:get() then ui.t("settings.appshots.press"):get() else ui.t("settings.appshots.change_shortcut", { keys = shown(Appshots.shortcut:get()) }):get()
		end,
		on_click = function()
			refused:set(false)
			listening:set(true)
			Appshots.set_recording(true)
		end,
		on_blur = function()
			if listening:peek() then
				stop()
			end
		end,
		on_key = function(e: KeyEvent): boolean?
			if not listening:peek() then
				return nil
			end
			if e.key == "Escape" then
				stop()
				return true
			end
			local combo = combo_of(e)
			if combo then
				Appshots.shortcut:set(combo)
				stop()
			elseif not BARE[e.key] then
				refused:set(true)
			end
			return true
		end,
		ui.text {
			function(): string
				if listening:get() and refused:get() then
					return ui.t("settings.appshots.key_rule"):get()
				end
				return if listening:get() then ui.t("settings.appshots.press_now"):get() else shown(Appshots.shortcut:get())
			end,
		},
	} :: any
end)

-- A permission's button: asks first, then opens System Settings at it.
local function grant(which: string, first: Message): Node
	local asked = ui.use_state(false)
	return ds.Button {
		tone = "secondary",
		size = "sm",
		on_click = function()
			Appshots.request(which)
			asked:set(true)
		end,
		ui.text(function(): string
			return if asked:get() then ui.t("settings.appshots.open_system_settings"):get() else first:get()
		end),
	} :: any
end

M.view = ui.component(function(_: {}): Node
	-- (Read afresh as the page opens.)
	ui.on_mount(Appshots.refresh)
	local function capture_ready(): boolean
		return state().capture == "ready"
	end
	local function text_ready(): boolean
		return state().text == "ready"
	end
	local function shortcut_ready(): boolean
		return state().shortcut_ready == true
	end
	return ds.Settings.Page {
		title = ui.t("settings.appshots.title"),
		description = ui.t("settings.appshots.description"),
		trail = restore(),
		ds.Settings.Group {
			ds.Settings.Row {
				icon = "gallery",
				label = ui.t("settings.appshots.intro.title"),
				description = ui.t("settings.appshots.intro.body"),
				detail = function(): string?
					return if state().platform == false then ui.t("settings.appshots.intro.desktop_only"):get() else nil
				end,
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.appshots.capture_section"),
			ds.Settings.Group {
				row("enabled", {
					label = ui.t("settings.appshots.enabled.label"),
					description = ui.t("settings.appshots.enabled.description"),
					detail = status,
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-enabled-reset", ui.t("settings.appshots.reset_what.enabled"), Appshots.enabled, DEFAULTS.enabled),
						ds.Switch { value = Appshots.enabled, label = ui.t("settings.appshots.enabled.label") },
					},
				}),
				row("sound", {
					label = ui.t("settings.appshots.sound.label"),
					description = ui.t("settings.appshots.sound.description"),
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-sound-reset", ui.t("settings.appshots.reset_what.sound"), Appshots.sound, DEFAULTS.sound),
						preview(),
						ds.Switch { value = Appshots.sound, label = ui.t("settings.appshots.sound.label") },
					},
				}),
				row("shortcut", {
					label = ui.t("settings.appshots.shortcut.label"),
					description = function(): string
						return if shortcut_ready()
							then ui.t("settings.appshots.shortcut.ready"):get()
							else ui.t("settings.appshots.shortcut.unavailable"):get()
					end,
					control = ui.row {
						gap = "$space.2",
						items = "center",
						reset("appshots-shortcut-reset", ui.t("settings.appshots.reset_what.shortcut"), Appshots.shortcut, DEFAULTS.shortcut),
						badge(shortcut_ready, ui.t("settings.appshots.set_up")),
						Recorder {},
					},
				}),
				row("destination", {
					label = ui.t("settings.appshots.destination.label"),
					description = function(): string
						local d = Appshots.destination:get()
						if d == "last" then
							return ui.t("settings.appshots.destination.last_description"):get()
						elseif d == "new" then
							return ui.t("settings.appshots.destination.new_description"):get()
						end
						return ui.t("settings.appshots.destination.automatic_description"):get()
					end,
					control = ui.row { gap = "$space.2", items = "center", reset("appshots-destination-reset", ui.t("settings.appshots.reset_what.destination"), Appshots.destination, DEFAULTS.destination), ds.Select {
						value = Appshots.destination,
						name = ui.t("settings.appshots.destination.label"),
						w = 148,
						-- (Read in the locale shown: they follow it.)
						options = function(): { any }
							return {
								{ value = "automatic", label = ui.t("settings.appshots.destination.automatic"):get() },
								{ value = "last", label = ui.t("settings.appshots.destination.last"):get() },
								{ value = "new", label = ui.t("settings.appshots.destination.new"):get() },
							}
						end,
					} },
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.appshots.permissions"),
			description = ui.t("settings.appshots.permissions_description"),
			ds.Settings.Group {
				row("capture", {
					label = ui.t("settings.appshots.capture.label"),
					description = ui.t("settings.appshots.capture.description"),
					control = ui.row {
						gap = "$space.2",
						items = "center",
						badge(capture_ready, ui.t("settings.appshots.required")),
						ui.show(function(): boolean
							return not capture_ready()
						end, grant("capture", ui.t("settings.appshots.capture.allow"))),
					},
				}),
				row("text", {
					label = ui.t("settings.appshots.text.label"),
					description = ui.t("settings.appshots.text.description"),
					control = ui.row {
						gap = "$space.2",
						items = "center",
						badge(text_ready, ui.t("settings.appshots.required")),
						ui.show(function(): boolean
							return not text_ready()
						end, grant("text", ui.t("settings.appshots.text.allow"))),
					},
				}),
			},
		},
		ui.row {
			gap = "$space.3",
			items = "center",
			pl = ds.Settings.inset(),
			ui.text { ui.t("settings.appshots.changed"), text_size = "$font.size.sm", color = "$text.secondary" },
			ds.Button { tone = "ghost", size = "sm", on_click = Appshots.refresh, ui.t("settings.appshots.check_again") },
		},
	}
end)

return M
