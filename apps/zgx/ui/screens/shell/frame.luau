
local ds = require("@ds")
local appearance = require("@app/screens/shell/appearance")
local art = require("@app/screens/shell/art")
local route = require("@app/screens/shell/route")
local host = require("@app/screens/settings/host")
local motion = require("@ds/motion")
local composer_surface = require("@ds/composer_surface")
local view = require("@app/screens/shell/view")
local fit = require("@app/screens/shell/fit")

local M = {}

local TOP_BAR = 42
local HERO_RATIO, HERO_MAX = 0.72, 760
local TAB_FADE_H = 46
local GAP = (ui.theme.get("zeron.inset.gap") :: number?) or 4
local WINDOW_EDGE = 5
local CORNER_FADE = { duration = 180, ease = "cubicBezier(0, 0, 0.58, 1)" }
local RAIL_SCRIM_W = 72
local COMPOSER_RADIUS = (ui.theme.get("zeron.composer.radius") :: number?) or 26
local LOOK_BLUR, LOOK_SHADE, LOOK_DARKER = 24, 0.62, 0.42
local BLUR_STEP = 4
local BLEND_FADE = 260
local CARD_LIFT = { y = 6, blur = 20, color = "#00000059" }
-- Frosted, the picture lets the glass through a little (shell.rs).
local HERO_FROSTED = 0.84
-- (Plain gradients in the window's tone, not a mask: the second masked scrim in a split, the right pane's, draws opaque.)
ui.theme.tokens {
	zeron = {
		art_scrim = {
			top = "alpha({zeron.window}, 0.1)",
			mid = "alpha({zeron.window}, 0.4)",
			foot = "alpha({zeron.window}, 0.75)",
			rail = "alpha({zeron.window}, 0.72)",
			none = "alpha({zeron.window}, 0)",
		},
	},
}
local function scrim_of(placement: string): any
	local function at(name: string): any
		return ui.theme.get(`zeron.art_scrim.{name}`)
	end
	return { angle = 180, stops = { { 0, at("top") }, { 0.45, at("mid") }, { 1, at(if placement == "full" then "foot" else "none") } } }
end
local RAIL_SCRIM = { angle = 90, stops = { { 0, "#000000b8" }, { 1, "#00000000" } } }
-- The bar's shade, and crates/ui's `tab_fade` under it, with the selected tab's folder left out of it, so that tab is the card's own picture.
local BAR_SHADE = ui.layer_shader([[
fn folder(p: vec2<f32>, x0: f32, w: f32, top: f32, scale: f32) -> f32 {
	let flare = 8.0;
	let corner = 8.0;
	let foot = 37.0;
	var x = p.x - x0;
	let y = p.y - top;
	if (w <= 0.0 || x < 0.0 || x > w || y > foot + 1.0) {
		return 0.0;
	}
	if (x > w * 0.5) {
		x = w - x;
	}
	var edge = 0.0;
	if (x < flare) {
		let t = 1.0 - sqrt(max(1.0 - x / flare, 0.0));
		edge = foot - t * t * flare;
	} else if (x < flare + corner) {
		let t = 1.0 - sqrt((x - flare) / corner);
		edge = t * t * corner;
	}
	return clamp((y - edge) * scale + 0.5, 0.0, 1.0);
}
fn layer(input: LayerInput) -> vec4<f32> {
	let a = input.params[0];
	let b = input.params[1];
	let c = input.params[2];
	let y = input.p.y;
	let fade = 0.55 * clamp((88.0 - y) / 82.0, 0.0, 1.0);
	let bar = clamp((42.0 - y) * input.scale + 0.5, 0.0, 1.0);
	var hole = max(folder(input.p, a.x, a.y, b.z, input.scale), folder(input.p, a.z, a.w, b.z, input.scale));
	hole *= clamp((input.p.x - b.x) / max(c.x, 0.5), 0.0, 1.0) * clamp((b.y - input.p.x) / max(c.y, 0.5), 0.0, 1.0);
	let v = mix(mix(fade, 1.0, bar), fade, hole);
	return vec4<f32>(input.color.rgb, input.color.a * v);
}]])

