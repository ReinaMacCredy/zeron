local motion = require("@self/motion")
local Icon = require("@self/icon")
local palettes = require("@self/palettes")
local status = require("@self/status")

export type Phase = "available" | "downloading" | "ready" | "failed"

local BOX, CIRCLE, ICON = 34, 28, 16
local UNIT = ICON / 24
local DROP_MS, ARC_MS, HALO_MS = 1100, 1200, 2600
local ARC_SPAN, ARC_RADIUS, ARC_STROKE = 0.26, 15.8, 2
local POP_MS, POP_PEAK = 450, 1.18
local GROW_MS, CHECK_MS, FADE_MS, TURN_MS = 200, 900, 100, 500
local HALOS = {
	{ d = CIRCLE + 6, shift = 0, peak = 0.32 },
	{ d = CIRCLE + 14, shift = -0.08, peak = 0.16 },
}
local EASE_OUT, EASE_IN_OUT = "cubicBezier(0, 0, .58, 1)", "cubicBezier(.42, 0, .58, 1)"
local FPS, HALO_FPS = 30, 15
local PARTS = { "#update-glyph-disc", "#update-glyph-check", "#update-glyph-rest" }

local function centered(size: number, props: { [any]: any }): any
	props.absolute = true
	props.left = (BOX - size) / 2
	props.top = (BOX - size) / 2
	props.w = size
	props.h = size
	props.pointer_events = "none"
	return props
end

local function lighter(): string
	ui.theme.current:get()
	local accent = ui.theme.get("accent")
	if type(accent) ~= "string" then
		return "$accent"
	end
	local h, s, l = palettes.hsl(accent)
	return palettes.from_hsl(h, s, math.min(l + 0.16, 0.92))
end

-- The loop starts from a wrapper: a node at opacity 0 never reports itself visible.
local function halo(i: number, fill: () -> string): Node
	local h = HALOS[i]
	local id = `#update-glyph-halo-{i}`
	local rise, fall = 0.12 * HALO_MS, 0.4 * HALO_MS
	return status.looping(
		centered(h.d, { ui.div { id = string.sub(id, 2), w = h.d, h = h.d, rounded = "$radius.full", bg = fill, opacity = 0 } }),
		function()
			return anime.animate(id, {
				opacity = {
					{ from = 0, to = h.peak, duration = rise, ease = "linear" },
					{ to = 0, duration = fall, ease = "linear" },
					{ to = 0, duration = HALO_MS - rise - fall },
				},
				loop = true,
				frameRate = HALO_FPS,
			})
		end,
		HALO_MS,
		h.shift * HALO_MS,
		id
	)
end

local function arc(): Node
	local path = ui.path():arc(BOX / 2, BOX / 2, ARC_RADIUS, -90, ARC_SPAN * 360)
	return status.looping(
		centered(BOX, {
			id = "update-glyph-arc",
			ui.canvas {
				w = BOX,
				h = BOX,
				draw = function(c)
					c:stroke(path, lighter(), ARC_STROKE)
				end,
			},
		}),
		function(el)
			return anime.animate(el, { rotate = { 0, 360 }, duration = ARC_MS, ease = "linear", loop = true, frameRate = FPS })
		end,
		ARC_MS
	)
end

local function arrow(): Node
	return status.looping(
		centered(ICON, { ui.div { id = "update-glyph-arrow", opacity = 0, color = "$accent.text", Icon { name = "lucide-download-arrow", size = ICON } } }),
		function()
			return anime.animate("#update-glyph-arrow", {
				y = {
					{ from = -9 * UNIT, to = UNIT, duration = 0.7 * DROP_MS, ease = "inQuad" },
					{ to = 7 * UNIT, duration = 0.3 * DROP_MS, ease = "inQuad" },
				},
				opacity = {
					{ from = 0, to = 1, duration = 0.25 * DROP_MS, ease = "linear" },
					{ to = 1, duration = 0.45 * DROP_MS },
					{ to = 0, duration = 0.3 * DROP_MS, ease = "linear" },
				},
				loop = true,
				frameRate = FPS,
			})
		end,
		DROP_MS,
		nil,
		"#update-glyph-arrow"
	)
end

local function at_rest()
	anime.utils.remove(PARTS)
	anime.utils.set("#update-glyph-disc", { scale = 1 })
	anime.utils.set("#update-glyph-check", { opacity = 0, scale = 1 })
	anime.utils.set("#update-glyph-rest", { opacity = 1, scale = 1, rotate = 0 })
end

local function enter()
	anime.animate("#update-glyph-disc", {
		scale = {
			{ from = 1, to = POP_PEAK, duration = 0.4 * POP_MS, ease = EASE_OUT },
			{ to = 1, duration = 0.6 * POP_MS, ease = EASE_IN_OUT },
		},
		frameRate = FPS,
	})
	anime.animate("#update-glyph-check", {
		opacity = {
			{ from = 0, to = 1, duration = GROW_MS, ease = EASE_OUT },
			{ to = 1, duration = CHECK_MS - GROW_MS - FADE_MS },
			{ to = 0, duration = FADE_MS, ease = "linear" },
		},
		scale = { from = 0.6, to = 1, duration = GROW_MS, ease = EASE_OUT },
		frameRate = FPS,
	})
	anime.animate("#update-glyph-rest", {
		opacity = { { from = 0, to = 0, duration = CHECK_MS }, { to = 1, duration = TURN_MS, ease = EASE_OUT } },
		scale = { { from = 0.4, to = 0.4, duration = CHECK_MS }, { to = 1, duration = TURN_MS, ease = EASE_OUT } },
		rotate = { { from = -200, to = -200, duration = CHECK_MS }, { to = 0, duration = TURN_MS, ease = EASE_OUT } },
		frameRate = FPS,
	})
end

return function(props: { phase: () -> Phase }): Node
	local phase = props.phase
	local function fill(): string
		return if phase() == "failed" then "$danger" else "$accent"
	end
	local function moving(p: Phase): () -> boolean
		return function(): boolean
			return phase() == p and not motion.reduced()
		end
	end
	-- The check pops in only on a change to ready while shown, as gpui's starts from none.
	local seen: Phase? = nil
	ui.effect(function()
		local p = phase()
		local before = seen
		seen = p
		if before == nil or before == p then
			return
		end
		at_rest()
		if p == "ready" and not motion.reduced_now() then
			enter()
		end
	end)
	return ui.div {
		relative = true,
		w = BOX,
		h = BOX,
		shrink = 0,
		pointer_events = "none",
		ui.show(moving("ready"), halo(2, fill)),
		ui.show(moving("ready"), halo(1, fill)),
		ui.show(moving("downloading"), arc()),
		ui.div(centered(CIRCLE, { id = "update-glyph-disc", rounded = "$radius.full", bg = fill })),
		ui.div(centered(ICON, {
			id = "update-glyph-rest",
			color = "$accent.text",
			Icon {
				name = function(): Icon.Name
					local p = phase()
					if p == "ready" then
						return "lucide-rotate-cw"
					end
					return if p == "downloading" and not motion.reduced() then "lucide-download-tray" else "lucide-download"
				end,
				size = ICON,
			},
		})),
		ui.show(moving("downloading"), arrow()),
		ui.div(centered(ICON, { id = "update-glyph-check", opacity = 0, color = "$accent.text", Icon { name = "lucide-check", size = ICON } })),
	}
end
