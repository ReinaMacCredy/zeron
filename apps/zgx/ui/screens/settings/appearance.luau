-- Settings → Appearance: how Zeron looks on this Mac, the shell's own
-- look (screens/shell/appearance: the scheme, which setup chose too; a new
-- chat's wallpaper and its effect, the frame and the art behind a chat), the
-- theme each scheme wears (a quick pick; every theme in the Themes section,
-- screens/themes) and the window's glass (ds.material). Past Advanced look: all
-- but the colours and the fonts, as the shipping app's page has it.

local ds = require("@ds")
local material = require("@ds/material")
local math_ = require("@ds/math")
local appearance = require("@app/screens/shell/appearance")
local search = require("@app/screens/settings/search")
local Builtins = require("@app/screens/plugins/builtins")
local prefs = require("@app/screens/settings/prefs")
local host = require("@app/screens/settings/host")
local wallpaper = require("@app/screens/settings/wallpaper")
local plugin_settings = require("@app/screens/plugins/settings")
local motion = require("@ds/motion")
local prefers = require("@ds/prefers")
local themes = require("@app/screens/themes/state")
local themes_page = require("@app/screens/themes/page")
local frame = require("@app/screens/shell/frame")
local keys = require("@app/screens/shell/keys")

local M = {}

M.ID = "appearance"
M.TITLE = "Appearance"

search.index(M.ID, ui.t("settings.appearance.title"), ui.t("settings.appearance.terms"), {
	{ key = "scheme", title = ui.t("settings.appearance.scheme.label"), terms = ui.t("settings.appearance.scheme.terms") },
	{ key = "theme_light", title = ui.t("settings.appearance.theme.light"), terms = ui.t("settings.appearance.theme.terms") },
	{ key = "theme_dark", title = ui.t("settings.appearance.theme.dark"), terms = ui.t("settings.appearance.theme.terms") },
	{ key = "themes", title = ui.t("settings.appearance.theme.more"), terms = ui.t("settings.appearance.theme.terms") },
	{ key = "accent", title = ui.t("settings.appearance.accent.label"), terms = ui.t("settings.appearance.accent.terms") },
	{ key = "themes_section", title = ui.t("themes.page.title"), terms = ui.t("settings.appearance.theme.terms") },
	{ key = "reduce_motion", title = ui.t("settings.appearance.reduce_motion.label"), section = ui.t("settings.appearance.motion"), terms = ui.t("settings.appearance.reduce_motion.terms") },
	{ key = "pause_motion", title = ui.t("settings.appearance.pause_motion.label"), section = ui.t("settings.appearance.motion"), terms = ui.t("settings.appearance.pause_motion.terms") },
	{ key = "smooth_scrolling", title = ui.t("settings.appearance.smooth_scrolling.label"), section = ui.t("settings.appearance.motion"), terms = ui.t("settings.appearance.smooth_scrolling.terms") },
	{ key = "glass", title = ui.t("settings.appearance.glass.label"), terms = ui.t("settings.appearance.glass.terms") },
	{ key = "interface_font", title = ui.t("settings.appearance.interface_font.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.interface_font.terms") },
	{ key = "text_size", title = ui.t("settings.appearance.text_size.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.text_size.terms") },
	{ key = "code_font", title = ui.t("settings.appearance.code_font.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.code_font.terms") },
	{ key = "code_font_size", title = ui.t("settings.appearance.code_font_size.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.code_font_size.terms") },
	{ key = "conversation_width", title = ui.t("settings.appearance.conversation_width.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.conversation_width.terms") },
	{ key = "render_math", title = ui.t("settings.appearance.render_math.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.render_math.terms") },
	{ key = "math_font", title = ui.t("settings.appearance.math_font.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.math_font.terms") },
	{ key = "math_size", title = ui.t("settings.appearance.math_size.label"), section = ui.t("settings.appearance.fonts"), terms = ui.t("settings.appearance.math_size.terms") },
	{ key = "home_widgets", title = ui.t("settings.features.home_widgets.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.features.home_widgets.terms") },
	{ key = "wallpaper", title = ui.t("settings.appearance.wallpaper.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.wallpaper.terms") },
	{ key = "wallpaper_image", title = ui.t("settings.appearance.wallpaper.image.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.wallpaper.image.terms") },
	{ key = "effect", title = ui.t("settings.appearance.effect.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.effect.terms") },
	{ key = "match_colors", title = ui.t("settings.appearance.match_colors.label"), terms = ui.t("settings.appearance.match_colors.terms") },
	{ key = "frost_tint", title = ui.t("settings.appearance.frost.tint.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frost.tint.terms") },
	{ key = "frost_dim", title = ui.t("settings.appearance.frost.dim.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frost.dim.terms") },
	{ key = "frost_brightness", title = ui.t("settings.appearance.frost.brightness.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frost.brightness.terms") },
	{ key = "inset", title = ui.t("settings.appearance.inset.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.inset.terms") },
	{ key = "frame", title = ui.t("settings.appearance.frame.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frame.terms") },
	{ key = "frame_sidebar", title = ui.t("settings.appearance.frame.sidebar.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frame.sidebar.terms") },
	{ key = "frame_blend", title = ui.t("settings.appearance.frame.blend.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frame.blend.terms") },
	{ key = "frame_shade", title = ui.t("settings.appearance.frame.shade.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frame.shade.terms") },
	{ key = "frame_blur", title = ui.t("settings.appearance.frame.blur.label"), section = ui.t("settings.appearance.material"), terms = ui.t("settings.appearance.frame.blur.terms") },
	{ key = "wallpaper_folder", title = ui.t("settings.appearance.folder.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.folder.terms") },
	{ key = "chat_background", title = ui.t("settings.appearance.chat.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.chat.terms") },
	{ key = "chat_placement", title = ui.t("settings.appearance.chat.placement.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.chat.placement.terms") },
	{ key = "chat_opacity", title = ui.t("settings.appearance.chat.opacity.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.chat.opacity.terms") },
	{ key = "chat_blur", title = ui.t("settings.appearance.chat.blur.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.chat.blur.terms") },
	{ key = "chat_scrim", title = ui.t("settings.appearance.chat.scrim.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.chat.scrim.terms") },
	{ key = "chat_tab_shade", title = ui.t("settings.appearance.chat.tab_shade.label"), section = ui.t("settings.appearance.new_chat"), terms = ui.t("settings.appearance.chat.tab_shade.terms") },
})

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

-- A row that comes and goes in a card: its own hairline over it (a card's
-- rules are between rows that are there, never under the last).
local function later(on: () -> boolean, node: Node): Node
	return ui.show(on, ui.column { w_full = true, ds.Settings.Rule(), node })
end

-- The fonts installed on this computer (src/fonts.rs: `{ { family, mono }
-- }`, by name), asked for once a font picker opens; nil until they come.
local installed = ui.state("settings_installed_fonts", nil :: { { family: string, mono: boolean } }?)
local asked = false
local function ask_fonts()
	if asked then
		return
	end
	asked = true
	host.call("fonts.list"):after(function(list: any)
		installed:set(if type(list) == "table" then list else {})
	end, function()
		-- (Asked again as a picker next opens.)
		asked = false
		installed:set({})
	end)
end

-- A font picker (the interface's, or code's, which lists monospaced fonts
-- alone: a terminal's grid and code's columns need every letter one width):
-- the theme's own first ("Default (Geist)", its value ""), then this
-- computer's, searched by name. One chosen but no longer installed stays
-- listed, so the picker still says what's chosen (the theme's own draws).
local function font_picker(kind: "interface" | "code", value: State<string>, label: Message): Node
	local token = if kind == "code" then "font.mono" else "font.sans"
	local fallback = if kind == "code" then "Geist Mono" else "Geist"
	return ds.Dropdown {
		value = value :: any,
		label = label,
		width = 280,
		search = ui.t("settings.appearance.font_search"),
		on_open = ask_fonts,
		items = function(): { any }
			local chosen = value:get()
			local own = ui.theme.get(token)
			local out: { any } = {
				{ value = "", label = ui.t("settings.appearance.font_default", { name = if type(own) == "string" and own ~= "" then own else fallback }):get() },
			}
			local listed = chosen == ""
			for _, font in installed:get() or {} do
				if kind == "interface" or font.mono then
					table.insert(out, { value = font.family, label = font.family })
					listed = listed or font.family == chosen
				end
			end
			if not listed then
				table.insert(out, 2, { value = chosen, label = chosen })
			end
			return out
		end,
		empty = function(_query: string): any
			return { title = ui.t("settings.appearance.font_none") }
		end,
	}
