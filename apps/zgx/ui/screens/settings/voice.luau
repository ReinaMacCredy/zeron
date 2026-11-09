-- Settings → Voice: gpui's page (crates/ui/src/dictation/model.rs). Voice
-- conversation: Codex voice device (this device, or another of yours that
-- can stream audio to Codex there) and style. Dictation: the switch (the
-- speech model downloaded first, its progress and Cancel on the switch's
-- line), the microphone (a saved one unplugged stays listed as
-- Disconnected), and the hold shortcut. Speech model: Parakeet v3 and
-- Remove. Words only ever leave this computer: the host records and
-- transcribes here (src/voice.rs).
-- Listed while the Voice built-in is on (Settings → Plugins). A host
-- without these calls (older than this page), or a build with no recorder
-- (`capable` false), says Dictation isn't available here and downloads
-- nothing.

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local Dictation = require("@ds/dictation")
local Recorder = require("@ds/shortcut_recorder")

local M = {}

M.ID = "voice"
M.TITLE = "Voice"

local function t(key: string, args: { [string]: any }?): Message
	return ui.t(`settings.voice.{key}`, args)
end

search.index(M.ID, t("title"), t("terms"), {
	{ key = "device", title = t("device.label"), section = t("conversation"), terms = t("device.terms") },
	{ key = "style", title = t("style.label"), section = t("conversation"), terms = t("style.terms") },
	{ key = "dictation", title = t("dictation.label"), section = t("dictation.section"), terms = t("dictation.terms") },
	{ key = "microphone", title = t("mic.label"), section = t("dictation.section"), terms = t("mic.terms") },
	{ key = "shortcut", title = t("shortcut.label"), section = t("dictation.section"), terms = t("shortcut.terms") },
	{ key = "model", title = t("model.label"), section = t("model.section"), terms = t("model.terms") },
})

