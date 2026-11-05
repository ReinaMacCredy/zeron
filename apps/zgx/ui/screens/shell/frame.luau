
local ds = require("@ds")
local appearance = require("@app/screens/shell/appearance")
local art = require("@app/screens/shell/art")
local route = require("@app/screens/shell/route")
local host = require("@app/screens/settings/host")

local M = {}

local TOP_BAR = 42
local HERO_RATIO, HERO_MAX = 0.72, 760
local TAB_FADE_H = 46
-- (`tab_fade`'s alpha at the top bar's foot, for a shade of 1.)
local TAB_FADE_START = 0.55 * TAB_FADE_H / (TAB_FADE_H + TOP_BAR - 6)
local LOOK_BLUR, LOOK_SHADE, LOOK_DARKER = 24, 0.62, 0.42
local BLUR_STEP = 4
local BLEND_FADE = 260
-- Frosted, the picture lets the glass through a little (shell.rs).
local HERO_FROSTED = 0.84
local SCRIM_HERO = { angle = 180, stops = { { 0, "#0000001a" }, { 0.45, "#00000066" }, { 1, "#00000000" } } }
local SCRIM_FULL = { angle = 180, stops = { { 0, "#0000001a" }, { 0.45, "#00000066" }, { 1, "#000000bf" } } }
local TAB_FADE = { angle = 180, stops = { "#000f", "#0000" } }

M.page_art = ui.computed(function(): string?
	if route.current:get().name == "plugin" or appearance.showing:get() ~= "image" then
		return nil
	end
	local name = route.chat:get().name
	if name == "home" then
		return "hero"
	end
	return if name == "session" and appearance.chat_shown() then "chat" else nil
end)

local drawn = ui.state("shell_frame_drawn", false)
M.framed = ui.computed(function(): boolean
	return drawn:get() and appearance.inset_card:get() and appearance.frame_style:get() ~= "off" and M.page_art:get() ~= nil
end)

function M.hero_height(vh: number): number
	-- (Whole points: a part-covered last row keeps the art at full strength, a bright line.)
	return math.floor(math.min(math.max(vh, 0) * HERO_RATIO, HERO_MAX))
end

local function layer_height(kind: string?, vh: number): number
	if kind == "chat" and appearance.chat_placement:get() == "full" then
		return math.max(vh, 0)
	end
	return M.hero_height(vh)
end

local baked = ui.state("shell_art_baked", {} :: { [string]: string })
local asked: { [string]: boolean } = {}
local function blurred(path: string, radius: number): string?
	local key = `{path}|{radius}`
	local got = baked:get()[key]
	if got then
		return got
	end
	if not asked[key] then
		asked[key] = true
		host.call("wallpaper.blurred", { path = path, radius = radius }):after(function(file: any)
			if type(file) == "string" then
				local next = table.clone(baked:peek())
				next[key] = file
				baked:set(next)
			end
		end, function()
			asked[key] = nil
		end)
	end
	return nil
end

local function key_of(kind: string?, radius: number): string?
	local image = appearance.image:get()
	if not image or not kind then
		return ""
	end
	local src: string? = image.path
	if radius > 0 then
		src = blurred(image.path, radius)
	end
	if not src then
		return nil
	end
	if kind == "chat" then
		return `chat:0.5:0.5:1:{src}`
	end
	return `hero:{image.x}:{image.y}:{image.zoom}:{src}`
end

local function content_of(key: string): Node
	local x, y, zoom, path = string.match(key, "^%a+:([^:]*):([^:]*):([^:]*):(.*)$")
	local image = appearance.image:peek()
	return ds.wallpaper.Picture { src = path or "", w = if image then image.w else 1, h = if image then image.h else 1, x = tonumber(x), y = tonumber(y), zoom = tonumber(zoom) }
end

export type LayerProps = {
	kind: (() -> string?)?,
	origin: (() -> { x: number, y: number })?,
	left: (() -> number)?,
	blur: (() -> number)?,
	size: (() -> { w: number, h: number })?,
	cutout: (() -> any)?,
	scrim: boolean?,
	tab_fade: boolean?,
	unfaded: boolean?,
	id: string?,
}