end

local function font_gone(kind: "interface" | "code"): { name: string, fallback: string }?
	local chosen = (if kind == "code" then ds.look.code_font else ds.look.interface_font):get()
	local list = installed:get()
	if chosen == "" or list == nil then
		return nil
	end
	for _, font in list do
		if font.family == chosen then
			return nil
		end
	end
	local own = ui.theme.get(if kind == "code" then "font.mono" else "font.sans")
	return { name = chosen, fallback = if type(own) == "string" and own ~= "" then own else (if kind == "code" then "Geist Mono" else "Geist") }
end

local function font_warning(kind: "interface" | "code", label: string): Node
	return ui.show(function(): boolean
		return font_gone(kind) ~= nil
	end, ui.row {
		id = `font-missing-{kind}`,
		w_full = true,
		pt = "$space.2",
		ds.Text {
			function(): string
				local gone = font_gone(kind)
				return ui.t("settings.appearance.font_missing", { kind = ui.t(label):get(), name = if gone then gone.name else "", fallback = if gone then gone.fallback else "" }):get()
			end,
			text_size = "$font.size.sm",
			color = "$warning",
		},
	})
end

local function choice(value: State<string>, label: Message, values: { string }, base: string): Node
	local options = {}
	for _, v in values do
		table.insert(options, { value = v, label = ui.t(`{base}.{v}.label`) })
	end
	return ds.Segmented { value = value, label = label, options = options :: any }
end

local function percent(v: number): string
	return `{math.floor(v * 100 + 0.5)}%`
end

local function slider_row(key: string, label: string, value: State<number>, opts: { min: number, max: number, step: number, say: (number) -> string, description: string? }): Node
	return row(key, {
		label = ui.t(label),
		description = if opts.description then ui.t(opts.description) else nil,
		control = ds.Slider {
			value = value,
			min = opts.min,
			max = opts.max,
			step = opts.step,
			w = 180,
			label = ui.t(label):get(),
			format = opts.say,
		},
	})
end

local function px_said(v: number): string
	return ui.t("settings.appearance.frame.px", { n = math.floor(v + 0.5) }):get()