-- Codex's voice (gpui's `codexVoice`, `codexVoiceDevice`): nil, its default and this device.
M.codex_voice = ui.persist("codex_voice", "")
M.codex_voice_device = ui.persist("codex_voice_device", "")
-- (gpui's `zeron_proto::voice::DEFAULT_VOICES`, until a live session names its own.)
M.VOICES = { "juniper", "maple", "spruce", "ember", "vale", "breeze", "arbor", "sol", "cove" }

local inputs = ui.state("settings_voice_inputs", { devices = {}, default = nil } :: any)
local remote = ui.state("settings_voice_remote", {} :: { [string]: boolean })
local error_word = ui.state("settings_voice_error", nil :: string?)
-- A download the switch started: Dictation turns on once the model is here (gpui's `pending_enable`).
local enable_after = ui.state("settings_voice_enable_after", false)

local function words(err: any): string
	local w = if type(err) == "table" then err.message else err
	if w == "busy" then
		return t("errors.busy"):get()
	elseif w == "remove_failed" or w == "download_failed" or w == "cancelled" or w == "interrupted" then
		return t(`errors.{w}`):get()
	end
	return if type(w) == "string" and w ~= "" then w else ui.t("common.something_went_wrong"):get()
end

-- A host call's promise, or nil from a host without it.
local function ask(name: string, arg: any?): any
	local ok, p = pcall(host.call, name, arg)
	return if ok then p else nil
end
local function tell(name: string, arg: any?): boolean
	return (pcall(host.invoke, name, arg))
end

-- (Only a list that changed: the same one again redraws nothing.)
local function same_inputs(a: any, b: any): boolean
	if a.default ~= b.default or #(a.devices or {}) ~= #(b.devices or {}) then
		return false
	end
	for i, d in a.devices or {} do
		local e = b.devices[i]
		if d.id ~= e.id or d.name ~= e.name then
			return false
		end
	end
	return true
end
local function heard_inputs(v: any)
	if type(v) == "table" and not same_inputs(inputs:peek(), v) then
		inputs:set(v)
	end
end
local function ask_inputs()
	local p = ask("voice.inputs")
	if p then
		p:after(heard_inputs, function() end)
	end
end

local function load()
	local p = ask("voice.model")
	if p then
		p:after(function(v: any)
			if v ~= nil then
				Dictation.model:set(v)
			end
		end, function() end)
	elseif Dictation.model:peek() == nil then
		Dictation.model:set({ ready = false, cache = false, size = 0, downloading = false, progress = 0, capable = false })
	end
	ask_inputs()
end
M.load = load

local function mb(bytes: number?): string
	return tostring(math.floor((bytes or 0) / 1e6 + 0.5))
end

local function model(): any
	return Dictation.model:get() or {}
end

-- Whether this build can dictate at all (a host with a recorder).
local function capable(): boolean
	return model().capable ~= false
end

-- Why Dictation's hold shortcut can't be `combo`: the composer's own keys,
-- or a command's (the shell's, the palette's, a plugin's), in gpui's words.
local EDIT_KEYS = { a = true, c = true, v = true, x = true, y = true, z = true }
local MOVE_KEYS = { left = true, right = true, up = true, down = true, home = true, ["end"] = true }
function M.refusal(combo: string): string?
	if combo == "" then
		return nil
	end
	local forms = { combo }
	local ok, platform = pcall(function(): any
		return ui.shared("platform", ""):peek()
	end)
	if not (ok and platform == "macos") then
		local mac = string.gsub(combo, "ctrl", "cmd")
		if mac ~= combo then
			table.insert(forms, mac)
		end
	end
	local shown = Recorder.shown(combo)
	for _, c in forms do
		local key = string.match(c, "([^+]+)$") or ""
		local cmd = string.find(c, "cmd", 1, true) ~= nil
		if key == "enter" or (EDIT_KEYS[key] and cmd) or (MOVE_KEYS[key] and (cmd or string.find(c, "alt", 1, true) ~= nil)) then
			return ui.t("settings.voice.shortcut.reserved", { keys = shown }):get()
		end
	end
	local found, shell_keys = pcall(require, "@app/screens/shell/keys")
	for _, c in forms do
		local owner = if ui.shortcut_owner then ui.shortcut_owner(c) else nil
		if owner then
			local name = owner
			if found and type(shell_keys) == "table" then
				local k = (shell_keys :: any).get(owner)
				if k then
					name = (shell_keys :: any).title(k)
				end
			end
			return ui.t("settings.shortcuts.taken", { keys = shown, name = name }):get()
		end
	end
	local has, appshots = pcall(require, "@app/screens/chat/appshots")
	local a = if has and type(appshots) == "table" then appshots :: any else nil
	if a and a.shortcut and a.active and a.active() and a.shortcut:peek() == combo then
		return ui.t("settings.shortcuts.taken_on_page", {
			keys = shown,
			name = ui.t("settings.shortcuts.appshot.label"):get(),
			page = ui.t("settings.appshots.title"):get(),
		}):get()
	end
	return nil
end

-- Why the hold shortcut kept from before collides now (a command given its keys while Voice was off).
function M.conflict(): string?
	return M.refusal(Dictation.key:get())
end

-- The switch: Cancel while downloading, the download until the model is here, then on and off.
local function primary()
	local m = Dictation.model:peek() or {}
	if m.capable == false then
		return
	elseif m.downloading then
		enable_after:set(false)
		tell("voice.cancel")
	elseif m.ready then
		Dictation.enabled:set(not Dictation.enabled:peek())
	else
		error_word:set(nil)
		enable_after:set(true)
		if not tell("voice.download") then
			enable_after:set(false)
		end
	end
end

-- (Heard here, not on the page: a download that ends after the page is left still turns Dictation on.)
ui.effect(function()
	local m = Dictation.model:get()
	if not enable_after:peek() or m == nil or m.downloading then
		return
	end
	enable_after:set(false)
	if m.ready then
		Dictation.enabled:set(true)
	end
end)

-- Voice turned off while the model downloads: the download stops, and nothing turns on after.
ui.effect(function()
	if Dictation.voice_on:get() ~= false then
		return
	end
	enable_after:set(false)
	local m = Dictation.model:peek()
	if m and m.downloading then
		tell("voice.cancel")
	end
end)

local function remove()
	if Dictation.busy:peek() then
		error_word:set(t("errors.busy"):get())
		return
	end
	Dictation.enabled:set(false)
	local p = ask("voice.remove")
	if p == nil then
		error_word:set(t("errors.remove_failed"):get())
		return
	end
	p:after(function(v: any)
		if v ~= nil then
			Dictation.model:set(v)
		end
		error_word:set(nil)
	end, function(err: any)
		error_word:set(words(err))
	end)
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

-- The microphones (gpui's `input_options`): the system's first, each, and a saved one gone as Disconnected.
function M.input_options(list: any, saved: string): { any }
	local default_name = nil
	for _, d in list.devices or {} do
		if d.id == list.default then
			default_name = d.name
		end
	end
	local out = { { value = "", label = t("mic.system"):get(), description = default_name } }
	local found = saved == ""
	for _, d in list.devices or {} do
		table.insert(out, { value = d.id, label = d.name })
		found = found or d.id == saved
	end
	if not found then
		table.insert(out, { value = saved, label = t("mic.disconnected"):get() })
	end
	return out
end

-- Codex voice's devices (gpui's `set_hosts`): this one, then each other of yours, usable when online with remote voice.
local function voice_hosts(): { any }
	local out = { { value = "", label = t("device.this"):get(), description = t("device.local"):get() } }
	local ok, sessions = pcall(require, "@account/sessions")
	local now = if ok then (sessions :: any).now:get() else nil
	local saved = M.codex_voice_device:get()
	local found = saved == ""
	for _, d in (now and now.devices) or {} do
		if not d.here and not d.cloud then
			local compatible = remote:get()[d.id] == true
			local detail = if d.online == false then t("device.offline"):get() elseif not compatible then t("device.update"):get() else t("device.available"):get()
			table.insert(out, { value = d.id, label = d.name, description = detail, disabled = d.online == false or not compatible })
			found = found or d.id == saved
		end
	end
	if not found then
		table.insert(out, { value = saved, label = t("device.unavailable"):get(), description = t("device.choose"):get(), disabled = true })
	end
	return out
end

local function select(value: State<string>, name: Message, options: () -> { any }, write: (v: string) -> ()): Node
	return ui.component(function(): Node
		local chosen = ui.use_state(value:peek())
		ui.effect(function()
			local v = value:get()
			if chosen:peek() ~= v then
				chosen:set(v)
			end
		end)
		return ds.Select {
			value = chosen,
			name = name,
			w = 200,
			options = options,
			on_change = function(v: any)
				if v ~= value:peek() then
					write(v)
				end
			end,
		}
	end)()
end

function M.view(): Node
	visits.once("voice", load)
	-- (While it shows, a microphone plugged in turns up within a few seconds, as gpui's.)
	local stop = ui.every(3000, ask_inputs)
	ui.on_cleanup(stop)
	ui.effect(function()
		local ok, sessions = pcall(require, "@account/sessions")
		local now = if ok then (sessions :: any).now:get() else nil
		local ids = {}
		for _, d in (now and now.devices) or {} do
			if not d.here then
				table.insert(ids, d.id)
			end
		end
		local p = if #ids > 0 then ask("voice.remote", { ids = ids }) else nil
		if p then
			p:after(function(v: any)
				if type(v) == "table" then
					remote:set(v)
				end
			end, function() end)
		end
	end)
	local switch_on = ui.state("settings_voice_switch", false)
	ui.effect(function()
		local m = model()
		local on = m.capable ~= false and (m.downloading == true or (Dictation.enabled:get() == true and m.ready == true))
		if switch_on:peek() ~= on then
			switch_on:set(on)
		end
	end)
	local function status(): string?
		local m = model()
		if m.capable == false then
			return t("dictation.no_recorder"):get()
		elseif m.downloading then
			return t("dictation.downloading", { done = mb(m.progress), total = mb(m.size) }):get()
		elseif not m.ready then
			return t("dictation.download", { total = mb(m.size) }):get()
		end
		return nil
	end
	local dictation_row = row("dictation", {
		label = t("dictation.label"),
		description = function(): string
			return status() or ""
		end,
		control = ui.div {
			id = "voice-enable",
			opacity = function(): number
				return if capable() then 1 else 0.5
			end,
			pointer_events = function(): string
				return if capable() then "auto" else "none"
			end,
			ds.Switch {
				value = switch_on,
				label = t("dictation.label"),
				on_change = function(_: boolean)
					primary()
					-- (The switch follows the model and the setting, not the press.)
					local m = Dictation.model:peek() or {}
					switch_on:set(m.capable ~= false and (m.downloading == true or (Dictation.enabled:peek() == true and m.ready == true)))
				end,
			},
		},
	})
	local progress = ui.show(function(): boolean
		return model().downloading == true
	end, ui.column {
		id = "voice-progress",
		w_full = true,
		px = "$space.4",
		pb = "$space.3",
		ds.ProgressBar {
			value = function(): number?
				local m = model()
				return if (m.size or 0) > 0 then math.clamp((m.progress or 0) / m.size, 0, 1) else nil
			end,
			label = t("dictation.label"),
		},
	})
	local mic_row = ui.show(function(): boolean
		return Dictation.enabled:get() == true and model().ready == true and capable()
	end, ui.column {
		w_full = true,
		ds.Settings.Rule(),
		row("microphone", {
			label = t("mic.label"),
			control = select(Dictation.input, t("mic.label"), function(): { any }
				return M.input_options(inputs:get(), Dictation.input:get())
			end, function(v: string)
				Dictation.input:set(v)
			end),
		}),
	})
	-- (A refused combination says whose it is, as Settings → Shortcuts does.)
	local notice = ui.state("settings_voice_shortcut_notice", nil :: string?)
	local shortcut_row = row("shortcut", {
		label = t("shortcut.label"),
		description = function(): string
			return notice:get() or M.conflict() or t("shortcut.hint"):get()
		end,
		control = Recorder.Recorder {
			value = function(): string
				return Dictation.key:get()
			end,
			label = t("shortcut.label"),
			on_listening = function(on: boolean)
				if on then
					notice:set(nil)
				end
			end,
			on_change = function(combo: string)
				local why = M.refusal(combo)
				notice:set(why)
				if why == nil then
					Dictation.key:set(combo)
				end
			end,
		},
	})
	return ds.Settings.Page {
		title = t("title"),
		description = t("subtitle"),
		ds.Settings.Section {
			title = t("conversation"),
			ds.Settings.Group {
				ui.column {
					w_full = true,
					row("device", {
						label = t("device.label"),
						description = t("device.hint"),
						control = select(M.codex_voice_device, t("device.label"), voice_hosts, function(v: string)
							M.codex_voice_device:set(v)
						end),
					}),
					ds.Settings.Rule(),
					row("style", {
						label = t("style.label"),
						description = t("style.hint"),
						control = select(M.codex_voice, t("style.label"), function(): { any }
							local out = { { value = "", label = t("style.default"):get() } }
							local saved = M.codex_voice:get()
							local found = saved == ""
							for _, v in M.VOICES do
								table.insert(out, { value = v, label = string.upper(string.sub(v, 1, 1)) .. string.sub(v, 2) })
								found = found or v == saved
							end
							if not found then
								table.insert(out, { value = saved, label = string.upper(string.sub(saved, 1, 1)) .. string.sub(saved, 2) })
							end
							return out
						end, function(v: string)
							M.codex_voice:set(v)
						end),
					}),
				},
			},
		},
		ds.Settings.Section {
			title = t("dictation.section"),
			ds.Settings.Group {
				ui.column {
					w_full = true,
					dictation_row,
					progress,
					mic_row,
					ds.Settings.Rule(),
					shortcut_row,
				},
			},
		},
		ui.show(function(): boolean
			local m = model()
			return m.cache == true and not m.downloading
		end, ds.Settings.Section {
			title = t("model.section"),
			ds.Settings.Group {
				row("model", {
					label = t("model.label"),
					description = function(): string
						return `{t("model.name"):get()} · {t("model.size", { total = mb(model().size) }):get()}`
					end,
					control = ds.Button {
						id = "voice-remove",
						tone = "secondary",
						size = "sm",
						label = t("model.remove_label"),
						on_click = remove,
						t("model.remove"),
					},
				}),
			},
		}),
		ui.show(function(): boolean
			return error_word:get() ~= nil or (model().error ~= nil)
		end, ui.text {
			function(): string
				return error_word:get() or (if model().error then words(model().error) else "")
			end,
			id = "voice-error",
			role = "alert",
			wrap = true,
			text_size = "$font.size.sm",
			color = "$zeron.tone.danger.ink",
		}),
	}
end

return M
