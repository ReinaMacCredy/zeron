-- Settings → Chat: gpui's Chat page (crates/ui settings/shortcuts.rs `Shown::Chat`), its rows on prefs.luau and General's thread naming.

local ds = require("@ds")
local prefs = require("@app/screens/settings/prefs")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local general = require("@app/screens/settings/general")
local chat = require("@app/screens/chat/store")

local M = {}

M.ID = "chat"
M.TITLE = "Chat"

search.index(M.ID, ui.t("settings.chat.title"), ui.t("settings.chat.terms"), {
	{ key = "send_with", title = ui.t("settings.general.send_with.label"), section = ui.t("settings.chat.composer"), terms = ui.t("settings.general.send_with.terms") },
	{ key = "follow_up", title = ui.t("settings.general.follow_up.label"), section = ui.t("settings.chat.composer"), terms = ui.t("settings.general.follow_up.terms") },
	{ key = "escape_stops", title = ui.t("settings.general.escape_stops.label"), section = ui.t("settings.chat.composer"), terms = ui.t("settings.general.escape_stops.terms") },
	{ key = "compact_mode", title = ui.t("settings.general.compact_mode.label"), section = ui.t("settings.chat.transcript"), terms = ui.t("settings.general.compact_mode.terms") },
	{ key = "thread_naming", title = ui.t("settings.general.thread_naming.label"), section = ui.t("settings.general.thread_naming.label"), terms = ui.t("settings.general.thread_naming.terms") },
})

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

function M.view(): Node
	visits.once("chat", function()
		general.load_titles()
		if chat.catalog:peek().state ~= "ready" then
			chat.load_catalog()
		end
	end)
	return ds.Settings.Page {
		title = ui.t("settings.chat.title"),
		description = ui.t("settings.chat.description"),
		ds.Settings.Section {
			title = ui.t("settings.chat.composer"),
			ds.Settings.Group {
				row("send_with", {
					label = ui.t("settings.general.send_with.label"),
					description = function()
						return if prefs.send_with:get() == "enter"
							then ui.t("settings.general.send_with.enter"):get()
							else ui.t("settings.general.send_with.mod_enter"):get()
					end,
					control = ds.Select { value = prefs.send_with, name = ui.t("settings.general.send_with.label"), w = 128, options = prefs.SEND_WITH },
				}),
				row("follow_up", {
					label = ui.t("settings.general.follow_up.label"),
					description = function()
						return if prefs.send_with:get() == "enter"
							then ui.t("settings.general.follow_up.description", { key = ui.shortcut_text("cmd+enter") }):get()
							else ui.t("settings.general.follow_up.description_send_key", { key = ui.shortcut_text("cmd+enter") }):get()
					end,
					control = ds.Select { value = prefs.follow_up, name = ui.t("settings.general.follow_up.label"), w = 128, options = prefs.FOLLOW_UP },
				}),
				row("escape_stops", {
					label = ui.t("settings.general.escape_stops.label"),
					description = ui.t("settings.general.escape_stops.description"),
					control = ds.Switch { value = prefs.escape_stops, label = ui.t("settings.general.escape_stops.label") },
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.chat.transcript"),
			ds.Settings.Group {
				row("compact_mode", {
					label = ui.t("settings.general.compact_mode.label"),
					description = ui.t("settings.general.compact_mode.description"),
					control = ds.Switch { value = prefs.compact_transcript, label = ui.t("settings.general.compact_mode.label") },
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.general.thread_naming.label"),
			ds.Settings.Group {
				general.thread_naming_row(),
			},
		},
	}
end

return M