end

local function frame_on(): boolean
	return appearance.inset_card:get() and appearance.frame_style:get() ~= "off"
end

local function material_group(): Node
	local function frosted_dark(): boolean
		local kind = ui.theme.kind()
		return material.see_through() and not (kind == "light" or kind == "high-contrast-light")
	end
	return ds.Settings.Group { ui.column {
		w_full = true,
		row("glass", {
			label = ui.t("settings.appearance.glass.label"),
			description = function(): string
				local v = material.surface:get()
				if v == "theme" then
					return ui.t("settings.appearance.glass.helper_theme", { kind = ui.t("settings.appearance.glass.kind_frosted"):get() }):get()
				end
				return ui.t(`settings.appearance.glass.helper_{v}`):get()
			end,
			control = ds.Select {
				value = material.surface,
				name = ui.t("settings.appearance.glass.label"),
				w = 148,
				options = function(): { any }
					local out = {}
					for _, v in { "theme", "frosted", "opaque" } do
						table.insert(out, { value = v, label = ui.t(`settings.appearance.glass.{v}`):get() })
					end
					return out
				end,
			},
		}),
		later(frosted_dark, slider_row("frost_tint", "settings.appearance.frost.tint.label", material.frost_tint, { min = material.FROST_TINT_MIN, max = 1, step = 0.01, say = percent })),
		later(frosted_dark, row("frost_dim", {
			label = ui.t("settings.appearance.frost.dim.label"),
			description = ui.t("settings.appearance.frost.dim.description"),
			control = ds.Switch { value = material.frost_dim, label = ui.t("settings.appearance.frost.dim.label") },
		})),
		later(function(): boolean
			return frosted_dark() and material.frost_dim:get()
		end, slider_row("frost_brightness", "settings.appearance.frost.brightness.label", material.frost_brightness, { min = material.FROST_BRIGHTNESS_MIN, max = 1, step = 0.01, say = percent })),
		ds.Settings.Rule(),
		-- (A template without the peek hasn't the setting.)
		if require("@app/screens/shell/view").sidebar_hover
			then row("sidebar_hover", {
				label = ui.t("settings.appearance.sidebar_hover.label"),
				description = ui.t("settings.appearance.sidebar_hover.description"),
				control = ds.Switch { value = require("@app/screens/shell/view").sidebar_hover :: any, label = ui.t("settings.appearance.sidebar_hover.label") },
			})
			else nil,
		row("inset", {
			label = ui.t("settings.appearance.inset.label"),
			description = ui.t("settings.appearance.inset.description"),
			control = ds.Switch { value = appearance.inset_card, label = ui.t("settings.appearance.inset.label") },
		}),
		later(function(): boolean
			return appearance.inset_card:get()
		end, row("frame", {
			label = ui.t("settings.appearance.frame.label"),
			description = function(): string
				return ui.t(`settings.appearance.frame.{appearance.frame_style:get()}.description`):get()
			end,
			control = choice(appearance.frame_style, ui.t("settings.appearance.frame.label"), { "off", "dark", "glass" }, "settings.appearance.frame"),
		})),
		later(frame_on, row("frame_sidebar", {
			label = ui.t("settings.appearance.frame.sidebar.label"),
			description = function(): string
				return ui.t(`settings.appearance.frame.sidebar.{appearance.frame_sidebar:get()}.description`):get()
			end,
			control = choice(appearance.frame_sidebar, ui.t("settings.appearance.frame.sidebar.label"), { "plain", "darker", "glass" }, "settings.appearance.frame.sidebar"),
		})),
		later(function(): boolean
			return frame_on() and appearance.frame_sidebar:get() ~= "plain"
		end, row("frame_blend", {
			label = ui.t("settings.appearance.frame.blend.label"),
			description = function(): string
				return ui.t(`settings.appearance.frame.blend.{appearance.frame_blend:get()}.description`):get()
			end,
			control = choice(appearance.frame_blend, ui.t("settings.appearance.frame.blend.label"), { "whole", "fade" }, "settings.appearance.frame.blend"),
		})),
		later(frame_on, slider_row("frame_shade", "settings.appearance.frame.shade.label", appearance.frame_shade, { min = 0, max = 0.8, step = 0.01, say = percent })),
		later(function(): boolean
			return frame_on() and appearance.frame_style:get() == "glass"
		end, slider_row("frame_blur", "settings.appearance.frame.blur.label", appearance.frame_blur, { min = 0, max = 40, step = 1, say = px_said })),
	} }
end

local PREVIEW_H = 200
local function chat_preview(): Node
	local box = ui.ref()
	local function line(width: string, alpha: string): Node
		return ui.div { w = width, h = 7, rounded = "$radius.full", bg = "$text.primary", opacity = tonumber(alpha) }
	end
	return ui.div {
		id = "chat-background-preview",
		ref = box,
		role = "img",
		label = ui.t("settings.appearance.chat.preview"),
		w_full = true,
		h = PREVIEW_H,
		relative = true,
		overflow = "hidden",
		rounded = "$radius.lg",
		border = 1,
		border_color = "$border",
		bg = "$zeron.window",
		frame.Layer {
			kind = function(): string?
				return if appearance.image:get() then "chat" else nil
			end,
			size = function(): { w: number, h: number }
				return { w = box.width or 0, h = PREVIEW_H }
			end,
		},
		ui.column {
			absolute = true,
			left = "20%",
			right = "20%",
			top = "$space.6",
			gap = "$space.3",
			aria_hidden = true,
			line("60%", "0.5"),
			line("90%", "0.28"),
			line("75%", "0.28"),
			line("40%", "0.28"),
		},
	}
end

local function chat_rows(): Node
	local function on(): boolean
		return appearance.chat_enabled:get()
	end
	return ui.column {
		w_full = true,
		ds.Settings.Rule(),
		row("chat_background", {
			label = ui.t("settings.appearance.chat.label"),
			description = ui.t("settings.appearance.chat.description"),
			control = ds.Switch { value = appearance.chat_enabled, label = ui.t("settings.appearance.chat.label") },
		}),
		later(on, ui.div { w_full = true, px = "$space.4", py = "$space.3", chat_preview() }),
		later(on, row("chat_placement", {
			label = ui.t("settings.appearance.chat.placement.label"),
			description = function(): string
				return ui.t(`settings.appearance.chat.placement.{appearance.chat_placement:get()}.description`):get()
			end,
			control = choice(appearance.chat_placement, ui.t("settings.appearance.chat.placement.label"), { "hero", "full" }, "settings.appearance.chat.placement"),
		})),
		later(on, slider_row("chat_opacity", "settings.appearance.chat.opacity.label", appearance.chat_opacity, { min = 0, max = 1, step = 0.01, say = percent })),
		later(on, slider_row("chat_blur", "settings.appearance.chat.blur.label", appearance.chat_blur, { min = 0, max = 48, step = 4, say = px_said })),
		later(on, row("chat_scrim", {
			label = ui.t("settings.appearance.chat.scrim.label"),
			description = ui.t("settings.appearance.chat.scrim.description"),
			control = ds.Switch { value = appearance.chat_scrim, label = ui.t("settings.appearance.chat.scrim.label") },
		})),
		later(on, slider_row("chat_tab_shade", "settings.appearance.chat.tab_shade.label", appearance.chat_tab_shade, { min = 0, max = 1, step = 0.01, say = percent, description = "settings.appearance.chat.tab_shade.description" })),
	}
end

-- Your picture, small, at the row's lead (nothing while there's none).
local THUMB = 36
local function thumbnail(): Node
	return ui.show(function(): boolean
		return appearance.image:get() ~= nil and not wallpaper.missing()
	end, ui.div {
		w = THUMB,
		h = THUMB,
		shrink = 0,
		rounded = "$radius.md",
		overflow = "hidden",
		border = 1,
		border_color = "$border",
		bg = "$zeron.secondary.bg",
		ui.presence(function(): string
			local image = appearance.image:get()
			return if image then image.path else ""
		end, function(path: string): Node
			local image = appearance.image:peek()
			if path == "" or not image then
				return ui.div {}
			end
			return ds.wallpaper.Picture { src = image.path, w = image.w, h = image.h, x = image.x, y = image.y, zoom = image.zoom }
		end, { w_full = true, h_full = true, relative = true }),
	})
