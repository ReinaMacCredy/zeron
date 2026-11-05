-- How the main window looks, as chosen: the colour scheme (setup's own
-- choice, the same setting), and a new chat's wallpaper and its effect.
-- Kept across launches; changing the scheme restyles every window.
--
--   appearance.scheme      "system" | "light" | "dark" (a state)
--   appearance.wallpaper   Appearance's own choice: "none" or "image" (your
--                          picture, `appearance.image`), kept while a plugin's
--                          wallpaper stands in for it
--   appearance.image       your picture: { path, name, w, h, x, y, zoom }
--                          (a copy the host keeps: screens/settings/wallpaper),
--                          or nil
--   appearance.effect      over your picture: "none" | "dither" | "ascii" |
--                          "halftone" | "scanlines"
--   appearance.plugin      the plugin whose wallpaper is used ("" none), set
--                          on its own settings page; one at a time
--   appearance.plugin_effect  the effect over a plugin's wallpaper
--   appearance.provider()  that plugin, while it's on and offers one ({ id,
--                          name }), else nil: Appearance's choice shows
--   appearance.showing()   what a new chat shows now: "none", "image", or
--                          "plugin:<id>"
--   appearance.effect_now()  the effect over what shows
--   appearance.use_plugin(id, on)  a plugin's wallpaper used (taking over from
--                          any other), or let go
--   appearance.choose(v)   Appearance's "none" or "image", taking it back from
--                          a plugin
--
-- A plugin offers a wallpaper by replacing a new chat's background place
-- (`#new-chat-background`, in its manifest's `ui.change`; the place provides
-- `pointer` = { x, y }, 0 to 1) and declaring a `wallpaper` setting, the
-- row Zeron draws on its settings page to use it. Zeron's effect is drawn
-- over whatever stands there.

local state = require("@app/screens/onboarding/state")
local actions = require("@app/screens/onboarding/actions")
local plugins = require("@app/screens/plugins/state")
local look = require("@ds/look")

local M = {}

M.scheme = state.appearance
-- Applied as it changes (and once now: the look chosen in an earlier run).
ui.effect(function()
	actions.apply_scheme(state.appearance:get())
end)

-- The place plugins draw a new chat's background (an element's id), and
-- its selector as a manifest's `ui.change` names it.
M.SLOT = "new-chat-background"
M.SELECTOR = `#{M.SLOT}`

export type Image = { path: string, name: string, w: number, h: number, x: number, y: number, zoom: number }

local EFFECTS = { none = true, dither = true, ascii = true, halftone = true, scanlines = true }

M.wallpaper = ui.persist("shell_wallpaper", "none")
M.image = ui.persist("shell_wallpaper_image", nil :: Image?)
M.effect = ui.persist("shell_wallpaper_effect", "dither")
M.plugin = ui.persist("shell_wallpaper_plugin", "")
M.plugin_effect = ui.persist("shell_wallpaper_plugin_effect", "dither")

M.inset_card = look.inset_card
M.frame_style = ui.persist("shell_frame_style", "dark")
M.frame_shade = ui.persist("shell_frame_shade", 0.58)
M.frame_blur = ui.persist("shell_frame_blur", 28)
M.frame_sidebar = ui.persist("shell_frame_sidebar", "glass")
M.frame_blend = ui.persist("shell_frame_blend", "whole")
M.chat_enabled = ui.persist("shell_chat_background", false)
M.chat_placement = ui.persist("shell_chat_background_placement", "hero")
M.chat_opacity = ui.persist("shell_chat_background_opacity", 0.2)
M.chat_blur = ui.persist("shell_chat_background_blur", 24)
M.chat_scrim = ui.persist("shell_chat_background_scrim", true)
M.chat_tab_shade = ui.persist("shell_chat_background_tab_shade", 0.55)
M.folder = ui.persist("shell_wallpaper_folder", "")
M.source = ui.persist("shell_wallpaper_source", "")
M.history = ui.persist("shell_wallpaper_history", {} :: { string })
M.color = ui.persist("shell_wallpaper_color", "")
ui.effect(function()
	local on = look.wallpaper_colors:get() and M.image:get() ~= nil and M.color:get() ~= ""
	local tint = if on then M.color:get() else nil
	if look.wallpaper_tint:peek() ~= tint then
		look.wallpaper_tint:set(tint)
	end
end)
do
	local w = M.wallpaper:peek()
	local id = string.match(w, "^plugin:(.+)$")
	if id then
		-- A plugin chosen here once: it's the plugin's to say now.
		M.plugin:set(id)
		M.wallpaper:set(if M.image:peek() then "image" else "none")
	elseif w ~= "none" and w ~= "image" then
		-- A retired choice (a scene's name, the wallpaper folder).
		M.wallpaper:set("none")
	end
	-- (Blur went: Zeron's effects are these.)
	for _, e in { M.effect, M.plugin_effect } do
		if not EFFECTS[e:peek()] then
			e:set("none")
		end
	end
end

-- The plugins on that can draw a new chat's background, by name.
M.providers = ui.computed(function(): { { id: string, name: string } }
	local out = {}
	for _, p in plugins.list:get() do
		if p.status == "on" and p.ui and p.ui.change then
			for _, at in p.ui.change do
				if at == M.SELECTOR then
					table.insert(out, { id = p.id, name = p.name })
					break
				end
			end
		end
	end
	return out
end)

-- The plugin whose wallpaper is used, while it can be.
M.provider = ui.computed(function(): { id: string, name: string }?
	local id = M.plugin:get()
	if id == "" then
		return nil
	end
	for _, p in M.providers:get() do
		if p.id == id then
			return p
		end
	end
	return nil
end)

-- What a new chat shows now.
M.showing = ui.computed(function(): string
	local p = M.provider:get()
	if p then
		return `plugin:{p.id}`
	end
	if M.wallpaper:get() == "image" and M.image:get() then
		return "image"
	end
	return "none"
end)

function M.chat_shown(): boolean
	return M.chat_enabled:get() and M.chat_opacity:get() > 0 and M.showing:get() == "image"
end

function M.effect_now(): string
	return if M.provider:get() then M.plugin_effect:get() else M.effect:get()
end

function M.use_plugin(id: string, on: boolean)
	if on then
		M.plugin:set(id)
	elseif M.plugin:peek() == id then
		M.plugin:set("")
	end
end

function M.choose(value: string)
	ui.batch(function()
		M.wallpaper:set(value)
		M.plugin:set("")
	end)
end

return M
