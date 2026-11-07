-- A reasoning-effort slider, Zeron's form with zgx's fill (locked with Reina,
-- .maestro/topic/zgx-port/DECISIONS.md): a 28-point full-round rail, a glass
-- thumb 44 by 32 that keeps its size and lifts on a softer shadow as it's
-- pressed, a fill running to the thumb's centre plus 14 (always hidden under
-- it), a ghost under the pointer showing where a press would land, a tick at
-- each level between the ends. Pressed, the thumb is under the pointer and
-- follows it as it drags, the level under it shown; let go, the value is that
-- level and the thumb settles on it. With focus, ←/→ (and Home/End) step it.
--
-- Its fill is a field of ASCII glyphs in the intro's 5 by 7 font
-- (decode.GLYPHS, ` .-+xXS80#@`), sparse and dim at the start, densest at the
-- thumb's left edge. Each level has its own colour and its own field (how
-- dense it gets, how much it wanders, how fast it drifts): the harder it
-- thinks, the busier the characters. A level's field mounts with its look and
-- crossfades in (a shader's parameters are given as its node is made; only
-- its ramp's end, which the thumb moves, is set on it after). Fast mode makes
-- it denser and quicker, with a halo breathing under the thumb in the level's
-- colour. It all stops with the panel it stands in, and stands still under
-- reduced motion.
--
-- `EffortSlider { value = effort, levels = { { value = "low", label = "Low" }, … } }`.
-- `levels` may be a function (the levels of a model that changes): the slider
-- follows them in place.

local motion = require("@self/motion")
local decode = require("@self/decode")
local locale = require("@self/locale")

export type Level = { value: string, label: string }
export type Props = {
	value: State<string?>,
	levels: { Level } | () -> { Level },
	on_change: ((value: string) -> ())?,
	-- Fast mode on.
	fast: (() -> boolean)?,
	-- Set to the level a drag is over, nil at rest (a title retitling as it
	-- drags).
	live: State<string?>?,
	-- Where the keys are while the panel has them (its accent ring).
	highlighted: (() -> boolean)?,
	-- A plain track for a choice that isn't reasoning (the context card's):
	-- the accent's tint, no field, nothing drifting; `none`, while true, no
	-- fill and no thumb (a value off its track).
	plain: boolean?,
	none: (() -> boolean)?,
	-- What assistive technology calls it.
	label: Label?,
	-- `false` inside a panel that keeps the keys for itself.
	keys: boolean?,
}

