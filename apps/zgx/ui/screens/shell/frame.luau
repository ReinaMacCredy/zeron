
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
-- (params as session_tabs sets them, in the window's points: [0] the notch's left wall, its right wall, the card's top, the bar's foot. A wall away, the chrome runs on past the card's edge. [1] the chosen tab's left, right, top and corner: the card rises through the notch's floor there, as a browser's tab joins its page.)
M.NOTCH_FLARE, M.NOTCH_RADIUS = 14, 18
local NOTCH = [[
fn bay_d(input: LayerInput) -> f32 {
	let f = 14.0;
	let r = 18.0;
	let a = input.params[0];
	let u = min(input.p.x - a.x, a.y - input.p.x);
	let y = input.p.y - a.z;
	let depth = a.w - a.z;
	if (u >= 0.0) {
		if (y > depth - r && u < r) {
			return r - length(vec2<f32>(r - u, y - (depth - r)));
		}
		return min(u, depth - y);
	}
	if (y < f) {
		if (u > -f) {
			return length(vec2<f32>(u + f, y - f)) - f;
		}
		return -y;
	}
	if (y > depth - r) {
		return r - length(vec2<f32>(r - u, y - (depth - r)));
	}
	return u;
}
fn notch_d(input: LayerInput) -> f32 {
	let b = bay_d(input);
	let t = input.params[1];
	if (t.y - t.x < 1.0) {
		return b;
	}
	let r = t.w;
	let c = vec2<f32>((t.x + t.y) * 0.5, t.z + 40.0);
	let q = abs(input.p - c) - vec2<f32>((t.y - t.x) * 0.5 - r, 40.0 - r);
	let tab = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
	let k = 8.0;
	let h = max(k - abs(b - tab), 0.0) / k;
	return min(b, tab) - h * h * k * 0.25;
}
fn notch(input: LayerInput) -> f32 {
	return clamp(notch_d(input) * input.scale + 0.5, 0.0, 1.0);
}
]]
local NOTCH_KEEP = ui.layer_shader(NOTCH .. [[
fn layer(input: LayerInput) -> vec4<f32> {
	return vec4<f32>(input.color.rgb, input.color.a * notch(input));
}]])
local NOTCH_CUT = ui.layer_shader(NOTCH .. [[
fn layer(input: LayerInput) -> vec4<f32> {
	return vec4<f32>(input.color.rgb, input.color.a * (1.0 - notch(input)));
}]])
local NOTCH_EDGE = ui.layer_shader(NOTCH .. [[
fn layer(input: LayerInput) -> vec4<f32> {
	let a = input.params[0];
	if (min(input.p.x - a.x, a.y - input.p.x) < -16.0) {
		return input.color;
	}
	let d = notch_d(input);
	let line = clamp(-d * input.scale + 0.5, 0.0, 1.0) * clamp((d + 1.0) * input.scale + 0.5, 0.0, 1.0);
	return vec4<f32>(input.color.rgb, input.color.a * line);
}]])
local STRIP_BLUR = 28
local STRIP_IN = { angle = 90, stops = { { 0, "#00000000" }, { 0.12, "#000000ff" }, { 1, "#000000ff" } } }
ui.theme.tokens {
	zeron = {
		strip = {
			bg_light = "mix({zeron.window}, {accent}, 0.18)",
			bg_dark = "mix({zeron.window}, {accent}, 0.16)",
			art_light = "alpha({zeron.window}, 0.22)",
			art_dark = "alpha({zeron.window}, 0.47)",
			ink_light = "alpha({text.primary}, 0.72)",
			ink_dark = "alpha({text.primary}, 0.7)",
			row_hover = "alpha({text.primary}, 0.04)",
			row_selected = "alpha({text.primary}, 0.075)",
			row_rim = "alpha({text.primary}, 0.06)",
			row_tile = "alpha({text.primary}, 0.06)",
			row_rule = "alpha({text.primary}, 0.16)",
			row_hover_art = "alpha({text.primary}, 0.1)",
			row_selected_art = "alpha({text.primary}, 0.18)",
			row_rim_art = "alpha({text.primary}, 0.12)",
			row_tile_art = "alpha({text.primary}, 0.1)",
			row_rule_art = "alpha({text.primary}, 0.24)",
			tab_top = "alpha({zeron.window}, 0.55)",
			tab_foot = "alpha({zeron.window}, 0.31)",
		},
	},
}
function M.strip_token(name: string): string
	local kind = ui.theme.kind()
	local light = kind == "light" or kind == "high-contrast-light"
	return `$zeron.strip.{name}_{if light then "light" else "dark"}`