M.page_art = ui.computed(function(): string?
	local page = route.current:get().name
	if page == "plugin" or page == "settings" or appearance.showing:get() ~= "image" then
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
-- (The selected tab's outline, on the same art with Inset card on or off.)
M.tab_framed = ui.computed(function(): boolean
	return drawn:get() and appearance.frame_style:get() ~= "off" and M.page_art:get() ~= nil
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
		local function bake(file: string)
			local next = table.clone(baked:peek())
			next[key] = file
			baked:set(next)
		end
		-- (Refused, as a picture outside the kept folder is: the picture unblurred, not no art at all.)
		host.call("wallpaper.blurred", { path = path, radius = radius }):after(function(file: any)
			bake(if type(file) == "string" then file else path)
		end, function()
			bake(path)
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
	blur: (() -> number)?,
	size: (() -> { w: number, h: number })?,
	cutout: (() -> any)?,
	scrim: boolean?,
	unfaded: (boolean | () -> boolean)?,
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
		return { left = -origin().x, top = -origin().y, w = math.max(win.w, 0), h = layer_height(kind(), win.h) }
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
			local off = if type(props.unfaded) == "function" then props.unfaded() else props.unfaded
			return not off and not (kind() == "chat" and appearance.chat_placement:get() == "full")
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
				gradient = function(): any
					return scrim_of(placement)
				end,
			}))
		end
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

local function folded(): boolean
	return view.collapsed:get() or not fit.current:get().sidebar
end

-- (In a split or under the expanded panel the picture isn't the page's: it can't run under the rail.)
function M.art_under_rail(): boolean
	return not appearance.inset_card:get() and folded() and M.page_art:get() ~= nil and not art.expanded:get()
end

local function plugin_wallpaper(): boolean
	local showing, page = appearance.showing:get(), route.current:get().name
	return showing ~= "none" and showing ~= "image" and page ~= "plugin" and page ~= "settings" and route.chat:get().name == "home"
end

local function shows_art(): boolean
	return M.page_art:get() ~= nil or plugin_wallpaper()
end

-- What stands on the bar over the picture: its quieter ink falls under 3:1 on a bright one.
function M.bar_theme(): any
	return if shows_art() then { ["text.secondary"] = "$text.primary" } else nil
end

local function page_x(): number
	if route.current:get().name == "settings" then
		local settings: any = require("@app/screens/shell/settings")
		return art.RAIL_W + WINDOW_EDGE + settings.list_w()
	end
	return art.page_left:get()
end

-- (Over Settings the card stops at the list at once, whatever its tween is still doing: the list has no fill of its own.)
local function clip_left(): number
	return if route.current:get().name == "settings" then page_x() else 0
end

local function moved(f: () -> number): any
	return motion.tween(ui.computed(f), motion.sidebar)
end

local function faded_in(f: () -> number): any
	return ui.tween(ui.computed(f), if motion.reduced_now() then { duration = motion.FADE, ease = CORNER_FADE.ease } else CORNER_FADE)
end

local function left_gap(): number
	if appearance.inset_card:get() then
		return GAP
	end
	return if M.art_under_rail() then -art.RAIL_W else 0
end

local function top_gap(): number
	return if appearance.inset_card:get() then GAP else 0
end

local function edge_gap(): number
	return if appearance.inset_card:get() then WINDOW_EDGE else 0
end

local function fill_token(): string
	return if M.framed:get() then "$zeron.inset.framed_bg" else "$zeron.inset.bg"
end

local function bar_shade(): Node
	return ui.div {
		ref = art.shade,
		id = "shell-bar-shade",
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		h = ui.raw(TOP_BAR + TAB_FADE_H, "the top bar and the tab's fade under it"),
		layer_shader = BAR_SHADE,
		layer_params = { { 0, 0, 0, 0 }, { 0, 0, 0, 0 }, { 0, 0, 0, 0 } },
		opacity = faded_in(function(): number
			return if shows_art() then appearance.chat_tab_shade:get() else 0
		end),
		ui.div { absolute = true, left = 0, right = 0, top = 0, bottom = 0, bg = "$zeron.window" },
	}
end

-- What the card shows, laid out in the window's own coordinates: it holds still as the card's edges move over it.
local function card_picture(): Node
	return ui.div {
		id = "shell-card-picture",
		absolute = true,
		left = function(): number
			return -page_x()
		end,
		ml = moved(function(): number
			return -left_gap()
		end),
		top = moved(function(): number
			return -top_gap()
		end),
		w = function(): number
			return art.window:get().w
		end,
		h = function(): number
			return art.window:get().h
		end,
		ui.show(function(): boolean
			return M.page_art:get() == "hero"
		end, M.Layer {
			id = "page-art",
			kind = function(): string?
				return if M.page_art:get() == "hero" then "hero" else nil
			end,
			cutout = function(): any
				local _ = composer_surface.moved:get()
				local el = composer_surface.element
				local w, h = el and el.width, el and el.height
				if not el or not el.x or not w or not h then
					return nil
				end
				local hero = M.hero_height(art.window:get().h)
				return { x = el.x, y = el.y, w = w, h = math.max(el.y + h, hero) - el.y, radius = COMPOSER_RADIUS, feather = math.clamp(hero * 0.52, 120, 280), clearance = 8 }
			end,
		}),
		M.Layer {
			id = "page-chat-art",
			-- (A fade under the bar's clip draws nothing, and the bar sees none of it anyway.)
			unfaded = function(): boolean
				return art.split:get()
			end,
			kind = function(): string?
				return if M.page_art:get() == "chat" then "chat" else nil
			end,
		},
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			hidden = function(): boolean
				return route.current:get().name == "settings"
			end,
			ds.Wallpaper {
				effect = appearance.effect_now,
				fill = true,
				shown = function(): boolean
					return route.chat:get().name == "home" and route.current:get().name ~= "plugin"
				end,
				key = function(): string
					local showing = appearance.showing:get()
					return if showing == "none" or showing == "image" then "" else showing
				end,
				content = function(_key: string): Node
					return ui.div { id = "new-chat-background", absolute = true, left = 0, right = 0, top = 0, bottom = 0 }
				end,
				-- (A record, not two numbers: a plugin reading it follows it with `zeron.live`, never built again as the pointer moves.)
				provide = {
					pointer = ui.computed(function(): { x: number, y: number }
						return { x = art.px:get(), y = art.py:get() }
					end),
				},
			},
		},
		bar_shade(),
		ui.div {
			id = "shell-rail-scrim",
			absolute = true,
			left = 0,
			top = 0,
			bottom = 0,
			w = ui.raw(RAIL_SCRIM_W, "the rail's scrim"),
			bg = "$zeron.window",
			mask_gradient = RAIL_SCRIM,
			opacity = faded_in(function(): number
				return if M.art_under_rail() then 1 else 0
			end),
		},
	}
end

local function card_view(): Node
	local radius = faded_in(function(): number
		return if appearance.inset_card:get() then (ui.theme.get("zeron.inset.radius") :: number?) or 12 else 0
	end)
	local whole = faded_in(function(): number
		return if art.split:get() then 0 else 1
	end)
	return ui.div {
		id = "shell-page-card",
		absolute = true,
		pointer_events = "none",
		left = function(): number
			return page_x() - clip_left()
		end,
		ml = moved(left_gap),
		right = art.panel_w,
		mr = moved(edge_gap),
		top = moved(top_gap),
		bottom = moved(edge_gap),
		rounded = radius,
		overflow = "hidden",
		opacity = function(): number
			return if art.expanded:get() then 0 else 1
		end,
		shadow = function(): any
			return if M.framed:get() and not art.split:get() then CARD_LIFT else false
		end,
		ui.div {
			id = "shell-page-fill",
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			bg = fill_token,
			opacity = whole,
		},
		-- (In a split the panes hold the art under the bar: what's left of it is the bar's.)
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = function(): any
				return if art.split:get() then nil else 0
			end,
			h = function(): any
				return if art.split:get() then ui.raw(TOP_BAR - top_gap(), "the bar in the card") else nil
			end,
			overflow = "hidden",
			bg = function(): string
				return if art.split:get() then fill_token() else "#00000000"
			end,
			card_picture(),
		},
		ui.show(function(): boolean
			return M.art_under_rail() and art.split:get()
		end, ui.div {
			id = "shell-rail-art",
			absolute = true,
			left = 0,
			top = ui.raw(TOP_BAR, "under the bar"),
			bottom = 0,
			w = art.RAIL_W,
			overflow = "hidden",
			bg = fill_token,
			M.Layer {
				kind = function(): string?
					return if M.page_art:get() == "chat" then "chat" else nil
				end,
				origin = function(): { x: number, y: number }
					return { x = 0, y = TOP_BAR }
				end,
			},
			ui.div {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				gradient = function(): any
					return { angle = 90, stops = { { 0, ui.theme.get("zeron.art_scrim.rail") }, { 1, ui.theme.get("zeron.art_scrim.none") } } }
				end,
			},
		}),
		ui.div {
			id = "shell-page-edge",
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = radius,
			border = 1,
			border_color = function(): string
				return if M.framed:get() then "$zeron.inset.framed_edge" else "$border.subtle"
			end,
			opacity = faded_in(function(): number
				return if appearance.inset_card:get() and not art.split:get() then 1 else 0
			end),
		},
	}
end

function M.View(): Node
	drawn:set(true)
	ui.effect(function()
		local on = M.art_under_rail()
		if art.under_rail:peek() ~= on then
			art.under_rail:set(on)
		end
	end)
	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		ui.presence(function(): string
			return if M.framed:get() then "frame" else ""
		end, function(which: string): Node
			return if which == "frame" then framed_view() else ui.div {}
		end, { absolute = true, left = 0, right = 0, top = 0, bottom = 0, pointer_events = "none" }),
		ui.div {
			absolute = true,
			left = clip_left,
			right = art.panel_gap,
			top = 0,
			bottom = 0,
			pointer_events = "none",
			overflow = "hidden",
			-- (A tween's timing is fixed as it's made, so a change of Reduce motion makes the card again.)
			ui.presence(function(): string
				return if motion.reduced() then "reduced" else "moving"
			end, function(_: string): Node
				return card_view()
			end, { absolute = true, left = 0, right = 0, top = 0, bottom = 0, pointer_events = "none" }),
		},
	}
end

return M