-- Its box, the rail in it, and the thumb (Zeron's compact.rs).
local H, RAIL, THUMB_W, THUMB_H = 36, 28, 44, 32
local RAIL_TOP = (H - RAIL) / 2
-- How far the fill runs past the thumb's centre.
local PAST = RAIL / 2

-- How often the field redraws as it drifts.
local FPS = 12
-- Fast mode's field and the halo's breath (one in a little under two seconds,
-- as Zeron's phase at full energy).
local FAST_TO, FAST_WANDER, FAST_DRIFT = 0.12, 0.1, 2.5
local BREATH_MS = 925

-- A level's look: its colour (a zeron.effort token) and its field: how
-- dense the glyphs get at the thumb, how far the density wanders, how fast
-- it drifts.
type Look = { token: string, to: number, wander: number, drift: number }
local LOOKS: { [string]: Look } = {
	none = { token = "none", to = 0.24, wander = 0.18, drift = 0.1 },
	minimal = { token = "minimal", to = 0.34, wander = 0.22, drift = 0.14 },
	low = { token = "low", to = 0.44, wander = 0.26, drift = 0.18 },
	medium = { token = "medium", to = 0.56, wander = 0.3, drift = 0.24 },
	high = { token = "high", to = 0.68, wander = 0.34, drift = 0.3 },
	xhigh = { token = "xhigh", to = 0.82, wander = 0.38, drift = 0.38 },
	max = { token = "max", to = 0.96, wander = 0.44, drift = 0.48 },
}
LOOKS.extended, LOOKS.ultra, LOOKS.deep = LOOKS.xhigh, LOOKS.max, LOOKS.max
local LADDER = { "none", "minimal", "low", "medium", "high", "xhigh", "max" }
-- A level the ladder doesn't name takes the look at its place in the list.
local function look_of(id: string, i: number, n: number): Look
	local known = LOOKS[id]
	if known then
		return known
	end
	local at = if n > 1 then (i - 1) / (n - 1) else 0
	return LOOKS[LADDER[math.clamp(math.floor(at * (#LADDER - 1) + 0.5) + 1, 1, #LADDER)]]
end

-- A look's colour: its token (each theme its own: deeper in light, where
-- the characters draw on a pale track). A wash of it is the token under an
-- opacity, so it follows the theme where it sits.
local function ink(look: Look): string
	return `$zeron.effort.{look.token}`
end

-- The field: cells of 6 by 8 font pixels (a pixel a whole number of device
-- pixels, so the glyphs stay crisp), rows centred in the box. A cell's
-- density ramps from the start to the thumb, wandering with a value noise
-- that drifts left over time; the density picks the glyph and how brightly
-- it is lit.
-- params[0]: ink; [1]: font pixel (points), density at the start, at the
-- end, curve; [2]: wander, drift (cells a second), seed, where the ramp
-- ends (points: the fill's end, so every level runs sparse to dense).
local FIELD = ui.shader(decode.GLYPHS .. [[
fn es_hash(p:vec2<f32>)->f32 { return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
fn es_noise(p:vec2<f32>)->f32 {
 let i=floor(p); let f=fract(p); let u=f*f*(3.-2.*f);
 let a=es_hash(i); let b=es_hash(i+vec2(1.,0.));
 let c=es_hash(i+vec2(0.,1.)); let d=es_hash(i+vec2(1.,1.));
 return mix(mix(a,b,u.x),mix(c,d,u.x),u.y);
}
fn fill(input:FillInput)->vec4<f32> {
 let ink=input.params[0];
 let r=input.params[1];
 let q=input.params[2];
 let px=max(1.,round(r.x*input.scale))/input.scale;
 let cell=vec2(6.,8.)*px;
 let rows=floor(input.size.y/cell.y);
 let top=(input.size.y-rows*cell.y)*.5;
 let p=vec2(input.p.x,input.p.y-top);
 if p.y<0. || p.y>=rows*cell.y {return vec4(0.);}
 let c=floor(p/cell);
 let u=clamp((c.x+.5)*cell.x/max(q.w,1.),0.,1.);
 var d=r.y+(r.z-r.y)*pow(u,r.w);
 d+=(es_noise(vec2(c.x*.31-input.time*q.y,c.y*.57+q.z))-.5)*q.x;
 d=clamp(d,0.,1.);
 let g=glyph(u32(d*10.999));
 let fp=vec2<u32>(floor((p-c*cell)/px));
 var lit=0.;
 if fp.x<5u && fp.y<7u {lit=f32((select(g.y,g.x>>(fp.y*5u),fp.y<6u)>>(4u-fp.x))&1u);}
 return vec4(ink.rgb,ink.a*lit*(.45+.55*d));
}
]], { fps = FPS })

-- A shadow in a token's colour at `alpha` (a shadow's fields, the colour read
-- from the theme where it's drawn).
local function shadow_of(token: string, y: number, blur: number, spread: number, alpha: number): any
	local hex = string.sub(tostring(ui.theme.get(token) or "#000000"), 1, 7)
	return ui.raw({ x = 0, y = y, blur = blur, spread = spread, color = `{hex}{string.format("%02x", math.floor(alpha * 255 + 0.5))}` }, "the level's own colour, which no token holds")
end

return ui.component(function(props: Props): Node
	local given = props.levels
	local function levels(): { Level }
		return if type(given) == "function" then given() else given
	end
	local function count(): number
		return #levels()
	end
	local track = ui.ref()
	-- The pointer's place across the track (a level's fraction), -1 outside.
	local hover = ui.use_state(-1)
	local pressed = ui.use_state(false)
	-- The level a drag is over, until it's let go (nil: no drag): the
	-- colour and the field follow it as the thumb crosses the levels, and
	-- the value is set once, as it's let go (setting it is the composer's
	-- pick and the model chip's label: none of that on each step of a drag).
	local live = ui.use_state(nil :: number?)
	local plain = props.plain == true
	local function none(): boolean
		return props.none ~= nil and props.none()
	end

	local function index(): number
		local dragged = live:get()
		if dragged then
			return dragged
		end
		local v = props.value:get()
		for i, l in levels() do
			if l.value == v then
				return i
			end
		end
		return 1
	end
	local function look(i: number): Look
		local list = levels()
		local l = list[i]
		return look_of(if l then l.value else "", i, #list)
	end
	local function fast(): boolean
		return props.fast ~= nil and props.fast()
	end
	local function width(): number
		return math.max(track.width or 200, THUMB_W)
	end
	-- Whether the track has been laid out (its width known).
	local function measured(): boolean
		return (track.width or 0) > THUMB_W
	end
	-- A level's fraction along the track, and the level nearest a fraction.
	local function frac(i: number): number
		local n = count()
		return if n > 1 then (i - 1) / (n - 1) else 0
	end
	local function nearest(f: number): number
		local n = count()
		return math.clamp(math.floor(f * (n - 1) + 0.5) + 1, 1, n)
	end
	-- Where the thumb centres at fraction `f`: the stops and the pointer share
	-- the thumb's half width as their inset.
	local function spot(f: number): number
		return THUMB_W / 2 + math.floor(f * (width() - THUMB_W) + 0.5)
	end
	-- The fill's end, and the field's ramp's (the thumb's left edge: so the
	-- densest glyphs show).
	local function fill_end(x: number): number
		return math.min(width(), x + PAST)
	end
	local function ramp_end(x: number): number
		return math.max(1, x - THUMB_W / 2)
	end
	-- The level's fraction under a pointer event.
	local function at(e: any): number
		local span = width() - THUMB_W
		if span <= 0 then
			return 0
		end
		return frac(nearest(math.clamp(((e.local_x or 0) - THUMB_W / 2) / span, 0, 1)))
	end
	-- Each level it lands on, a tap under the trackpad, as the shipping
	-- app's detents (src/haptics.rs; nothing where the host has none).
	local function tap()
		pcall(ui.invoke, "haptic.step")
	end
	local function set(i: number, quiet: boolean?)
		local l = levels()[i]
		if not l or l.value == props.value:peek() then
			return
		end
		if props.on_change then
			props.on_change(l.value)
		else
			props.value:set(l.value)
		end
		if not quiet then
			tap()
		end
	end

	-- Where the thumb centres, as drawn (the fill runs from it): held, under
	-- the pointer, set the moment the pointer moves (the frame it moves,
	-- nothing easing after it); let go, it settles on its level, and a
	-- level changed otherwise (the keys, another model) eases it there.
	-- (It sprang after the level, which moved only as the pointer crossed
	-- half way to the next: the thumb trailed the pointer, then caught up.)
	local pos = ui.use_state(0)
	local settling: any = nil
	local function put(x: number, ease: boolean)
		if settling then
			settling:cancel()
			settling = nil
		end
		if not ease or motion.reduced_now() or math.abs(pos:peek() - x) < 0.5 then
			pos:set(x)
			return
		end
		settling = anime.animate(pos, { value = x, duration = 140, ease = "out(3)" })
	end
	-- (On its level at once as the track is first measured: the slider
	-- opens with its thumb in place.)
	local placed = false
	ui.effect(function()
		if pressed:get() or not measured() then
			return
		end
		put(spot(frac(index())), placed)
		placed = true
	end)
	-- The thumb's centre under a pointer event, held to the track's ends,
	-- and the level nearest it.
	local function under(e: any): number
		return math.clamp(e.local_x or 0, spot(0), spot(1))
	end
	local function level_at(x: number): number
		local span = width() - THUMB_W
		return nearest(if span > 0 then (x - spot(0)) / span else 0)
	end
	local function say_live(i: number?)
		if props.live then
			local l = if i then levels()[i] else nil
			props.live:set(if l then l.value else nil)
		end
	end
	-- Held: the thumb where the pointer is, the level under it live.
	local function follow(e: any)
		local x = under(e)
		put(x, false)
		local i = level_at(x)
		if i ~= index() then
			live:set(i)
			say_live(i)
			tap()
		end
	end
	-- Let go: the level it was over taken, as the thumb settles on it.
	local function let_go()
		local i = live:peek()
		ui.batch(function()
			pressed:set(false)
			live:set(nil)
			say_live(nil)
			if i then
				set(i, true)
			end
		end)
	end

	-- Each look's fill, mounted with it and crossfading in over the last, once
	-- the track has a width: its tint, its field, its ring. The field's ramp
	-- ends at the thumb's left edge: set on it as the thumb moves (a level of
	-- the same look, or a new width, keeps the field).
	local fill = ui.presence(function(): string?
		if not measured() then
			return nil
		end
		if plain then
			return "plain"
		end
		return look(index()).token .. (if fast() then "+" else "")
	end, function(key: string): Node
		local l = look(index())
		local quick = string.sub(key, -1) == "+"
		local el = ui.ref()
		ui.effect(function()
			local ramp = ramp_end(pos:get())
			local node = el:peek()
			if node then
				anime.utils.set(node, { shaderParam11 = ramp })
			end
		end)
		local paint = if plain then "$accent" else ink(l)
		local to = if quick then math.min(1, l.to + FAST_TO) else l.to
		local wander = if quick then l.wander + FAST_WANDER else l.wander
		local drift = if quick then l.drift * FAST_DRIFT else l.drift
		return ui.div {
			absolute = true,
			left = 0,
			top = 0,
			right = 0,
			bottom = 0,
			pointer_events = "none",
			-- The level's tint.
			ui.div {
				absolute = true,
				left = 0,
				top = 0,
				right = 0,
				bottom = 0,
				opacity = "$zeron.slider.tint",
				bg = paint,
			},
			if plain
				then nil
				else ui.div {
					ref = el,
					absolute = true,
					left = 0,
					top = 0,
					w = width,
					h = RAIL,
					bg_shader = FIELD,
					-- 5 by 7 point characters in three rows, the ramp ending at
					-- the thumb. (A token: the shader's colour follows the theme.)
					shader_params = { paint, { 1, 0.04, to, 1.4 }, { wander, drift, 3, ramp_end(pos:peek()) }, { 0, 0, 0, 0 } },
					on_mount = function(node)
						anime.utils.set(node, { shaderParam11 = ramp_end(pos:peek()) })
						-- Drifting, unless motion is reduced.
						if not motion.reduced_now() then
							anime.animate(node, {
								shaderTime = { 0, 600 },
								duration = 600000,
								loop = true,
								ease = "linear",
								frameRate = FPS,
							})
						end
					end,
				},
			-- Its ring.
			ui.div {
				absolute = true,
				left = 0,
				top = 0,
				right = 0,
				bottom = 0,
				rounded = "$radius.full",
				border = 1,
				opacity = "$zeron.slider.ring",
				pointer_events = "none",
				border_color = paint,
			},
		}
	end, {
		absolute = true,
		left = 0,
		top = 0,
		right = 0,
		bottom = 0,
		enter = { opacity = { 0, 1 }, duration = 220 },
		exit = { opacity = 0, duration = 220 },
	})

	-- A tick at each level between the ends, each its place's, lit once the
	-- front has passed it.
	local ticks = ui.each(
		ui.computed(function(): { number }
			local out: { number } = {}
			for k = 2, count() - 1 do
				table.insert(out, k)
			end
			return out
		end),
		function(k: number): Node
			return ui.div {
				absolute = true,
				top = (H - 6) / 2,
				w = 2,
				h = 6,
				rounded = "$radius.full",
				pointer_events = "none",
				bg = function()
					return if not none() and pos:get() >= spot(frac(k)) - 0.5 then "$zeron.slider.stop_on" else "$zeron.slider.stop_off"
				end,
				left = function()
					return spot(frac(k)) - 1
				end,
			}
		end,
		{
			key = function(k: number): number
				return k
			end,
		}
	)

	-- What sits with the thumb, each sized as it is and at its place: a layer
	-- whose shadow fades in (a shadow can't be eased, its opacity can).
	local function thumb_layer(p: { [any]: any }): Node
		p.absolute = true
		p.left = 0
		p.top = 0
		p.w = THUMB_W
		p.h = THUMB_H
		p.rounded = "$radius.full"
		p.pointer_events = "none"
		return ui.div(p)
	end
	-- Fast mode's halo: the level's colour, breathing under the thumb.
	local function halo(): Node
		return ui.presence(function(): string?
			if plain or not fast() then
				return nil
			end
			return look(index()).token
		end, function(token: string): Node
			return thumb_layer {
				class = "effort-halo",
				shadow = function()
					return shadow_of(`zeron.effort.{token}`, 1, 10, 0, 0.35)
				end,
				on_mount = function(node)
					if motion.reduced_now() then
						anime.utils.set(node, { opacity = 0.775 })
					else
						anime.animate(node, {
							opacity = { 0.55, 1 },
							duration = BREATH_MS,
							loop = true,
							alternate = true,
							ease = "inOutSine",
							frameRate = FPS,
						})
					end
				end,
			}
		end, {
			absolute = true,
			left = 0,
			top = 0,
			w = THUMB_W,
			h = THUMB_H,
			enter = { opacity = { 0, 1 }, duration = 240, ease = "out(3)" },
			exit = { opacity = 0, duration = 240, ease = "out(3)" },
		})
	end

	local keys = props.keys ~= false
	return ui.div {
		ref = track,
		w_full = true,
		h = H,
		shrink = 0,
		relative = true,
		cursor = "pointer",
		-- (zgx has no slider role for a node of the script's own; ui.slider
		-- is its native one.)
		label = props.label or locale.t("ds.effort_slider.reasoning"),
		focusable = keys,
		on_hover = function(inside: boolean)
			if not inside then
				hover:set(-1)
			end
		end,
		on_press = function(e)
			pressed:set(true)
			follow(e)
		end,
		on_pointer_move = function(e)
			hover:set(at(e))
			if pressed:peek() then
				follow(e)
			end
		end,
		on_release = let_go,
		on_key = if keys
			then function(e)
				local i = index()
				if e.key == "ArrowLeft" or e.key == "ArrowDown" then
					set(math.max(1, i - 1))
				elseif e.key == "ArrowRight" or e.key == "ArrowUp" then
					set(math.min(count(), i + 1))
				elseif e.key == "Home" then
					set(1)
				elseif e.key == "End" then
					set(count())
				else
					return false
				end
				return true
			end
			else nil,
		-- The rail: neutral glass.
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = RAIL_TOP,
			h = RAIL,
			rounded = "$radius.full",
			gradient = function()
				return {
					angle = 180,
					stops = { ui.theme.get("zeron.slider.rail_top"), ui.theme.get("zeron.slider.rail_foot") },
				}
			end,
			border = 1,
			border_color = "$zeron.slider.rail_edge",
			inset_shadow = "$zeron.slider.rail_inset",
			shadow = "$zeron.slider.rail_drop",
			pointer_events = "none",
		},
		-- What moves (the ghost, the fill, the ticks, the thumb), mounted
		-- once the track has its width: built before it, they sprang from a
		-- stand-in width to their place each time the slider opened.
		ui.show(measured, ui.div {
			absolute = true,
			left = 0,
			top = 0,
			right = 0,
			bottom = 0,
			pointer_events = "none",
			-- The ghost: where a press would land.
			ui.div {
				absolute = true,
				left = 0,
				top = RAIL_TOP,
				h = RAIL,
				pointer_events = "none",
				w = ui.tween(ui.computed(function(): number
					return fill_end(spot(math.max(hover:get(), 0)))
				end), { duration = 140 }),
				opacity = ui.tween(ui.computed(function(): number
					return if hover:get() >= 0 then 1 else 0
				end), { duration = 120 }),
				ui.div {
					w_full = true,
					h_full = true,
					rounded = "$radius.full",
					opacity = 0.1,
					bg = function()
						local h = hover:get()
						return if plain then "$accent" else ink(look(if h >= 0 then nearest(h) else index()))
					end,
				},
			},
			-- The fill: the field over the level's tint, in its ring.
			ui.show(function(): boolean
				return not none()
			end, ui.div {
				class = "effort-fill",
				absolute = true,
				left = 0,
				top = RAIL_TOP,
				h = RAIL,
				rounded = "$radius.full",
				overflow = "hidden",
				pointer_events = "none",
				w = function(): number
					return fill_end(pos:get())
				end,
				fill,
			}),
			ticks,
			-- The thumb: a handle of its own in either theme, keeping its size; a
			-- press lifts it on a softer, wider shadow.
			ui.show(function(): boolean
				return not none()
			end, ui.div {
				-- (Named for tests: where it is drawn.)
				class = "effort-thumb",
				absolute = true,
				left = 0,
				pointer_events = "none",
				x = function(): number
					return pos:get() - THUMB_W / 2
				end,
				top = (H - THUMB_H) / 2,
				w = THUMB_W,
				h = THUMB_H,
				halo(),
				thumb_layer {
					class = "effort-press",
					shadow = "$zeron.slider.press",
					opacity = ui.tween(ui.computed(function(): number
						return if pressed:get() then 1 else 0
					end), { duration = 120 }),
				},
				if props.highlighted
					then ui.show(props.highlighted, thumb_layer {
						shadow = function()
							return shadow_of("accent", 0, 0, 3, 0.4)
						end,
					})
					else nil,
				thumb_layer {
					gradient = { angle = 180, stops = { "#ffffff", "#f6f6f6" } },
					border = 1,
					border_color = "$zeron.slider.thumb_edge",
					inset_shadow = "$zeron.slider.thumb_light",
					shadow = "$zeron.slider.thumb_drop",
				},
			}),
		}),
	}
end)
