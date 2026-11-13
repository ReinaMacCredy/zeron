-- Settings → Voice: dictation, as the shipping app's page has it (crates/ui
-- dictation/model.rs): turning it on downloads the speech model the first time,
-- then the microphone, the shortcut, and the model to remove. Above it, the
-- Voice built-in's switch (screens/plugins/builtins: off, no microphone and
-- no shortcut) and Codex's voice conversation (gpui's `codexVoice`,
-- `codexVoiceDevice`: this device, or another of yours that can stream
-- audio to Codex there, and the style; kept for a call, none in zgx yet).

local host = require("@app/screens/settings/host")
local ds = require("@ds")
local toast = require("@ds/toast")
local Shortcut = require("@ds/shortcut_recorder")
local prefs = require("@app/screens/settings/prefs")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local keys = require("@app/screens/shell/keys")
local dictation = require("@app/screens/chat/dictation")
local Builtins = require("@app/screens/plugins/builtins")

local M = {}

M.ID = "voice"
M.TITLE = "Voice"
local SHORTCUT = "shell.dictate"

search.index(M.ID, ui.t("settings.voice.title"), ui.t("settings.voice.terms"), {
	{ key = "feature_voice", title = ui.t("settings.features.voice.label"), terms = ui.t("settings.features.voice.terms") },
	{ key = "device", title = ui.t("settings.voice.device.label"), section = ui.t("settings.voice.conversation"), terms = ui.t("settings.voice.device.terms") },
	{ key = "style", title = ui.t("settings.voice.style.label"), section = ui.t("settings.voice.conversation"), terms = ui.t("settings.voice.style.terms") },
	{ key = "dictation", title = ui.t("settings.voice.dictation.label") },
	{ key = "microphone", title = ui.t("settings.voice.microphone") },
	{ key = "shortcut", title = ui.t("settings.voice.shortcut.label") },
	{ key = "model", title = ui.t("settings.voice.model.label") },
})

-- Codex's voice: "" its default and this device.
M.codex_voice = ui.persist("codex_voice", "")
M.codex_voice_device = ui.persist("codex_voice_device", "")
-- (gpui's `zeron_proto::voice::DEFAULT_VOICES`, until a live session names its own.)
M.VOICES = { "juniper", "maple", "spruce", "ember", "vale", "breeze", "arbor", "sol", "cove" }
-- Your other devices that can stream voice to Codex (src/voice.rs `voice.remote`).
local remote = ui.state("settings_voice_remote", {} :: { [string]: boolean })

local installed = ui.state("settings_voice_installed", false)
local size = ui.state("settings_voice_size", 0)
local inputs = ui.state("settings_voice_inputs", {} :: { any })
local on = ui.state("settings_voice_on", false)

local function refresh()
	host.call("dictation.call", { op = "status" }):after(function(s: any)
		ui.batch(function()
			installed:set(s.installed)
			size:set(s.size)
		end)
	end, function() end)
	host.call("dictation.call", { op = "inputs" }):after(function(list: any)
		inputs:set(list)
	end, function() end)
end
local function refused(err: any)
	toast.show {
		title = ui.t("settings.voice.couldnt"):get(),
		body = if type(err) == "table" and err.message then tostring(err.message) else nil,
		tone = "warning",
	}
end
local function downloading(): boolean
	return dictation.state:get().phase == "downloading"
end
local function mb(bytes: number): string
	return string.format("%.0f", bytes / 1e6)
end

-- The switch says what's so: on while downloading, or on with the model here.
ui.effect(function()
	local now = downloading() or (prefs.dictation:get() and installed:get())
	if on:peek() ~= now then
		on:set(now)
	end
end)
-- A download ending: on when it arrived, and why not when it didn't.
local fetching = ui.state("settings_voice_fetching", false)
ui.effect(function()
	local s = dictation.state:get()
	if s.phase == "downloading" then
		fetching:set(true)
	elseif fetching:peek() then
		fetching:set(false)
		if s.phase == "ready" then
			installed:set(true)
			prefs.dictation:set(true)
		else
			on:set(false)
			if s.phase == "download_failed" then
				refused({ message = s.error })
			end
		end
	end
end)
local function flip(want: boolean)
	if downloading() then
		host.call("dictation.call", { op = "cancel" }):after(function() end, refused)
	elseif installed:peek() or not want then
		prefs.dictation:set(want)
	else
		host.call("dictation.call", { op = "download" }):after(function() end, function(err: any)
			on:set(false)
			refused(err)
		end)
	end
end
local function remove()
	host.call("dictation.call", { op = "remove" }):after(function()
		ui.batch(function()
			installed:set(false)
			prefs.dictation:set(false)
		end)
	end, refused)
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end
local function microphones(): { any }
	local saved = prefs.dictation_input:get()
	local options = { { value = "system", label = ui.t("settings.voice.system_default"):get() } }
	local listed = saved == "system"
	for _, d in inputs:get() do
		table.insert(options, { value = d.id, label = d.name })
		listed = listed or d.id == saved
	end
	-- (Unplugged, it stays chosen and the system's records: back when it's back.)
	if not listed then
		table.insert(options, { value = saved, label = ui.t("settings.voice.disconnected"):get() })
	end
	return options
end

-- Codex voice's devices (gpui's `set_hosts`): this one, then each other of yours, usable when online with remote voice.
local function voice_hosts(): { any }
	local function t(key: string): string
		return ui.t(`settings.voice.device.{key}`):get()
	end
	local out = { { value = "", label = t("this"), description = t("local") } }
	local ok, sessions = pcall(require, "@account/sessions")
	local now = if ok then (sessions :: any).now:get() else nil
	local saved = M.codex_voice_device:get()
	local found = saved == ""
	for _, d in (now and now.devices) or {} do
		if not d.here and not d.cloud then
			local compatible = remote:get()[d.id] == true
			local detail = if d.online == false then t("offline") elseif not compatible then t("update") else t("available")
			table.insert(out, { value = d.id, label = d.name, description = detail, disabled = d.online == false or not compatible })
			found = found or d.id == saved
		end
	end
	if not found then
		table.insert(out, { value = saved, label = t("unavailable"), description = t("choose"), disabled = true })
	end
	return out