function M.Layer(props: LayerProps): Node
	local function kind(): string?
		return if props.kind then props.kind() else M.page_art:get()
	end
	local function origin(): { x: number, y: number }
		return if props.origin then props.origin() else { x = 0, y = 0 }
	end
	local function window(): { w: number, h: number }
		return if props.size then props.size() else art.window:get()
	end
	local function place(): { left: number, top: number, w: number, h: number }
		local win = window()
		local from = if props.left then props.left() else 0
		return { left = from - origin().x, top = -origin().y, w = math.max(win.w - from, 0), h = layer_height(kind(), win.h) }
	end
	-- Snapped, as Zeron's chat blur: a few files, not one for every step of a drag.
	local function radius(): number
		local own = if kind() == "chat" then appearance.chat_blur:get() else 0
		local more = if props.blur then props.blur() else 0
		return math.floor(math.sqrt(own * own + more * more) / BLUR_STEP + 0.5) * BLUR_STEP
	end
	local showing = ""
	local wall: { [any]: any } = {
		id = props.id,
		fill = true,
		effect = function(): string
			return if radius() > 0 then "none" else appearance.effect:get()
		end,
		-- (A blurred copy on its way: what stands there stays till it's here.)
		key = function(): string
			local k = key_of(kind(), radius())
			if k then
				showing = k
			end
			return showing
		end,
		content = content_of,
		faded = function(): boolean
			return not props.unfaded and not (kind() == "chat" and appearance.chat_placement:get() == "full")
		end,
	}
	if props.cutout then
		local cutout = props.cutout
		wall.mask = function(): any
			return { fade = layer_height(kind(), window().h), reveal = 0.5, cutout = cutout() }
		end
	end
	local kids: { any } = {
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			opacity = function(): number
				if kind() == "chat" then
					return appearance.chat_opacity:get()
				end
				return if ds.material.see_through() then HERO_FROSTED else 1
			end,
			ds.Wallpaper(wall :: any),
		},
	}
	if props.scrim ~= false then
		for _, placement in { "hero", "full" } do
			table.insert(kids, ui.show(function(): boolean
				return kind() == "chat" and appearance.chat_scrim:get() and appearance.chat_placement:get() == placement
			end, ui.div {
				class = "art-scrim",
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				bg = "$zeron.window",
				mask_gradient = if placement == "full" then SCRIM_FULL else SCRIM_HERO,
			}))
		end
	end
	if props.tab_fade then
		table.insert(kids, ui.show(function(): boolean
			return kind() ~= nil
		end, ui.div {
			id = "shell-tab-fade",
			absolute = true,
			left = 0,
			right = 0,
			top = ui.raw(TOP_BAR, "the top bar's foot"),
			h = ui.raw(TAB_FADE_H, "the tab bar's fade"),
			bg = "$zeron.window",
			mask_gradient = TAB_FADE,
			opacity = function(): number
				return appearance.chat_tab_shade:get() * TAB_FADE_START
			end,
		}))
	end
	local box: { [any]: any } = {
		absolute = true,
		pointer_events = "none",
		left = function(): number
			return place().left
		end,
		top = function(): number
			return place().top
		end,
		w = function(): number
			return place().w
		end,
		h = function(): number
			return place().h
		end,
	}
	for _, k in kids do
		table.insert(box, k)
	end
	return ui.div(box)
end

local function quad(opacity: () -> number, id: string?): Node
	return ui.div {
		id = id,
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		bg = "$zeron.window",
		opacity = opacity,
	}
end

local function base_blur(): number
	return if appearance.frame_style:get() == "glass" then appearance.frame_blur:get() else 0
end
local function look_blur(): number
	return if appearance.frame_sidebar:get() == "glass" then math.max(base_blur(), LOOK_BLUR) else base_blur()
end
local function extra_shade(): number
	local look = appearance.frame_sidebar:get()
	if look == "darker" then
		return LOOK_DARKER
	elseif look == "glass" then
		return math.max(0, 1 - (1 - LOOK_SHADE) / math.max(1 - appearance.frame_shade:get(), 0.01))
	end
	return 0
end
local function fades(): boolean
	return appearance.frame_blend:get() == "fade" and appearance.frame_sidebar:get() ~= "plain"
end

local function framed_view(): Node
	return ui.div {
		id = "shell-frame",
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		overflow = "hidden",
		M.Layer {
			id = "shell-frame-art",
			blur = function(): number
				return if fades() then base_blur() else look_blur()
			end,
		},
		quad(function(): number
			local under = 1 - appearance.frame_shade:get()
			return if fades() then 1 - under else 1 - under * (1 - extra_shade())
		end, "shell-frame-shade"),
		ui.show(fades, ui.div {
			id = "shell-frame-look",
			absolute = true,
			left = 0,
			top = 0,
			bottom = 0,
			w = function(): number
				return art.card_left:get() + BLEND_FADE
			end,
			overflow = "hidden",
			fade_x = { 0, BLEND_FADE },
			ui.show(function(): boolean
				return look_blur() > base_blur()
			end, M.Layer { id = "shell-frame-look-art", blur = look_blur }),
			quad(function(): number
				if look_blur() > base_blur() then
					return 1 - (1 - appearance.frame_shade:get()) * (1 - extra_shade())
				end
				return extra_shade()
			end),
		}),
	}
end

local function band_view(): Node
	return ui.div {
		id = "shell-frame",
		absolute = true,
		left = function(): number
			return art.card_left:get()
		end,
		right = 0,
		top = 0,
		h = ui.raw(TOP_BAR, "the top bar"),
		pointer_events = "none",
		overflow = "hidden",
		bg = "$zeron.inset.bg",
		-- (A fade under this clip draws nothing, and the bar sees none of it anyway.)
		M.Layer {
			id = "shell-frame-art",
			unfaded = true,
			origin = function(): { x: number, y: number }
				return { x = art.card_left:get(), y = 0 }
			end,
			left = function(): number
				return art.card_left:get()
			end,
		},
		quad(function(): number
			return if appearance.inset_card:get() then appearance.chat_tab_shade:get() else 0
		end, "shell-frame-shade"),
	}
end

function M.View(): Node
	drawn:set(true)
	return ui.presence(function(): string
		if M.page_art:get() == nil then
			return ""
		end
		return if M.framed:get() then "frame" else "band"
	end, function(which: string): Node
		if which == "" then
			return ui.div {}
		end
		return if which == "band" then band_view() else framed_view()
	end, { absolute = true, left = 0, right = 0, top = 0, bottom = 0, pointer_events = "none" })
end

return M