end

local shows_art: () -> boolean
function M.row_token(name: string): string
	return `$zeron.strip.row_{name}{if shows_art() then "_art" else ""}`
end

M.page_art = ui.computed(function(): string?
	local name = route.chat:get().name
	if name == "plugin" or appearance.showing:get() ~= "image" then
		return nil
	end
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
	local full = M.hero_height(vh)
	local lift, foot = composer_surface.lift:get(), composer_surface.foot:get()
	if kind == "hero" and lift > 0 and foot > 0 then
		return math.floor(full + (math.min(full, foot + 24) - full) * lift)
	end
	return full
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

local HERO_OUT = 260

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
	-- (The picture and where it's cut alone: a chat's that stands as a new chat's did is never made again.)
	if kind == "chat" then
		return `0.5:0.5:1:{src}`
	end
	return `{image.x}:{image.y}:{image.zoom}:{src}`
end

local function content_of(key: string): Node
	local x, y, zoom, path = string.match(key, "^([^:]*):([^:]*):([^:]*):(.*)$")
	local image = appearance.image:peek()
	return ds.wallpaper.Picture { src = path or "", w = if image then image.w else 1, h = if image then image.h else 1, x = tonumber(x), y = tonumber(y), zoom = tonumber(zoom) }
end

export type LayerProps = {
	kind: (() -> string?)?,
	origin: (() -> { x: number, y: number })?,
	blur: (() -> number)?,
	size: (() -> { w: number, h: number })?,
	cutout: (() -> any)?,
	gone: (() -> number)?,
	shown: (() -> boolean)?,
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
	if props.shown then
		wall.shown = props.shown
		wall.kept = true
	end
	if props.cutout then
		local cutout = props.cutout
		wall.mask = function(): any
			return { fade = layer_height(kind(), window().h), reveal = 0.5, cutout = cutout(), gone = if props.gone then props.gone() else 0 }
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
				return kind() == "chat" and (not props.shown or props.shown()) and appearance.chat_scrim:get() and appearance.chat_placement:get() == placement
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
	local showing = appearance.showing:get()
	return showing ~= "none" and showing ~= "image" and route.chat:get().name == "home"
end

function shows_art(): boolean
	return M.page_art:get() ~= nil or plugin_wallpaper()
end

-- What stands on the bar over the picture: its quieter ink falls under 3:1 on a bright one.
function M.bar_theme(): any
	return if shows_art() then { ["text.secondary"] = "$text.primary" } else nil
end

function M.island(): boolean
	return folded() and (M.page_art:get() == "hero" or plugin_wallpaper())
end

local function page_x(): number
	return art.page_left:get()
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


local function edge_gap(): number
	return if appearance.inset_card:get() then WINDOW_EDGE else 0
end

-- The strip's accent tint where no picture shows; the window round the cards (rail, sidebar, gaps) wears it too. Over a picture both are the sidebar's plain look.
local function plain_tint(): string
	return if shows_art() then "#00000000" else M.strip_token("bg")
end

local function fill_token(): string
	return if M.framed:get() then "$zeron.inset.framed_bg" else "$zeron.inset.bg"
end
M.card_fill = fill_token

-- crates/ui's `tab_fade` under the bar, in the card.
local function tab_fade(): Node
	return ui.div {
		id = "shell-tab-fade",
		absolute = true,
		left = 0,
		right = 0,
		top = ui.raw(TOP_BAR, "under the top bar"),
		h = ui.raw(TAB_FADE_H, "the tab's fade under the bar"),
		gradient = function(): any
			return { angle = 180, stops = { { 0, ui.theme.get("zeron.strip.tab_foot") }, { 1, ui.theme.get("zeron.art_scrim.none") } } }
		end,
		opacity = faded_in(function(): number
			return if shows_art() then appearance.chat_tab_shade:get() else 0
		end),
	}
end

local CUT_PARAMS = { { 0, 0, 0, 0 }, { 0, 0, 0, 0 }, { 0, 0, 0, 0 } }

-- (Between a new chat and a session the new chat's picture goes and comes by its own alpha, over the chat's: a fading box redraws the whole window, which costs the flying composer frames. gpui's dissolve.)
local art_gone = ui.state("shell_frame_art_gone", 0)
local art_leaving = ui.state("shell_frame_art_leaving", false)
local art_coming = ui.state("shell_frame_art_coming", false)

-- What the card shows, laid out in the window's own coordinates: it holds still as the card's edges move over it.
local function card_picture(): Node
	local was, was_on = M.page_art:peek(), route.chat:peek().name
	local run: any = nil
	ui.effect(function()
		local now, on = M.page_art:get(), route.chat:peek().name
		local moves = not motion.reduced_now()
		local landing = moves and was == "hero" and now ~= "hero" and on == "session"
		local rising = moves and now == "hero" and was ~= "hero" and was_on == "session"
		was, was_on = now, on
		if run then
			run:cancel()
			run = nil
		end
		art_leaving:set(landing)
		art_coming:set(rising)
		if landing then
			run = anime.animate(art_gone, {
				value = 1,
				delay = composer_surface.TAKEOFF + 0.06 * 420,
				duration = 0.82 * 420,
				ease = "linear",
				onComplete = function()
					art_leaving:set(false)
				end,
			})
		elseif rising then
			run = anime.animate(art_gone, {
				value = 0,
				delay = 0.08 * 470,
				duration = 0.77 * 470,
				ease = "linear",
				onComplete = function()
					art_coming:set(false)
				end,
			})
		else
			art_gone:set(if now == "hero" then 0 else 1)
		end
	end)
	return ui.div {
		id = "shell-card-picture",
		absolute = true,
		left = function(): number
			return -page_x()
		end,
		ml = moved(function(): number
			return -left_gap()
		end),
		top = ui.raw(-TOP_BAR, "the card starts at the bar's foot"),
		w = function(): number
			return art.window:get().w
		end,
		h = function(): number
			return art.window:get().h
		end,
		-- (Under the new chat's, and kept built on a new chat: there at once as that one thins, never faded in under it.)
		M.Layer {
			id = "page-chat-art",
			-- (A fade under the bar's clip draws nothing, and the bar sees none of it anyway.)
			unfaded = function(): boolean
				return art.split:get()
			end,
			kind = function(): string?
				local page = M.page_art:get()
				return if page == "chat" or (page == "hero" and appearance.chat_shown()) then "chat" else nil
			end,
			shown = function(): boolean
				return M.page_art:get() == "chat" or art_coming:get()
			end,
		},
		-- (Leaving for anything but a session, it fades.)
		ui.presence(function(): string?
			return if M.page_art:get() == "hero" or art_leaving:get() then "hero" else nil
		end, function(): Node
			return M.Layer {
				id = "page-art",
				kind = function(): string?
					return "hero"
				end,
				gone = function(): number
					return art_gone:get()
				end,
				shown = function(): boolean
					return M.page_art:get() == "hero" or art_gone:get() < 1
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
			}
		end, {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			pointer_events = "none",
			exit = motion.transition({ opacity = 0, duration = HERO_OUT, ease = "in(2)" }),
		}),
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
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
		tab_fade(),
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

local function inset_radius(): number
	return if appearance.inset_card:get() then (ui.theme.get("zeron.inset.radius") :: number?) or 12 else 0
end

-- The bar over the card (Reina's N1): the card's own surface at both corners, the window's chrome dipping into it between them, as a browser's tab notch; the tabs stand in the chrome (screens/shell/session_tabs, which says where its walls are). Framed, the chrome is the frame's band blurred and tinted. The sidebar away, it runs from the window's edge, under the lights.
function M.band_top(): number
	return if appearance.inset_card:get() then GAP else 0
end

local function strip_view(): Node
	local function start(): number
		return if folded() then 0 else page_x()
	end
	local function start_gap(): number
		return if folded() then 0 else left_gap()
	end
	local function top(): number
		return if appearance.inset_card:get() and not folded() then GAP else 0
	end
	local function top_radius(): number
		return if folded() then 0 else inset_radius()
	end
	local function window_box(kids: { any }, from: () -> number, gap: () -> number, down: () -> number): Node
		local box: { [any]: any } = {
			absolute = true,
			left = function(): number
				return -from()
			end,
			ml = moved(function(): number
				return -gap()
			end),
			top = moved(function(): number
				return -down()
			end),
			w = function(): number
				return art.window:get().w
			end,
			h = ui.raw(TOP_BAR + 1, "the top bar and the card's first line"),
		}
		for _, k in kids do
			table.insert(box, k)
		end
		return ui.div(box)
	end
	local function card_band(props: { [any]: any }): Node
		props.absolute = true
		props.left = page_x
		props.ml = moved(left_gap)
		props.right = art.panel_w
		props.mr = moved(edge_gap)
		props.top = moved(M.band_top)
		props.bottom = 0
		props.overflow = "hidden"
		props.rounded_tl = faded_in(inset_radius)
		props.rounded_tr = faded_in(inset_radius)
		return ui.div(props)
	end
	local function framed(): boolean
		return M.framed:get()
	end
	local function edge_color(): string
		return if framed() then "$zeron.inset.framed_edge" else "$border.subtle"
	end
	local strip = ui.div {
		absolute = true,
		left = start,
		ml = moved(start_gap),
		right = art.panel_w,
		mr = moved(edge_gap),
		top = moved(top),
		bottom = 0,
		overflow = "hidden",
		rounded_tl = faded_in(top_radius),
		rounded_tr = faded_in(inset_radius),
		bg = function(): string
			if framed() then
				return "$zeron.window"
			elseif M.page_art:get() ~= nil then
				return fill_token()
			end
			return plain_tint()
		end,
		mask_gradient = function(): any
			return if framed() then STRIP_IN else nil
		end,
		window_box({
			ui.show(function(): boolean
				return M.page_art:get() ~= nil
			end, M.Layer {
				id = "shell-strip-art",
				blur = function(): number
					return STRIP_BLUR
				end,
				unfaded = true,
			}),
			ui.show(framed, quad(function(): number
				return if fades() then appearance.frame_shade:get() else 1 - (1 - appearance.frame_shade:get()) * (1 - extra_shade())
			end)),
			ui.show(function(): boolean
				return shows_art() and not framed()
			end, ui.div {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				bg = function(): string
					return M.strip_token("art")
				end,
			}),
		}, start, start_gap, top),
	}
	return ui.div {
		id = "shell-strip",
		absolute = true,
		left = 0,
		right = art.panel_gap,
		top = 0,
		h = ui.raw(TOP_BAR + 1, "the top bar and the card's first line"),
		pointer_events = "none",
		hidden = function(): boolean
			return art.expanded:get()
		end,
		ui.div {
			id = "shell-notch",
			ref = art.strip,
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			h = ui.raw(TOP_BAR, "the top bar"),
			layer_shader = NOTCH_KEEP,
			layer_params = CUT_PARAMS,
			strip,
		},
		ui.div {
			id = "shell-bar-surface",
			ref = art.tab,
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			h = ui.raw(TOP_BAR, "the top bar"),
			layer_shader = NOTCH_CUT,
			layer_params = CUT_PARAMS,
			card_band {
				bg = fill_token,
				window_box({
					-- (Unfaded: a faded picture inside this cut layer draws nothing.)
					M.Layer { id = "shell-tab-art", unfaded = true },
					ui.div {
						absolute = true,
						left = 0,
						right = 0,
						top = 0,
						h = ui.raw(TOP_BAR, "the top bar"),
						gradient = function(): any
							return { angle = 180, stops = { { 0, ui.theme.get("zeron.strip.tab_top") }, { 1, ui.theme.get("zeron.strip.tab_foot") } } }
						end,
						opacity = function(): number
							return if shows_art() then appearance.chat_tab_shade:get() else 0
						end,
					},
				}, page_x, left_gap, M.band_top),
			},
		},
		ui.div {
			id = "shell-notch-edge",
			ref = art.edge,
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			layer_shader = NOTCH_EDGE,
			layer_params = CUT_PARAMS,
			opacity = faded_in(function(): number
				return if appearance.inset_card:get() then 1 else 0
			end),
			card_band {
				border = 1,
				border_sides = { 1, 1, 0, 1 },
				border_color = edge_color,
				window_box({
					ui.div {
						absolute = true,
						left = function(): number
							return math.max(art.notch:get().x0 - M.NOTCH_FLARE - 1, -8)
						end,
						w = function(): number
							local n = art.notch:get()
							local from = math.max(n.x0 - M.NOTCH_FLARE - 1, -8)
							return math.max(math.min(n.x1 + M.NOTCH_FLARE + 1, art.window:get().w + 8) - from, 0)
						end,
						top = function(): number
							return M.band_top() + 1
						end,
						bottom = 0,
						bg = edge_color,
					},
				}, page_x, left_gap, M.band_top),
			},
		},
	}
end

local function card_view(): Node
	local radius = faded_in(inset_radius)
	local top_left = faded_in(function(): number
		return if art.notch:get().left then 0 else inset_radius()
	end)
	local top_right = faded_in(function(): number
		return if art.notch:get().right then 0 else inset_radius()
	end)
	local whole = faded_in(function(): number
		return if art.split:get() then 0 else 1
	end)
	return ui.div {
		id = "shell-page-card",
		absolute = true,
		pointer_events = "none",
		left = function(): number
			return page_x()
		end,
		ml = moved(left_gap),
		right = art.panel_w,
		mr = moved(edge_gap),
		top = ui.raw(TOP_BAR, "the card starts at the bar's foot, as gpui's"),
		bottom = moved(edge_gap),
		rounded_tl = top_left,
		rounded_tr = top_right,
		rounded_bl = radius,
		rounded_br = radius,
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
		-- (In a split the panes hold the art.)
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			overflow = "hidden",
			hidden = function(): boolean
				return art.split:get()
			end,
			card_picture(),
		},
		ui.show(function(): boolean
			return M.art_under_rail() and art.split:get()
		end, ui.div {
			id = "shell-rail-art",
			absolute = true,
			left = 0,
			top = 0,
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
			rounded_tl = top_left,
			rounded_tr = top_right,
			rounded_bl = radius,
			rounded_br = radius,
			border = 1,
			border_sides = { 0, 1, 1, 1 },
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
		ui.div {
			id = "shell-window-tint",
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			pointer_events = "none",
			bg = plain_tint,
		},
		ui.presence(function(): string
			return if M.framed:get() then "frame" else ""
		end, function(which: string): Node
			return if which == "frame" then framed_view() else ui.div {}
		end, { absolute = true, left = 0, right = 0, top = 0, bottom = 0, pointer_events = "none" }),
		ui.div {
			absolute = true,
			left = 0,
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
		strip_view(),
	}
end

return M