end
local function voice_styles(): { any }
	local function named(v: string): string
		return string.upper(string.sub(v, 1, 1)) .. string.sub(v, 2)
	end
	local out = { { value = "", label = ui.t("settings.voice.style.default"):get() } }
	local saved = M.codex_voice:get()
	local found = saved == ""
	for _, v in M.VOICES do
		table.insert(out, { value = v, label = named(v) })
		found = found or v == saved
	end
	if not found then
		table.insert(out, { value = saved, label = named(saved) })
	end
	return out
end
-- Which of your other devices can stream voice (asked as the page opens and as they change).
local function ask_remote()
	local ok, sessions = pcall(require, "@account/sessions")
	local now = if ok then (sessions :: any).now:get() else nil
	local ids = {}
	for _, d in (now and now.devices) or {} do
		if not d.here then
			table.insert(ids, d.id)
		end
	end
	if #ids == 0 then
		return
	end
	local asked, p = pcall(host.call, "voice.remote", { ids = ids })
	if asked and p then
		p:after(function(v: any)
			if type(v) == "table" then
				remote:set(v)
			end
		end, function() end)
	end
end

function M.view(): Node
	visits.once(M.ID, refresh)
	ui.effect(ask_remote)
	return ds.Settings.Page {
		title = ui.t("settings.voice.title"),
		description = ui.t("settings.voice.description"),
		search.anchor("feature_voice", Builtins.Row("voice")),
		ds.Settings.Section {
			title = ui.t("settings.voice.conversation"),
			ds.Settings.Group {
				row("device", {
					label = ui.t("settings.voice.device.label"),
					description = ui.t("settings.voice.device.hint"),
					control = ds.Select { value = M.codex_voice_device, options = voice_hosts, name = ui.t("settings.voice.device.label"), w = 200 },
				}),
				row("style", {
					label = ui.t("settings.voice.style.label"),
					description = ui.t("settings.voice.style.hint"),
					control = ds.Select { value = M.codex_voice, options = voice_styles, name = ui.t("settings.voice.style.label"), w = 200 },
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.voice.dictation.section"),
			ds.Settings.Group {
				row("dictation", {
					label = ui.t("settings.voice.dictation.label"),
					description = function(): string
						local s = dictation.state:get()
						if s.phase == "downloading" then
							return ui.t("settings.voice.dictation.downloading", { done = mb(s.bytes or 0), total = mb(s.total or 0) }):get()
						elseif not installed:get() then
							return ui.t("settings.voice.dictation.needs_download", { total = mb(size:get()) }):get()
						end
						return ui.t("settings.voice.dictation.description"):get()
					end,
					control = ds.Switch { value = on, label = ui.t("settings.voice.dictation.label"), on_change = flip },
				}),
				-- (Stays put while it's off, dimmed and out of reach.)
				search.anchor(
					"microphone",
					ui.div {
						w_full = true,
						opacity = function()
							return if on:get() then 1 else "$zeron.disabled"
						end,
						disabled = function()
							return not on:get()
						end,
						ds.Settings.Row {
							label = ui.t("settings.voice.microphone"),
							control = ds.Select { value = prefs.dictation_input, options = microphones, name = ui.t("settings.voice.microphone"), w = 200 },
						},
					}
				),
				row("shortcut", {
					label = ui.t("settings.voice.shortcut.label"),
					description = ui.t("settings.voice.shortcut.description"),
					control = Shortcut.Recorder {
						value = function(): string
							return keys.combo(SHORTCUT)
						end,
						label = ui.t("settings.voice.shortcut.label") :: any,
						on_change = function(combo: string)
							local owner = keys.owner(combo, SHORTCUT)
							local taken = if owner then keys.get(owner) else nil
							if owner then
								refused({ message = ui.t("settings.shortcuts.taken", { keys = Shortcut.shown(combo), name = if taken then keys.title(taken) else owner }):get() })
								return
							end
							keys.bind(SHORTCUT, combo)
						end,
					},
				}),
			},
		},
		ui.show(function(): boolean
			return installed:get() and not downloading()
		end, ds.Settings.Section {
			ds.Settings.Group {
				row("model", {
					label = ui.t("settings.voice.model.label"),
					description = function(): string
						return ui.t("settings.voice.model.description", { total = mb(size:get()) }):get()
					end,
					control = ds.Button { ui.t("settings.voice.model.remove"), tone = "secondary", size = "sm", on_click = remove },
				}),
			},
		}),
	}
end

return M