end

-- Adjust: the picture as a new chat frames it, dragged to place and zoomed
-- (the slider, or the wheel), kept on Done. The frame has the window's
-- shape, as the shipping app's.
local FRAME_W, FRAME_H = 360, 225
local function adjust_dialog(): Node
	local from: { x: number, y: number, fx: number, fy: number }? = nil
	local zoom = ui.state("wallpaper_adjust_zoom", 1)
	-- (The slider's own state, kept with the draft both ways.)
	ui.effect(function()
		local z = wallpaper.draft:get().zoom
		if zoom:peek() ~= z then
			zoom:set(z)
		end
	end)
	local function set_zoom(z: number)
		local d = table.clone(wallpaper.draft:peek())
		d.zoom = math.clamp(z, ds.wallpaper.MIN_ZOOM, ds.wallpaper.MAX_ZOOM)
		wallpaper.draft:set(d)
	end
	local function picture(): any
		local image = appearance.image:get()
		local d = wallpaper.draft:get()
		if not image then
			return nil
		end
		return { src = image.path, w = image.w, h = image.h, x = d.x, y = d.y, zoom = d.zoom }
	end
	local function at(): { left: number, top: number, w: number, h: number }
		local p = picture()
		return if p then ds.wallpaper.cover(p, FRAME_W, FRAME_H) else { left = 0, top = 0, w = 0, h = 0 }
	end
	local function zoom_button(label: string, by: number, glyph: string): Node
		return ds.Button {
			glyph,
			label = ui.t(label),
			tone = "secondary",
			size = "sm",
			on_click = function()
				set_zoom(wallpaper.draft:peek().zoom + by)
			end,
		}
	end
	return ds.Dialog {
		open = wallpaper.adjusting,
		title = ui.t("settings.appearance.wallpaper.adjust.title"),
		body = ui.t("settings.appearance.wallpaper.adjust.body"),
		size = "md",
		confirm = { label = ui.t("settings.appearance.wallpaper.adjust.done"), on_click = wallpaper.keep_adjustment, keep_open = true },
		ui.column {
			w_full = true,
			items = "center",
			gap = "$space.4",
			ui.div {
				id = "wallpaper-adjust",
				w = FRAME_W,
				h = FRAME_H,
				shrink = 0,
				relative = true,
				rounded = "$radius.lg",
				overflow = "hidden",
				bg = "$zeron.secondary.bg",
				cursor = "grab",
				role = "img",
				label = ui.t("settings.appearance.wallpaper.adjust.preview"),
				on_press = function(e: PointerEvent)
					local d = wallpaper.draft:peek()
					from = { x = e.x, y = e.y, fx = d.x, fy = d.y }
				end,
				on_pointer_move = function(e: PointerEvent)
					local f = from
					if not f then
						return
					end
					-- (The overflow moves with the pointer: dragging right shows
					-- more of the left.)
					local a = at()
					local over_x, over_y = math.max(1, a.w - FRAME_W), math.max(1, a.h - FRAME_H)
					local d = table.clone(wallpaper.draft:peek())
					d.x = if a.w > FRAME_W then math.clamp(f.fx - (e.x - f.x) / over_x, 0, 1) else 0.5
					d.y = if a.h > FRAME_H then math.clamp(f.fy - (e.y - f.y) / over_y, 0, 1) else 0.5
					wallpaper.draft:set(d)
				end,
				on_release = function()
					from = nil
				end,
				on_wheel = function(e: any): boolean
					set_zoom(wallpaper.draft:peek().zoom - (e.dy or 0) * 0.005)
					return true
				end,
				ui.image {
					src = function(): string
						local p = picture()
						return if p then p.src else ""
					end,
					on_load = function()
						local image = appearance.image:peek()
						if image then
							wallpaper.preview_ready:set(image.path)
						end
					end,
					absolute = true,
					left = function(): number
						return at().left
					end,
					top = function(): number
						return at().top
					end,
					w = function(): number
						return at().w
					end,
					h = function(): number
						return at().h
					end,
					aria_hidden = true,
				},
				ui.show(function(): boolean
					return wallpaper.adjust_blocked() ~= ""
				end, ui.row {
					id = "wallpaper-adjust-state",
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					bottom = 0,
					items = "center",
					justify = "center",
					bg = "$zeron.secondary.bg",
					ds.Text {
						function(): string
							local key = wallpaper.adjust_blocked()
							return if key ~= "" then ui.t(key):get() else ""
						end,
						text_size = "$font.size.sm",
						color = "$text.secondary",
					},
				}),
			},
			ds.Slider {
				value = zoom,
				min = ds.wallpaper.MIN_ZOOM,
				max = ds.wallpaper.MAX_ZOOM,
				step = 0.01,
				label = ui.t("settings.appearance.wallpaper.adjust.zoom"),
				w = FRAME_W,
				on_change = function(v: number)
					set_zoom(v)
				end,
				format = function(v: number): string
					return `{math.floor(v * 100 + 0.5)}%`
				end,
			},
			ui.row {
				w = FRAME_W,
				items = "center",
				justify = "between",
				ds.Button {
					id = "wallpaper-adjust-reset",
					ui.t("settings.appearance.wallpaper.adjust.reset"),
					tone = "secondary",
					size = "sm",
					on_click = function()
						wallpaper.draft:set({ x = 0.5, y = 0.5, zoom = 1 })
					end,
				},
				ui.row {
					gap = "$space.2",
					zoom_button("settings.appearance.wallpaper.adjust.zoom_out", -0.1, "−"),
					zoom_button("settings.appearance.wallpaper.adjust.zoom_in", 0.1, "+"),
				},
			},
		},
	}
end

-- A scheme's theme, picked from every theme that has a variant for it,
-- grouped as the Themes page's tabs are. Its label says which scheme while
-- following the system ("Light theme"), else just "Theme".
local GROUPS = { builtin = "themes.page.builtin", template = "themes.page.from_templates", imported = "themes.page.yours" }
local function theme_row(kind: string): Node
	local chosen = ui.state(`settings_theme_{kind}`, themes.worn(kind))
	ui.effect(function()
		local now = themes.worn(kind)
		if chosen:peek() ~= now then
			chosen:set(now)
		end
	end)
	local function label(): string
		local key = if appearance.scheme:get() == "system" then kind else "one"
		return ui.t(`settings.appearance.theme.{key}`):get()
	end
	return row(`theme_{kind}`, {
		label = label,
		control = ds.Select {
			value = chosen,
			name = label,
			w = 220,
			on_change = function(id: string)
				themes.use_for(kind, id)
			end,
			options = function(): { any }
				local out = {}
				for _, e in themes.entries:get() do
					for _, id in e.variants do
						local p = themes.palette(id)
						if p and p.kind == kind then
							-- A built-in theme by its name (Gruvbox, not Gruvbox
							-- Dark); one of several variants of yours by both.
							local name = if e.source == "builtin" or #e.variants == 1 or p.name == e.name then e.name else `{e.name} {p.name}`
							table.insert(out, { value = id, label = name, group = ui.t(GROUPS[e.source] or GROUPS.builtin):get() })
						end
					end
				end
				return out
			end,
		},
	})
end

-- The quick picks: one for each scheme showing (both, following the
-- system), and the way to every theme.
local function theme_rows(): Node
	local scheme = appearance.scheme
	return ui.column {
		w_full = true,
		ui.show(function(): boolean
			return scheme:get() ~= "dark"
		end, theme_row("light")),
		ui.show(function(): boolean
			return scheme:get() == "system"
		end, ds.Settings.Rule()),
		ui.show(function(): boolean
			return scheme:get() ~= "light"
		end, theme_row("dark")),
		ds.Settings.Rule(),
		row("themes", {
			label = ui.t("settings.appearance.theme.more"),
			description = ui.t("settings.appearance.theme.more_description"),
			on_click = function()
				search.open({ page = M.ID, key = "themes_section", title = "", where = "", score = 0 })
			end,
		}),
	}
end

-- What the Wallpaper select shows: Appearance's choice, or nothing while a
-- plugin's wallpaper is used (picking one then takes it back).
local pick = ui.state("settings_wallpaper_pick", nil :: string?)
ui.effect(function()
	local now = if appearance.provider:get() then nil else appearance.wallpaper:get()
	if pick:peek() ~= now then
		pick:set(now)
	end
end)

local ADVANCED = {
	glass = true, frost_tint = true, frost_dim = true, frost_brightness = true, inset = true, frame = true, frame_sidebar = true, frame_blend = true, frame_shade = true, frame_blur = true,
	reduce_motion = true, pause_motion = true, smooth_scrolling = true, render_math = true, math_font = true, math_size = true,
	wallpaper = true, wallpaper_image = true, wallpaper_folder = true, effect = true,
	chat_background = true, chat_placement = true, chat_opacity = true, chat_blur = true, chat_scrim = true, chat_tab_shade = true,
}
local advanced = ui.state("settings_appearance_advanced", false)
ui.effect(function()
	if ADVANCED[search.lit:get() :: any] and not advanced:peek() then
		advanced:set(true)
	end
end)

local motion_pick = ui.state("settings_reduce_motion", motion.choice())
ui.effect(function()
	local now = motion.choice()
	if motion_pick:peek() ~= now then
		motion_pick:set(now)
	end
end)

function M.view(): Node
	if ds.look.interface_font:peek() ~= "" or ds.look.code_font:peek() ~= "" then
		ask_fonts()
	end
	ui.effect(function()
		if ds.look.wallpaper_colors:get() then
			wallpaper.ask_color()
		end
	end)
	return ds.Settings.Page {
		title = ui.t("settings.appearance.title"),
		description = ui.t("settings.appearance.description"),
		ds.Settings.Section {
			ds.Settings.Group {
				row("scheme", {
					label = ui.t("settings.appearance.scheme.label"),
					description = ui.t("settings.appearance.scheme.description"),
					-- gpui's option cards, each a small window in the theme its scheme wears now.
					stacked = true,
					control = ds.AppearancePicker.Schemes { value = appearance.scheme, accent = ds.look.accent },
				}),
				theme_rows(),
				-- The accent: the theme's own, or one of Zeron's presets, the
				-- whole window wearing it (ds.look).
				row("accent", {
					label = ui.t("settings.appearance.accent.label"),
					description = function(): string
						if ds.look.wallpaper_colors:get() then
							return ui.t("settings.appearance.accent_wallpaper_on"):get()
						end
						return ds.look.find(ds.look.accent:get()).label
					end,
					control = ds.AppearancePicker.Accents { value = ds.look.accent },
				}),
				row("match_colors", {
					label = ui.t("settings.appearance.match_colors.label"),
					description = ui.t("settings.appearance.match_colors.description"),
					control = ds.Switch { value = ds.look.wallpaper_colors, label = ui.t("settings.appearance.match_colors.label") },
				}),
				-- The accent in more places (ds.look `accented`).
				row("accented", {
					label = ui.t("settings.appearance.accented.label"),
					description = ui.t("settings.appearance.accented.description"),
					control = ds.Switch { value = ds.look.accented, label = ui.t("settings.appearance.accented.label") },
				}),
			},
		},
		search.anchor("themes_section", themes_page.Section()),
		ds.Settings.Section {
			title = ui.t("settings.appearance.fonts"),
			ds.Settings.Group {
				-- The interface's font and code's (ds.look), as the shipping
				-- app's Interface font and Code font: the window's font tokens,
				-- so everything set in them follows at once.
				row("interface_font", {
					label = ui.t("settings.appearance.interface_font.label"),
					description = ui.t("settings.appearance.interface_font.description"),
					control = font_picker("interface", ds.look.interface_font, ui.t("settings.appearance.interface_font.label")),
				}),
				-- The interface's text: menus, settings, conversations, every
				-- size and line height scaled from the body's (ds.look).
				row("text_size", {
					label = ui.t("settings.appearance.text_size.label"),
					description = ui.t("settings.appearance.text_size.description"),
					control = ds.Select {
						value = ds.look.text_size,
						name = ui.t("settings.appearance.text_size.label"),
						w = 132,
						-- (Read in the locale shown: they follow it.)
						options = function(): { any }
							local out = {}
							for _, n in ds.look.TEXT_SIZES do
								table.insert(out, { value = n, label = (if n == 13 then ui.t("settings.appearance.pt_default", { n = n }) else ui.t("settings.appearance.pt", { n = n })):get() })
							end
							return out
						end,
					},
				}),
				row("code_font", {
					label = ui.t("settings.appearance.code_font.label"),
					description = ui.t("settings.appearance.code_font.description"),
					control = font_picker("code", ds.look.code_font, ui.t("settings.appearance.code_font.label")),
				}),
				-- Code's size: the editor, markdown's code and diffs (not
				-- terminals), as the shipping app's Code & diff font.
				row("code_font_size", {
					label = ui.t("settings.appearance.code_font_size.label"),
					description = ui.t("settings.appearance.code_font_size.description"),
					control = ds.Select {
						value = prefs.code_font_size,
						name = ui.t("settings.appearance.code_font_size.label"),
						w = 112,
						options = function(): { any }
							local out = {}
							for _, n in prefs.CODE_FONT_SIZES do
								table.insert(out, { value = n, label = ui.t("settings.appearance.pt", { n = n }):get() })
							end
							return out
						end,
					},
				}),
				row("conversation_width", {
					label = ui.t("settings.appearance.conversation_width.label"),
					description = ui.t("settings.appearance.conversation_width.description"),
					control = ui.row {
						gap = "$space.2",
						items = "center",
						ds.Button {
							ui.t("settings.appearance.reset"),
							tone = "secondary",
							size = "sm",
							on_click = function()
								prefs.conversation_width:set(prefs.CONVERSATION_WIDTH.default)
							end,
						},
						ds.Slider {
							value = prefs.conversation_width,
							min = prefs.CONVERSATION_WIDTH.min,
							max = prefs.CONVERSATION_WIDTH.max,
							step = prefs.CONVERSATION_WIDTH.step,
							-- (The slider's name joins its value as a string: read as
							-- the page is built.)
							label = ui.t("settings.appearance.conversation_width.label"),
							format = function(v: number): string
								return ui.t("settings.appearance.pt", { n = math.floor(v) }):get()
							end,
						},
					},
				}),
			},
			font_warning("interface", "settings.appearance.interface_font.label"),
			font_warning("code", "settings.appearance.code_font.label"),
		},
		ds.Settings.Section {
			ds.Settings.Group {
				row("advanced_look", {
					label = ui.t("settings.appearance.advanced.label"),
					description = ui.t("settings.appearance.advanced.description"),
					control = ds.IconButton {
						id = "appearance-advanced",
						label = ui.t("settings.appearance.advanced.label"),
						on_click = function()
							advanced:set(not advanced:peek())
						end,
						ui.presence(function(): string
							return if advanced:get() then "open" else "shut"
						end, function(which: string): Node
							return ds.Icon { name = if which == "open" then "alt-arrow-up" else "alt-arrow-down", size = 16 }
						end),
					},
				}),
			},
		},
		ui.show(function(): boolean
			return advanced:get()
		end, ui.column {
			id = "appearance-advanced-rows",
			w_full = true,
			gap = "$space.6",
			ds.Settings.Section {
				title = ui.t("settings.appearance.material"),
				material_group(),
			},
		-- What a new chat opens on: nothing, your picture (the shipping
		-- app's one image, screens/settings/wallpaper), or what a plugin
		-- draws there; and Zeron's effect over it.
		ds.Settings.Section {
			title = ui.t("settings.appearance.new_chat"),
			description = ui.t("settings.appearance.new_chat_description"),
			search.anchor("home_widgets", Builtins.Row("home_widgets")),
			-- (One card, its later rows bringing their own rules.)
			ui.column {
				id = "appearance-new-chat",
				w_full = true,
				ds.Settings.Group {
					ui.column {
						w_full = true,
						row("wallpaper", {
							label = ui.t("settings.appearance.wallpaper.label"),
							description = function(): string
								local p = appearance.provider:get()
								if p then
									return ui.t("settings.appearance.wallpaper.from_plugin", { name = p.name }):get()
								end
								local key = if appearance.wallpaper:get() == "image" then "image_description" else "none_description"
								return ui.t(`settings.appearance.wallpaper.{key}`):get()
							end,
							control = ui.row {
								gap = "$space.2",
								items = "center",
								-- (While a plugin's wallpaper is used: its settings, where
								-- it's chosen and let go.)
								ui.div {
									hidden = function(): boolean
										return appearance.provider:get() == nil
									end,
									ds.Button {
										function(): string
											local p = appearance.provider:get()
											return ui.t("settings.appearance.wallpaper.open_plugin", { name = if p then p.name else "" }):get()
										end,
										tone = "secondary",
										size = "sm",
										on_click = function()
											local p = appearance.provider:peek()
											if p then
												plugin_settings.open(p.id)
											end
										end,
									},
								},
								-- None or your picture: picking either takes the wallpaper
								-- back from a plugin. (Blank while a plugin's is used: the
								-- plugin's name stands in it.)
								ds.Select {
									value = pick,
									name = ui.t("settings.appearance.wallpaper.label"),
									placeholder = function(): string
										local p = appearance.provider:get()
										return if p then p.name else ""
									end,
									w = 180,
									on_change = appearance.choose,
									options = function(): { any }
										return {
											{ value = "none", label = ui.t("settings.appearance.wallpaper.none"):get() },
											{ value = "image", label = ui.t("settings.appearance.wallpaper.image.label"):get() },
										}
									end,
								},
							},
						}),
						-- Your picture: its name, and what can be done with it.
						later(function(): boolean
							return appearance.provider:get() == nil and appearance.wallpaper:get() == "image"
						end, row("wallpaper_image", {
							label = ui.t("settings.appearance.wallpaper.image.label"),
							lead = thumbnail(),
							description = function(): string
								local problem = wallpaper.problem_text()
								if problem ~= "" then
									return problem
								end
								local image = appearance.image:get()
								if not image then
									return ui.t("settings.appearance.wallpaper.image.none"):get()
								elseif wallpaper.missing() then
									return ui.t("settings.appearance.wallpaper.image.missing"):get()
								end
								return image.name
							end,
							control = ui.row {
								gap = "$space.2",
								items = "center",
								ui.div {
									hidden = function(): boolean
										return appearance.image:get() == nil or wallpaper.missing()
									end,
									ds.Button { ui.t("settings.appearance.wallpaper.image.adjust"), tone = "secondary", size = "sm", on_click = wallpaper.adjust },
								},
								ds.Button {
									function(): string
										local key = if appearance.image:get() then "replace" else "choose"
										return ui.t(`settings.appearance.wallpaper.image.{key}`):get()
									end,
									tone = "secondary",
									size = "sm",
									on_click = wallpaper.choose,
								},
								ui.div {
									hidden = function(): boolean
										return appearance.image:get() == nil
									end,
									ds.Button { ui.t("settings.appearance.wallpaper.image.remove"), tone = "secondary", size = "sm", on_click = wallpaper.remove },
								},
							},
						})),
						-- Zeron's effect over your picture (production's list, each
						-- saying what it does). A plugin's has its own, on its page.
						later(function(): boolean
							return appearance.provider:get() == nil and appearance.wallpaper:get() == "image"
						end, row("effect", {
							label = ui.t("settings.appearance.effect.label"),
							description = function(): string
								return ui.t(`settings.appearance.effect.{appearance.effect:get()}.description`):get()
							end,
							control = wallpaper.EffectSelect(appearance.effect),
						})),
						ds.Settings.Rule(),
						row("wallpaper_folder", {
							label = ui.t("settings.appearance.folder.label"),
							description = function(): string
								local folder = appearance.folder:get()
								local name = if folder ~= "" then folder else ui.t("settings.appearance.folder.none"):get()
								return ui.t("settings.appearance.folder.hint", { folder = name, key = ui.shortcut_text(keys.combo("shell.random_wallpaper")) }):get()
							end,
							control = ui.row {
								gap = "$space.2",
								items = "center",
								ds.Button { ui.t("settings.appearance.folder.choose"), tone = "secondary", size = "sm", on_click = wallpaper.choose_folder },
								ui.show(function(): boolean
									return appearance.folder:get() ~= ""
								end, ds.Button { ui.t("settings.appearance.folder.shuffle"), tone = "secondary", size = "sm", on_click = wallpaper.shuffle }),
							},
						}),
						chat_rows(),
					},
				},
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.appearance.motion"),
			ds.Settings.Group {
				row("reduce_motion", {
					label = ui.t("settings.appearance.reduce_motion.label"),
					description = function(): string
						local v = motion.choice()
						if v == "system" then
							return ui.t(if prefers.reduced_motion:get() then "settings.appearance.reduce_motion.helper_system_on" else "settings.appearance.reduce_motion.helper_system_off"):get()
						end
						return ui.t(`settings.appearance.reduce_motion.helper_{v}`):get()
					end,
					control = ds.Select {
						value = motion_pick,
						name = ui.t("settings.appearance.reduce_motion.label"),
						w = 128,
						on_change = motion.choose,
						options = function(): { any }
							local out = {}
							for _, v in { "system", "on", "off" } do
								table.insert(out, { value = v, label = ui.t(`settings.appearance.reduce_motion.{v}`):get() })
							end
							return out
						end,
					},
				}),
				row("pause_motion", {
					label = ui.t("settings.appearance.pause_motion.label"),
					description = ui.t("settings.appearance.pause_motion.description"),
					control = ds.Switch { value = motion.pause_in_background, label = ui.t("settings.appearance.pause_motion.label") },
				}),
				row("smooth_scrolling", {
					label = ui.t("settings.appearance.smooth_scrolling.label"),
					description = function(): string
						local reduced = motion.smooth_scrolling:get() and not motion.wheel_glides()
						return ui.t(if reduced then "settings.appearance.smooth_scrolling.helper_reduced" else "settings.appearance.smooth_scrolling.helper"):get()
					end,
					control = ds.Switch { value = motion.smooth_scrolling, label = ui.t("settings.appearance.smooth_scrolling.label") },
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.appearance.math"),
			ds.Settings.Group {
				row("render_math", {
					label = ui.t("settings.appearance.render_math.label"),
					description = ui.t("settings.appearance.render_math.description"),
					control = ds.Switch { value = math_.render, label = ui.t("settings.appearance.render_math.label") },
				}),
				row("math_font", {
					label = ui.t("settings.appearance.math_font.label"),
					control = ds.Select {
						value = math_.face,
						name = ui.t("settings.appearance.math_font.label"),
						w = 180,
						disabled = function(): boolean
							return not math_.render:get()
						end,
						options = (function(): { any }
							local out = {}
							for _, f in math_.FACES do
								table.insert(out, { value = f.value, label = f.label, description = f.style })
							end
							return out
						end)(),
					},
				}),
				row("math_size", {
					label = ui.t("settings.appearance.math_size.label"),
					description = ui.t("settings.appearance.math_size.description"),
					control = ds.Select {
						value = math_.size,
						name = ui.t("settings.appearance.math_size.label"),
						w = 112,
						disabled = function(): boolean
							return not math_.render:get()
						end,
						options = (function(): { any }
							local out = {}
							for _, n in math_.SIZES do
								table.insert(out, { value = n, label = `{n}%` })
							end
							return out
						end)(),
					},
				}),
			},
		},
		}),
		adjust_dialog(),
	}
end

return M
