local K = require("@app/screens/shell/whats_new_visuals/kit")

local MENU_AT, PICK_AT, PIN_AT = 0.5, 1.2, 1.6
local WIDTH, HEIGHT, RAIL, BAR = 300, 96, 26, 24
local TAB_W, TILE_W, TAB_H, GAP, RULE = 60, 18, 16, 3, 7
local NOTCH_L, NOTCH_R, PAD = RAIL + 34, 38, 5
local CELL, MARKS_TOP, PITCH = 14, 34, 17
local PAGE = "$background"
local WASH = "$zeron.whats_new.wash_10"
local lerp = K.lerp

local function dot(ink: string): Node
	return ui.div { shrink = 0, w = 7, h = 7, rounded = "$radius.full", bg = ink }
end

local function wash(shown: () -> number, radius: any): Node
	return ui.div { absolute = true, left = 0, right = 0, top = 0, bottom = 0, rounded = radius, bg = WASH, opacity = shown }
end

local function build(clock: K.Clock): Node
	local function pin(): number
		return K.ramp(clock.t(), PIN_AT, 420, K.EASE_OUT_QUINT)
	end
	local function one(): number
		return 1
	end
	local function none(): number
		return 0
	end
	local function tab(title: string, ink: string, x: () -> number, w: () -> number, fill: () -> number, named: () -> number, rises: boolean?): Node
		return ui.row {
			absolute = true,
			top = (BAR - TAB_H) / 2,
			left = 0,
			x = x,
			w = w,
			h = if rises then BAR - (BAR - TAB_H) / 2 else TAB_H,
			pb = if rises then K.raw((BAR - TAB_H) / 2) else nil,
			bg = if rises then PAGE else nil,
			px = K.raw(5.5),
			items = "center",
			overflow = "hidden",
			rounded = if rises then nil else K.raw(5),
			rounded_tl = if rises then K.raw(5) else nil,
			rounded_tr = if rises then K.raw(5) else nil,
			wash(fill, K.raw(5)),
			dot(ink),
			ui.row { absolute = true, left = 17, top = 0, bottom = 0, items = "center", opacity = named, K.text(9.5, "$text.primary", title) },
		}
	end
	local function mark(ink: string, y: () -> number, fill: () -> number): Node
		return ui.row {
			absolute = true,
			left = (RAIL - CELL) / 2,
			top = 0,
			y = y,
			w = CELL,
			h = CELL,
			items = "center",
			justify = "center",
			rounded = K.raw(5),
			wash(fill, K.raw(5)),
			dot(ink),
		}
	end
	local function menu(): number
		return K.ramp(clock.t(), MENU_AT, 140, K.EASE_OUT) * (1 - K.ramp(clock.t(), PIN_AT - 0.1, 120, K.EASE_IN))
	end
	local function picked(): number
		return K.ramp(clock.t(), PICK_AT, 120, K.EASE_OUT)
	end
	local rest_x = TILE_W + GAP + RULE
	return K.card {
		w = WIDTH,
		h = HEIGHT,
		shrink = 0,
		relative = true,
		overflow = "hidden",
		ui.div { absolute = true, left = RAIL, right = 4, top = 4, bottom = 4, rounded = K.raw(8), bg = PAGE },
		ui.div {
			absolute = true,
			left = NOTCH_L,
			right = NOTCH_R,
			top = 0,
			h = BAR,
			rounded_bl = K.raw(9),
			rounded_br = K.raw(9),
			bg = "$zeron.whats_new.block",
			ui.div {
				absolute = true,
				left = PAD,
				right = PAD,
				top = 0,
				bottom = 0,
				tab("Shop", "$accent", function(): number
					return lerp(0, rest_x, pin())
				end, function(): number
					return TAB_W
				end, none, one, true),
				tab("Login", "$warning", function(): number
					return lerp(TAB_W + GAP, 0, pin())
				end, function(): number
					return lerp(TAB_W, TILE_W, pin())
				end, pin, function(): number
					return 1 - pin()
				end),
				tab("Usage", "$text.tertiary", function(): number
					return lerp(2 * (TAB_W + GAP), rest_x + TAB_W + GAP, pin())
				end, function(): number
					return TAB_W
				end, none, one),
				ui.div {
					absolute = true,
					left = TILE_W + (GAP + RULE) / 2,
					top = (BAR - 10) / 2,
					w = 1,
					h = 10,
					bg = "$border.strong",
					opacity = pin,
				},
			},
		},
		ui.row {
			absolute = true,
			left = RAIL + 8,
			top = 4,
			h = BAR - 4,
			items = "center",
			gap = K.sp(4),
			K.glyph("arrow-left", 9, "$text.tertiary"),
			K.glyph("plus", 9, "$text.tertiary"),
		},
		ui.row { absolute = true, right = 12, top = 4, h = BAR - 4, items = "center", K.bar(18) },
		ui.column { absolute = true, left = RAIL + 16, top = BAR + 14, gap = K.sp(6), K.bar(120), K.bar(86), K.bar(104) },
		ui.row { absolute = true, left = 0, top = MARKS_TOP - 13, w = RAIL, justify = "center", K.glyph("lucide-chevron-down", 9, "$text.tertiary") },
		mark("$accent", function(): number
			return lerp(MARKS_TOP, MARKS_TOP + PITCH + 5, pin())
		end, one),
		mark("$warning", function(): number
			return lerp(MARKS_TOP + PITCH, MARKS_TOP, pin())
		end, pin),
		mark("$text.tertiary", function(): number
			return lerp(MARKS_TOP + 2 * PITCH, MARKS_TOP + 2 * PITCH + 5, pin())
		end, none),
		ui.div { absolute = true, left = (RAIL - 8) / 2, top = MARKS_TOP + PITCH + 1, w = 8, h = 1, bg = "$border.strong", opacity = pin },
		ui.row(K.surface {
			absolute = true,
			left = NOTCH_L + PAD + TAB_W + GAP + 10,
			top = BAR - 2,
			h = 20,
			px = K.sp(6),
			gap = K.sp(4),
			items = "center",
			bg = "$zeron.whats_new.panel",
			shadow = "$shadow.md",
			opacity = menu,
			wash(picked, K.raw(10)),
			K.glyph("pin", 10, "$text.secondary"),
			K.text(9.5, "$text.primary", "Pin tab"),
		}),
	}
end

return { period = 4.8, rest = 3, build = build }
