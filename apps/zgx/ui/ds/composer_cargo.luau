-- What a message carries, as the composer shows it over the draft (Zeron's
-- strip, composer.rs `render_attachment_strip`): tiles that wrap, bottom
-- aligned, an image 120 square and a file 56 square, with the row chips
-- (annotations, quotes, chats) and the one badge for review comments as the
-- last cells; Appshots over them in a strip of their own that scrolls.
--
-- A remove disc hangs at a tile's corner under the pointer or the keys,
-- taking no room. Text pasted whole and a linked file are small chips of a
-- label, where Zeron has them in the draft's own text.
--
-- Past what a short window can hold (Zeron's wrap has no limit and runs the
-- composer off the top), the strip shows what fits and "+N more".
--
-- The chip under the pointer is `hovered` (its id), for whatever shows what
-- a chip stands for elsewhere (the page's mark firms).
--
-- `Strip { items = cargo, on_remove = …, on_retry = …, on_open = …,
-- on_open_pasted = … }`.

local Icon = require("@self/icon")
local HoverCard = require("@self/hover_card")
local Mark = require("@self/mark")
local FileIcon = require("@self/file_icons")
local FadeText = require("@self/fade_text")
local Menu = require("@self/menu")
local control = require("@self/control")
local locale = require("@self/locale")
local motion = require("@self/motion")

local M = {}

-- ("comment": a review comment left on a file; "pick": ds/html_picks' tray.)
export type Kind = "image" | "file" | "text" | "annotation" | "comment" | "appshot" | "chat" | "quote" | "pick"
export type Item = {
	id: string,
	kind: Kind,
	name: string,
	-- An image: its picture.
	src: string?,
	-- What it is, under its name ("Rust · 24 KB", "412 lines · 18 KB").
	meta: string?,
	-- Still uploading: how far, 0 to 1.
	progress: number?,
	-- Failed: why, in a few words ("Couldn't upload").
	failed: string?,
	-- An annotation: the element's selector ("button#save.btn.primary"),
	-- the page it is on, its box, and its markup (a few lines).
	element: string?,
	page: string?,
	box: { number }?,
	html: string?,
	-- A quoted passage, whole: its card shows it.
	quote: string?,
	-- Selected text staged (kind "quote"): its words, its comment (nil: a
	-- plain selection), the file it was selected in, and where its badge
	-- stands (none: nothing to edit it in).
	text: string?,
	comment: string?,
	source: string?,
	anchor: any?,
	-- An Appshot (screens/chat/appshots): its picture is `src`, its app's
	-- icon `icon`; what the host writes of it rides in `appshot`.
	icon: string?,
	appshot: any?,
	bytes: number?,
	-- An annotation's number, the note typed for it, and whether its name
	-- is code (a tag and class) rather than words.
	index: number?,
	note: string?,
	code: boolean?,
	path: string?,
	-- Its number in the draft, for `@` (screens/chat/shared).
	number: number?,
	-- What a pasted or linked chip reads.
	label: string?,
	pasted: boolean?,
	linked: boolean?,
}
export type Props = {
	items: State<{ Item }>,
	on_remove: ((id: string) -> ())?,
	on_retry: ((id: string) -> ())?,
	on_open: ((id: string) -> ())?,
	-- A pasted chip pressed: its text back in the draft.
	on_open_pasted: ((id: string) -> ())?,
	-- An image chip as the lightbox's thumbnail (ds/lightbox `source`): the
	-- props that let its picture grow from the chip and go back into it.
	source: ((item: Item) -> { [string]: any }?)?,
}

local IMAGE_TILE, IMAGE_RADIUS, FILE_TILE, FILE_RADIUS = 120, 12, 56, 8
local TILE_ROUND = ui.raw(IMAGE_RADIUS, "Zeron's strip tile")
local MENU_GAP = "$zeron.space.hair"
local PAD_TOP = 12
local SHOT_W, SHOT_H = 208, 128
-- A row chip's and the badge's width as the fold counts them (Zeron's
-- STAGED_BADGE_WIDTH).
local CELL_W = 100
local BADGE_H, PILL_H = 24, 24
-- What the window keeps of itself beside the strip before it folds: the
-- title bar, the draft and its controls, a little of the transcript.
local WINDOW_KEEPS = 320
-- The glyph's slot, and the ring in it.
local SLOT, RING = 18, 18
-- The ×: its disc, and how far it hangs past the chip's corner.
local CROSS, CROSS_OUT = 18, 5
local DISC, DISC_OUT = 18, 6
-- How the × comes and goes.
local SWAP: TweenOptions = { duration = 140 }
-- (The icon set has no link.)
local LINK = [==[code:<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>]==]

-- The chip under the pointer, by id (nil: none).
M.hovered = ui.state("composer_cargo_hovered", nil :: string?)

local function over(item: Item, inside: boolean)
	if inside then
		M.hovered:set(item.id)
	elseif M.hovered:peek() == item.id then
		M.hovered:set(nil)
	end
end

local function n(token: string, fallback: number): number
	local v = ui.theme.get(token)
	return if type(v) == "number" then v else fallback
end

-- The hollow round a chip's ×: the chip cleared in a disc as wide as the
-- button and the hollow's width round it, edged over half a point either
-- side (no fade past that), so what is behind the strip (the pill's
-- glass, or its fill) shows in the gap: a cut, not a painted ring, as
-- ds/avatar cuts a stack. The × stands in the cut, outside the chip's
-- layer, so its fill lies on what is behind the chip, one even disc
-- whether over the chip or past its corner. It is the chip's whole layer,
-- so a picture is cut as a file is.
-- params[0]: the ×'s centre (in from the box's right edge, down from its
-- top), the cut's radius, and how far it is open (0 to 1, with the ×).
local HOLLOW = ui.layer_shader([[
fn layer(input:LayerInput)->vec4<f32> {
 let c=vec2(input.size.x-input.params[0].x,input.params[0].y);
 let d=distance(input.p,c);
 let cut=(1.-smoothstep(-.5,.5,d-input.params[0].z))*clamp(input.params[0].w,0.,1.);
 return vec4(input.color.rgb,input.color.a*(1.-cut));
}
]])

-- The hollow's props for a chip whose × is lit by `lit`: the layer and its
-- parameters, the cut opening as the × fades in (and closing as it goes).
local function hollow(lit: () -> boolean): { [string]: any }
	local box = ui.ref()
	local width = n("zeron.composer.chip.hollow", 2)
	ui.effect(function()
		local on = lit()
		local el = box:peek()
		if el then
			anime.animate(el, { layerParam3 = if on then 1 else 0, duration = SWAP.duration })
		end
	end)
	return {
		ref = box,
		layer_shader = HOLLOW,
		layer_params = { { CROSS / 2 - CROSS_OUT, CROSS / 2 - CROSS_OUT, CROSS / 2 + width, 0 } },
	}
end

-- A ring of `ink`, `f` of the way round from the top, over a faint full
-- turn: the upload's progress, drawn where it is.
local function ring(f: () -> number, ink: string, faint: string): Node
	return ui.canvas {
		w = RING,
		h = RING,
		shrink = 0,
		draw = function(c, w, h)
			local r = w / 2 - 2
			c:stroke(ui.path():arc(w / 2, h / 2, r, 0, 360), faint, 2)
			local v = math.clamp(f(), 0, 1)
			if v > 0 then
				c:stroke(ui.path():arc(w / 2, h / 2, r, -90, 360 * v), ink, 2, { cap = "round" })
			end
		end,
	}
end

-- The × that takes a chip off: a disc hung at the chip's top-right corner,
-- over its edge, laid over the chip (no room of its own), shown while
-- `lit` (the chip under the pointer, or the × with the keys).
local function cross(item: Item, lit: () -> boolean, on_remove: (id: string) -> (), on_lit: (boolean) -> (), name: (() -> string)?): Node
	-- (Its chip's name as it is now, when the chip's can change.)
	local function named(): string
		return locale.s("ds.composer_cargo.remove", { name = if name then name() else item.name })
	end
	return ui.button {
		absolute = true,
		top = -CROSS_OUT,
		right = -CROSS_OUT,
		w = CROSS,
		h = CROSS,
		p = 0,
		justify = "center",
		items = "center",
		rounded = "$radius.full",
		bg = "$zeron.composer.chip.cross",
		color = "$text.secondary",
		cursor = "pointer",
		hover = { bg = "$zeron.composer.chip.cross", color = "$text.primary" },
		active = { bg = "$zeron.composer.chip.cross", color = "$text.primary" },
		focus = control.ring,
		label = named,
		tooltip = named(),
		opacity = ui.tween(ui.computed(function(): number
			return if lit() then 1 else 0
		end), SWAP),
		on_focus = function()
			on_lit(true)
		end,
		on_blur = function()
			on_lit(false)
		end,
		on_click = function()
			on_remove(item.id)
		end,
		Icon { name = "close", size = 10 },
	}
end

-- A thumbnail's scrim, with what it holds over the picture.
local function scrim(...: Node): Node
	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		items = "center",
		justify = "center",
		bg = "$zeron.composer.chip.scrim",
		color = "$zeron.composer.chip.scrim_ink",
		pointer_events = "none",
		...,
	}
end

-- The markup the card shows under the pointer: its first lines (the
-- shipping app's ANNOTATION_SNIPPET_LINES), an ellipsis past them.
local CARD_LINES = 4
local CARD_W = 360

-- What the element is, under the pointer (the shipping app's annotation
-- card): its tag, a weight up, its id and classes muted, its size at the
-- end; under a rule, the first lines of its markup in the mono face.
local function pick_card(a: any): Node?
	if type(a) ~= "table" or type(a.element) ~= "string" then
		return nil
	end
	local cut = string.find(a.element, "[#.]")
	local tag = if cut then string.sub(a.element, 1, cut - 1) else a.element
	local rest = if cut then string.sub(a.element, cut) else ""
	local rect = if type(a.rect) == "table" then a.rect else {}
	local w, hgt = math.floor((rect[3] or 0) + 0.5), math.floor((rect[4] or 0) + 0.5)
	local lines: { string } = {}
	local more = false
	for line in string.gmatch(if type(a.html) == "string" then a.html else "", "[^\n]+") do
		if string.find(line, "%S") then
			if #lines >= CARD_LINES then
				more = true
				break
			end
			table.insert(lines, line)
		end
	end
	if more then
		table.insert(lines, "…")
	end
	local code: { any } = {}
	for _, line in lines do
		table.insert(code, FadeText { line, literal = true, w_full = true, font = "$font.mono", text_size = "$font.size.xs", line_height = "$font.line.xs", color = "$text.secondary" })
	end
	return HoverCard.Card {
		w = CARD_W,
		p = 0,
		gap = 0,
		ui.row {
			w_full = true,
			h = 30,
			px = "$space.3",
			gap = "$space.2",
			items = "center",
			ui.row {
				grow = 1,
				min_w = 0,
				items = "center",
				ui.text { tag, literal = true, shrink = 0, text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "$text.primary" },
				FadeText { rest, literal = true, grow = 1, text_size = "$font.size.sm", color = "$text.tertiary" },
			},
			if w > 0 or hgt > 0
				then ui.text { `{w} × {hgt}`, literal = true, shrink = 0, tabular = true, text_size = "$font.size.sm", color = "$text.tertiary" }
				else nil,
		},
		if #code > 0
			then ui.column {
				w_full = true,
				px = "$space.3",
				py = "$space.2",
				border_sides = { 1, 0, 0, 0 },
				border_color = "$border.subtle",
				table.unpack(code),
			}
			else nil,
	}
end

-- A review comment's card, as it hovers: who left it (a plugin's), its
-- note, and the code that goes with it, every line of what's sent (the
-- host holds it to 40 lines), so nothing reaches the agent unseen.
local function comment_card(c: any): Node?
	if type(c) ~= "table" or (type(c.snippet) ~= "string" and type(c.plugin) ~= "string") then
		return nil
	end
	local code: { any } = {}
	for line in string.gmatch(if type(c.snippet) == "string" then c.snippet .. "\n" else "", "([^\n]*)\n") do
		table.insert(code, FadeText { line, literal = true, w_full = true, font = "$font.mono", text_size = "$font.size.xs", line_height = "$font.line.xs", color = "$text.secondary" })
	end
	return HoverCard.Card {
		w = CARD_W,
		p = 0,
		gap = 0,
		ui.column {
			w_full = true,
			px = "$space.3",
			py = "$space.2",
			gap = "$zeron.space.hair",
			if type(c.plugin) == "string"
				then ui.text { locale.t("ds.composer_cargo.comment_from", { plugin = c.plugin }), text_size = "$font.size.xs", color = "$text.tertiary" }
				else nil,
			ui.text { c.name or "", literal = true, text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "$text.primary" },
			if type(c.body) == "string" and c.body ~= ""
				then ui.text { c.body, literal = true, wrap = true, text_size = "$font.size.sm", color = "$text.primary" }
				else nil,
		},
		if #code > 0
			then ui.column {
				w_full = true,
				px = "$space.3",
				py = "$space.2",
				border_sides = { 1, 0, 0, 0 },
				border_color = "$border.subtle",
				table.unpack(code),
			}
			else nil,
	}
end

-- A page's address as a chip names it: no scheme, no "www.", no lone
-- trailing slash.
local function page_of(url: any): string?
	if type(url) ~= "string" or url == "" then
		return nil
	end
	local s = string.gsub(url, "^%a[%w+.-]*://", "")
	s = string.gsub(s, "^www%.", "")
	if string.sub(s, -1) == "/" and not string.find(string.sub(s, 1, -2), "/", 1, true) then
		s = string.sub(s, 1, -2)
	end
	return s
end

local function disc(item: Item, lit: () -> boolean, on_remove: (id: string) -> (), on_lit: (boolean) -> ()): Node
	local named = locale.s("ds.composer_cargo.remove", { name = item.name })
	return ui.button {
		absolute = true,
		top = -DISC_OUT,
		right = -DISC_OUT,
		w = DISC,
		h = DISC,
		p = 0,
		justify = "center",
		items = "center",
		rounded = "$radius.full",
		bg = "$background",
		shadow = "$shadow.sm",
		color = "$text.secondary",
		cursor = "pointer",
		hover = { bg = "$background", color = "$text.primary" },
		active = { bg = "$background", color = "$text.primary" },
		focus = control.ring,
		label = named,
		tooltip = named,
		opacity = function(): number
			return if lit() then 1 else 0
		end,
		on_focus = function()
			on_lit(true)
		end,
		on_blur = function()
			on_lit(false)
		end,
		on_click = function()
			on_remove(item.id)
		end,
		Icon { name = "close-circle", size = 14 },
	}
end

local function image_tile(item: Item, row: State<Item>, props: Props): Node
	local hovered = ui.use_state(false)
	local focused = ui.use_state(false)
	local function lit(): boolean
		return hovered:get() or focused:get()
	end
	local function uploading(): boolean
		local p = row:get().progress
		return p ~= nil and p < 1 and row:get().failed == nil
	end
	local function failed(): boolean
		return row:get().failed ~= nil
	end
	local remove = props.on_remove
	local lifted = if props.source then props.source(item) else nil
	return ui.overlay {
		w = IMAGE_TILE,
		h = IMAGE_TILE,
		shrink = 0,
		relative = true,
		rounded = TILE_ROUND,
		label = item.name,
		-- (Opening: with the keys too, and the lightbox's thumbnail.)
		focusable = props.on_open ~= nil,
		focus = control.ring,
		on_mount = if lifted then lifted.on_mount else nil,
		on_visible = if lifted then lifted.on_visible else nil,
		opacity = if lifted then lifted.opacity else nil,
		tooltip = if item.meta then `{item.name} · {item.meta}` else item.name,
		cursor = if props.on_open then "pointer" else nil,
		on_click = if props.on_open
			then function()
				(props.on_open :: (string) -> ())(item.id)
			end
			else nil,
		on_hover = function(inside: boolean)
			hovered:set(inside)
			over(item, inside)
		end,
		ui.overlay {
			w_full = true,
			h_full = true,
			rounded = TILE_ROUND,
			overflow = "hidden",
			border = 1,
			border_color = "$zeron.composer.chip.edge",
			border_over = true,
			-- (Its file gone since, or not a picture: the gallery's glyph in
			-- its place, `ui.image`'s `fallback`.)
			ui.image {
				src = item.src,
				fit = "cover",
				w_full = true,
				h_full = true,
				fallback = scrim(ui.div { color = "$text.tertiary", Icon { name = "gallery", size = 16 } }),
			},
			ui.show(uploading, scrim(ring(function(): number
				return row:get().progress or 0
			end, "$zeron.composer.chip.scrim_ink", "#ffffff4d"))),
			ui.show(failed, scrim(ui.div { color = "$zeron.tone.danger.ink", Icon { name = "danger-triangle", size = 16 } })),
		},
		if remove
			then disc(item, lit, remove, function(on: boolean)
				focused:set(on)
			end)
			else nil,
	}
end

local function file_tile(item: Item, props: Props): Node
	local hovered = ui.use_state(false)
	local focused = ui.use_state(false)
	local function lit(): boolean
		return hovered:get() or focused:get()
	end
	local remove = props.on_remove
	return ui.overlay {
		w = FILE_TILE,
		h = FILE_TILE,
		shrink = 0,
		relative = true,
		rounded = "$radius.lg",
		label = item.name,
		tooltip = if item.meta then `{item.name} · {item.meta}` else item.name,
		on_hover = function(inside: boolean)
			hovered:set(inside)
			over(item, inside)
		end,
		ui.column {
			w_full = true,
			h_full = true,
			rounded = "$radius.lg",
			border = 1,
			border_color = "$zeron.composer.chip.edge",
			px = "$space.1",
			gap = ui.raw(3, "Zeron's file tile"),
			items = "center",
			justify = "center",
			FileIcon { name = item.name, size = 24 },
			FadeText { item.name, literal = true, max_w = FILE_TILE - 10, fade = 8, text_size = ui.raw(10, "Zeron's file tile"), line_height = ui.raw(12, "Zeron's file tile"), color = "$text.secondary" },
		},
		if remove
			then disc(item, lit, remove, function(on: boolean)
				focused:set(on)
			end)
			else nil,
	}
end

-- (Zeron paints these inline in the draft; zgx's text has no chips.)
local function pill_chip(item: Item, props: Props): Node
	local hovered = ui.use_state(false)
	local focused = ui.use_state(false)
	local function lit(): boolean
		return hovered:get() or focused:get()
	end
	local opens = if item.pasted then props.on_open_pasted else nil
	local tip = if item.pasted
		then locale.s("ds.composer_cargo.pasted_tip")
		else locale.s("ds.composer_cargo.linked_tip", { path = item.path or item.name })
	local remove = props.on_remove
	return ui.row {
		shrink = 0,
		relative = true,
		max_w = n("zeron.composer.chip.max", 214),
		label = item.label or item.name,
		on_hover = function(inside: boolean)
			hovered:set(inside)
			over(item, inside)
		end,
		ui.row {
			h = PILL_H,
			min_w = 0,
			pl = "$zeron.space.hair",
			pr = "$space.2",
			gap = "$zeron.space.tight",
			items = "center",
			rounded = ui.raw(5, "Zeron's chip"),
			bg = "$zeron.composer.chip.bg",
			tooltip = tip,
			tooltip_delay = 420,
			tooltip_placement = { side = "top", align = "start", offset = 1 },
			cursor = if opens then "pointer" else nil,
			on_click = if opens
				then function()
					opens(item.id)
				end
				else nil,
			ui.div {
				w = PILL_H - 4,
				h = PILL_H - 4,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = ui.raw(4, "Zeron's chip well"),
				bg = "$hover",
				FileIcon { name = item.name, size = 14 },
			},
			FadeText { item.label or item.name, literal = true, grow = 1, min_w = 0, text_size = "$font.size.md", line_height = "$font.line.md", color = "$text.primary" },
			if item.linked then ui.div { shrink = 0, color = "$text.tertiary", ui.svg { src = LINK, tint = "currentColor", w = 12, h = 12 } } else nil,
		},
		if remove
			then disc(item, lit, remove, function(on: boolean)
				focused:set(on)
			end)
			else nil,
	}
end

local function shot_app(item: Item): string
	local a = item.appshot
	local app = if a and type(a.app) == "string" then (string.match(a.app, "^%s*(.-)%s*$") or "") else ""
	return if app ~= "" then app else locale.s("ds.composer_cargo.captured_app")
end
local function provenance(app: string, title: string?): string
	local t = if title then (string.match(title, "^%s*(.-)%s*$") or "") else ""
	return if t ~= "" and string.lower(t) ~= string.lower(app) then `{t} / {app}` else app
end

local function appshot_tile(item: Item, props: Props): Node
	local hovered = ui.use_state(false)
	local over_x = ui.use_state(false)
	local app = shot_app(item)
	local source = provenance(app, item.appshot and item.appshot.title)
	local lifted = if props.source then props.source(item) else nil
	local remove = props.on_remove
	local named = locale.s("ds.composer_cargo.remove_appshot", { app = app })
	return ui.overlay {
		w = SHOT_W,
		h = SHOT_H,
		shrink = 0,
		relative = true,
		rounded = TILE_ROUND,
		bg = "$surface.raised",
		label = locale.t("ds.composer_cargo.preview_appshot", { app = app }),
		tooltip = source,
		focusable = props.on_open ~= nil,
		focus = control.ring,
		cursor = if props.on_open then "pointer" else nil,
		on_mount = if lifted then lifted.on_mount else nil,
		on_visible = if lifted then lifted.on_visible else nil,
		opacity = if lifted then lifted.opacity else nil,
		on_click = if props.on_open
			then function()
				(props.on_open :: (string) -> ())(item.id)
			end
			else nil,
		on_hover = function(inside: boolean)
			hovered:set(inside)
			over(item, inside)
		end,
		ui.image {
			src = item.src,
			absolute = true,
			left = 4,
			top = 4,
			w = SHOT_W - 8,
			h = SHOT_H - 8,
			rounded = ui.raw(6, "Zeron's Appshot picture"),
			fit = "contain",
		},
		ui.column {
			absolute = true,
			left = 0,
			right = 0,
			bottom = 0,
			h = 68,
			justify = "end",
			pointer_events = "none",
			rounded_bl = ui.raw(11, "inside the tile's edge"),
			rounded_br = ui.raw(11, "inside the tile's edge"),
			gradient = { stops = { "#00000000", "#0000008c", "#000000e6" } },
			ui.row {
				w_full = true,
				min_w = 0,
				px = "$zeron.space.snug",
				pb = "$space.2",
				gap = "$space.2",
				items = "center",
				if item.icon
					then ui.image { src = item.icon, w = 20, h = 20, shrink = 0, rounded = ui.raw(5, "Zeron's Appshot source"), fit = "contain", aria_hidden = true }
					else ui.div {
						w = 20,
						h = 20,
						shrink = 0,
						items = "center",
						justify = "center",
						rounded = ui.raw(5, "Zeron's Appshot source"),
						bg = ui.raw("#ffffff26", "on the scrim, in both themes"),
						color = ui.raw("#ffffffd9", "on the scrim, in both themes"),
						Icon { name = "monitor", size = 12 },
					},
				FadeText { source, literal = true, grow = 1, min_w = 0, text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$zeron.composer.chip.scrim_ink" },
			},
		},
		-- (The edge's own layer: a token's alpha can't ease, its opacity can.)
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = TILE_ROUND,
			border = 1,
			border_color = "$border",
			pointer_events = "none",
			opacity = ui.tween(ui.computed(function(): number
				return if hovered:get() then 1 else 0.55
			end), { duration = 150 }),
		},
		if remove
			then ui.button {
				absolute = true,
				top = 4,
				right = 4,
				w = 14,
				h = 14,
				p = 0,
				justify = "center",
				items = "center",
				rounded = "$radius.full",
				bg = "$text.primary",
				shadow = "$shadow.sm",
				color = "$background",
				cursor = "pointer",
				hover = { bg = "$text.primary", color = "$background" },
				active = { bg = "$text.primary", color = "$background" },
				focus = control.ring,
				label = named,
				tooltip = named,
				on_hover = function(inside: boolean)
					over_x:set(inside)
				end,
				opacity = ui.tween(ui.computed(function(): number
					if over_x:get() then
						return 1
					end
					return if hovered:get() then 1 else 0.7
				end), { duration = 150 }),
				on_click = function()
					remove(item.id)
				end,
				Icon { name = "close", size = 10 },
			}
			else nil,
	}
end

local function first_line(body: any): string
	return if type(body) == "string" then (string.match(body, "^%s*([^\n]*)") or "") else ""
end

local function comments_badge(comments: State<{ Item }>, props: Props): Node
	local open = ui.use_state(false)
	local function count_label(): string
		local count = #comments:get()
		return if count == 1 then locale.s("ds.composer_cargo.comment_one") else locale.s("ds.composer_cargo.comment_many", { n = count })
	end
	local trigger = ui.button {
		h = BADGE_H,
		px = "$space.2",
		gap = "$zeron.space.tight",
		shrink = 0,
		items = "center",
		rounded = "$radius.lg",
		bg = "$hover",
		color = "$text.secondary",
		cursor = "pointer",
		text_size = "$font.size.sm",
		weight = "$font.weight.medium",
		hover = { bg = "$pressed" },
		active = { bg = "$pressed" },
		focus = control.ring,
		expanded = open,
		label = function(): string
			return locale.s("ds.composer_cargo.comments_show", { label = count_label() })
		end,
		on_click = function()
			open:set(not open:peek())
		end,
		ui.div { opacity = 0.7, Icon { name = "chat-round-line", size = 12 } },
		ui.text { count_label, literal = true },
	}
	ui.effect(function()
		if #comments:get() == 0 and open:peek() then
			open:set(false)
		end
	end)
	return ui.popover(Menu.panel {
		open = open,
		anchor = trigger,
		placement = { side = "top", align = "start", offset = 6 },
		w = 340,
		build = function(): Node
			return ui.column {
				w_full = true,
				gap = 0,
				ui.column {
					w_full = true,
					p = "$space.1",
					gap = MENU_GAP,
					ui.each(comments, function(c: Item): Node
						-- (Its row follows it: a comment carried to another line.)
						local function now(): any
							for _, it in comments:get() do
								if it.id == c.id then
									return it
								end
							end
							return c
						end
						local function text(): string
							return first_line(now().body)
						end
						return ui.row(Menu.look {
							gap = "$space.2",
							tooltip = comment_card(c),
							tooltip_placement = { side = "right", align = "start", offset = 8 },
							cursor = "default",
							-- (Where Zeron's severity dot stands.)
							ui.div { w = 6, h = 6, shrink = 0 },
							FadeText { text, literal = true, grow = 1, min_w = 0, text_size = "$font.size.md", color = "$text.secondary" },
							ui.text { function(): string
								return now().name
							end, literal = true, shrink = 0, font = "$font.mono", text_size = ui.raw(10.5, "Zeron's staged comment location"), color = "$text.tertiary" },
							ui.button {
								w = 18,
								h = 18,
								p = 0,
								shrink = 0,
								justify = "center",
								items = "center",
								rounded = ui.raw(4, "Zeron's staged comment row"),
								bg = "#00000000",
								color = "$text.secondary",
								cursor = "pointer",
								hover = { bg = "$hover", color = "$text.primary" },
								active = { bg = "$pressed", color = "$text.primary" },
								focus = control.ring,
								label = function(): string
									return locale.s("ds.composer_cargo.comment_remove", { text = text() })
								end,
								on_click = function()
									if props.on_remove then
										props.on_remove(c.id)
									end
								end,
								Icon { name = "close-circle", size = 12 },
							},
						})
					end, {
						key = function(c: Item): string
							return c.id
						end,
						gap = MENU_GAP,
					}),
				},
				Menu.Rule(),
				ui.column {
					w_full = true,
					p = "$space.1",
					ui.button(Menu.look {
						bg = "#00000000",
						color = "$text.secondary",
						label = locale.t("ds.composer_cargo.comments_clear_named"),
						on_click = function()
							ui.batch(function()
								for _, c in comments:peek() do
									if props.on_remove then
										props.on_remove(c.id)
									end
								end
								open:set(false)
							end)
						end,
						ui.text { locale.t("ds.composer_cargo.comments_clear"), text_size = "$font.size.md" },
					}),
				},
			}
		end,
	})
end

-- A chip's pencil: the annotation opened again where it was drawn
-- (screens/chat/shared's `editing`, which the transcript or preview takes).
local editing = ui.shared("chat.annotation.edit", nil :: any)
local NOTE_W, NOTE_H, NOTE_CARD_W, NOTE_CARD_H = 130, 32, 385, 420

local function note_action(label: Label, icon: string, act: () -> ()): Node
	return ui.button {
		label = label,
		w = 20,
		h = 20,
		p = 0,
		shrink = 0,
		items = "center",
		justify = "center",
		rounded = ui.raw(4, "Zeron's annotation card action"),
		bg = "#00000000",
		color = "$text.tertiary",
		cursor = "pointer",
		hover = { bg = "$hover", color = "$text.primary" },
		active = { bg = "$pressed" },
		focus = control.ring,
		on_click = act,
		Icon { name = icon :: any, size = 16 },
	}
end

-- One staged quote in the card: its number, where it was selected, its
-- words, its comment, and its pencil (only where a badge can take the edit)
-- and trash.
local function note_row(item: Item, n: () -> number, notes: boolean, props: Props): Node
	local faint = "$text.tertiary"
	local size, line = ui.raw(13, "Zeron's annotation card text"), ui.raw(18.5, "Zeron's annotation card line")
	local head = if item.source
		then locale.t("ds.composer_cargo.selected_from", { path = item.source })
		else locale.t("ds.composer_cargo.selected")
	local comment = if notes and type(item.comment) == "string" and item.comment ~= "" then item.comment else nil
	return ui.row {
		w_full = true,
		items = "start",
		ui.text { function(): string
			return `{n()}.`
		end, literal = true, w = ui.raw(17.5, "Zeron's annotation number"), shrink = 0, text_size = size, line_height = line, color = faint },
		ui.column {
			grow = 1,
			min_w = 0,
			gap = 1,
			ui.row {
				w_full = true,
				gap = "$space.2",
				justify = "between",
				items = "center",
				ui.text { head, literal = true, min_w = 0, truncate = true, text_size = size, line_height = line, color = faint },
				ui.row {
					shrink = 0,
					gap = ui.raw(4.5, "Zeron's annotation card actions"),
					if notes and item.anchor
						then note_action(function(): string
							return locale.s("ds.composer_cargo.annotation_edit", { n = n() })
						end, "pen", function()
							editing:set { id = item.id }
						end)
						else nil,
					note_action(function(): string
						return if notes then locale.s("ds.composer_cargo.annotation_remove", { n = n() }) else locale.s("ds.composer_cargo.selection_remove", { n = n() })
					end, "trash-bin-minimalistic", function()
						if props.on_remove then
							props.on_remove(item.id)
						end
					end),
				},
			},
			ui.text { item.text or item.quote or "", literal = true, wrap = true, pr = ui.raw(59.5, "Zeron's annotation body inset"), text_size = size, line_height = line, color = "$text.primary" },
			if comment then ui.text { locale.t("ds.composer_cargo.user_comment"), mt = ui.raw(5, "Zeron's comment label"), text_size = size, line_height = line, color = faint } else nil,
			if comment then ui.text { comment, literal = true, wrap = true, pr = ui.raw(59.5, "Zeron's annotation body inset"), text_size = size, line_height = line, color = "$text.primary" } else nil,
		},
	}
end

-- Zeron's one chip for all a draft's annotations ("2 annotations"), or for
-- its plain selections: its × takes them all (shown under the pointer), and
-- its card, which the pointer can move into, lists each.
local function quotes_chip(list: State<{ Item }>, notes: boolean, props: Props): Node
	local hovered = ui.use_state(false)
	-- (Its card goes once an annotation is being edited, until the pointer comes back, as gpui's.)
	local put_away = ui.use_state(false)
	ui.effect(function()
		if notes and editing:get() ~= nil then
			put_away:set(true)
		end
	end)
	local function label(): string
		local count = #list:get()
		return if notes then locale.s("ds.composer_cargo.annotations", { n = count }) else locale.s("ds.composer_cargo.selections", { n = count })
	end
	local function number_of(item: Item): () -> number
		return function(): number
			for i, it in list:get() do
				if it.id == item.id then
					return i
				end
			end
			return 0
		end
	end
	local rows = ui.ref()
	local card = HoverCard.Card {
		id = if notes then "composer-annotations-card" else "composer-selections-card",
		w = NOTE_CARD_W,
		max_h = NOTE_CARD_H,
		p = 0,
		ui.scroll {
			w_full = true,
			h = function(): number
				return math.min(rows.height or 0, NOTE_CARD_H)
			end,
			ui.column {
				ref = rows,
				w_full = true,
				pt = ui.raw(13, "Zeron's annotation card"),
				pb = ui.raw(13, "Zeron's annotation card"),
				pl = ui.raw(14, "Zeron's annotation card"),
				pr = ui.raw(14, "Zeron's annotation card"),
				ui.each(list, function(item: Item): Node
					return note_row(item, number_of(item), notes, props)
				end, {
					key = function(item: Item): string
						return item.id
					end,
					gap = "$space.3",
				}),
			},
		},
	}
	local function chip(with_card: boolean): Node
		return ui.row {
			id = if notes then "composer-annotations" else "composer-selections",
			role = "group",
			label = label,
			h = NOTE_H,
			min_w = NOTE_W,
			shrink = 0,
			gap = ui.raw(5, "Zeron's annotation chip"),
			pl = ui.raw(10, "Zeron's annotation chip"),
			pr = "$space.1",
			items = "center",
			rounded = ui.raw(12, "Zeron's annotation chip"),
			border = 1,
			border_color = "$border",
			color = "$text.primary",
			text_size = ui.raw(13, "Zeron's annotation chip"),
			tooltip = if with_card then card else nil,
			tooltip_interactive = true,
			tooltip_delay = 0,
			tooltip_placement = { side = "top", align = "start", offset = 4.5 },
			on_hover = function(inside: boolean)
				hovered:set(inside)
				if inside then
					put_away:set(false)
				end
			end,
			ui.div { color = "$text.secondary", Icon { name = if notes then "chat-round-line" else "wrap-text", size = 12 } },
			ui.text { label, literal = true, grow = 1 },
			ui.button {
				id = if notes then "composer-annotations-remove" else "composer-selections-remove",
				label = if notes then locale.t("ds.composer_cargo.annotations_remove") else locale.t("ds.composer_cargo.selections_remove"),
				w = 19,
				h = 19,
				p = 0,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.full",
				border = 1,
				border_color = "$background",
				bg = "$text.primary",
				color = "$background",
				cursor = "pointer",
				hover = { bg = "$text.primary" },
				focus = control.ring,
				opacity = ui.tween(ui.computed(function(): number
					return if hovered:get() then 1 else 0
				end), SWAP),
				on_click = function()
					ui.batch(function()
						for _, it in list:peek() do
							if props.on_remove then
								props.on_remove(it.id)
							end
						end
					end)
				end,
				Icon { name = "close", size = 10 },
			},
		}
	end
	return ui.switch(function(): string
		return if put_away:get() then "away" else "card"
	end, { card = chip(true), away = chip(false) })
end

local function row_chip(item: Item, row: State<Item>, props: Props): Node
	local hovered = ui.use_state(false)
	local focused = ui.use_state(false)
	local removing = false
	local function lit(): boolean
		return hovered:get() or focused:get()
	end
	local h = n("zeron.composer.chip.h", 40)
	local glyph_size = n("zeron.composer.chip.glyph", 18)
	local function uploading(): boolean
		local p = row:get().progress
		return p ~= nil and p < 1 and row:get().failed == nil
	end
	local function failed(): boolean
		return row:get().failed ~= nil
	end
	local glyph: Node
	if item.kind == "file" then
		glyph = FileIcon { name = item.name, size = glyph_size }
	elseif item.kind == "annotation" then
		glyph = Icon { name = "cursor", size = glyph_size - 2 }
	elseif item.kind == "chat" then
		-- Another chat mentioned: its agent's mark, as the sidebar's row.
		glyph = if item.mark then Mark { name = item.mark, size = glyph_size - 2, bare = true } else Icon { name = "side-chat", size = glyph_size - 2 }
	else
		glyph = Icon { name = "wrap-text", size = glyph_size - 2 }
	end
	local remove = props.on_remove
	local on_remove = if remove
		then function(id: string)
			removing = true
			over(item, false)
			remove(id)
		end
		else nil
	-- An element picked on a page (the browser panel's): its number and
	-- page over its note or words, its card under the pointer, no click.
	local pick = item.kind == "annotation" and item.element ~= nil
	local opens = props.on_open ~= nil and not pick
	-- The slot: the glyph, or the ring, or the warning.
	local slot = ui.div {
		w = SLOT,
		h = SLOT,
		shrink = 0,
		items = "center",
		justify = "center",
		color = "$text.secondary",
		ui.show(uploading, ring(function(): number
			return row:get().progress or 0
		end, "$text.primary", "$border.strong"), ui.show(failed, ui.div { color = "$zeron.tone.danger.ink", Icon { name = "danger-triangle", size = 16 } }, glyph)),
	}
	local function meta(): string
		local now = row:get()
		if pick then
			return if now.note and now.note ~= "" then now.note else (now.name or "")
		end
		if now.failed then
			return now.failed
		end
		if now.progress and now.progress < 1 then
			return locale.s("ds.composer_cargo.uploading", { n = math.floor(now.progress * 100 + 0.5) })
		end
		return now.meta or ""
	end
	local card: Node? = if pick then pick_card(item.annotation) else nil
	local function title(): string
		local now = row:get()
		if pick then
			return page_of(now.annotation and now.annotation.url) or now.page or now.name or ""
		end
		return now.name
	end
	-- (A passage from the conversation has no card: its chip's line, who
	-- said it over the words, is all it shows.)
	local cut = hollow(lit)
	-- The chip (the cut layer) and the × beside it, uncut, in a box that
	-- holds both and knows the pointer over either.
	return ui.row {
		-- (In points: `max_w` doesn't take a token.)
		max_w = n("zeron.composer.chip.max", 214),
		shrink = 0,
		relative = true,
		label = function(): string
			if pick then
				return locale.s("ds.composer_cargo.annotation", { n = row:get().index or 0, note = `{title()}: {meta()}` })
			end
			return row:get().name
		end,
		on_hover = function(inside: boolean)
			if removing then
				return
			end
			hovered:set(inside)
			over(item, inside)
		end,
		ui.row {
			ref = cut.ref,
			layer_shader = cut.layer_shader,
			layer_params = cut.layer_params,
			h = h,
			min_w = 0,
			-- (Held to the chip's widest too: a pick's two lines, each wider,
			-- fade at its edge rather than push it wider.)
			max_w = n("zeron.composer.chip.max", 214),
			pl = "$zeron.space.snug",
			pr = "$space.2",
			gap = "$space.2",
			items = "center",
			rounded = "$zeron.composer.chip.radius",
			bg = "$zeron.composer.chip.bg",
			tooltip = card,
			tooltip_placement = if card then { side = "top", align = "start", offset = 8 } else nil,
			cursor = if opens then "pointer" else nil,
			on_click = if opens
				then function()
					(props.on_open :: (string) -> ())(item.id)
				end
				else nil,
			slot,
			ui.column {
				min_w = 0,
				grow = 1,
				gap = 0,
				ui.row {
					-- (Held to the column's width, so the page fades at the
					-- chip's edge rather than pushing it wider.)
					w_full = true,
					min_w = 0,
					gap = "$space.1",
					items = "center",
					-- A pick's number, as its mark on the page wears it.
					if pick
						then ui.text {
							function(): string
								return tostring(row:get().index or 0)
							end,
							literal = true,
							shrink = 0,
							tabular = true,
							text_size = "$font.size.md",
							line_height = "$font.line.md",
							weight = "$font.weight.semibold",
							color = "$zeron.tone.accent.ink",
						}
						else nil,
					FadeText {
						-- (Its row's: a comment carried to another line says so; a
						-- pick's page.)
						title,
						grow = 1,
						literal = true,
						text_size = "$font.size.md",
						line_height = "$font.line.md",
						weight = "$font.weight.medium",
						color = "$text.primary",
					},
				},
				-- (Only with something to say: an empty line would hold the
				-- name off the chip's middle.)
				ui.show(
					function(): boolean
						return meta() ~= ""
					end,
					FadeText {
						meta,
						literal = true,
						text_size = "$font.size.xs",
						line_height = "$font.line.xs",
						color = function()
							return if failed() then "$zeron.tone.danger.ink" else "$text.tertiary"
						end,
					}
				),
			},
			-- Failed: the way back, in the chip.
			ui.show(
				failed,
				ui.button {
					label = locale.t("ds.composer_cargo.retry_named", { name = item.name }),
					h = 24,
					px = "$space.2",
					shrink = 0,
					rounded = ui.raw(control.nest("$zeron.composer.chip.radius", "$space.2"), "concentric with the chip: its corner less the button's inset"),
					bg = "#00000000",
					color = "$text.primary",
					text_size = "$font.size.sm",
					weight = "$font.weight.medium",
					cursor = "pointer",
					hover = { bg = "$hover" },
					active = { bg = "$pressed" },
					focus = control.ring,
					on_click = function()
						if props.on_retry then
							props.on_retry(item.id)
						end
					end,
					locale.t("ds.composer_cargo.retry"),
				}
			),
		},
		if on_remove
			then cross(item, lit, on_remove, function(on: boolean)
				if removing then
					return
				end
				focused:set(on)
			end, function(): string
				return row:get().name
			end)
			else nil,
	}
end

local function is_pill(item: Item): boolean
	return item.kind == "file" and (item.pasted == true or item.linked == true)
end

-- A file mid-upload or failed keeps the row that can say so.
local function is_tile(item: Item): boolean
	return item.kind == "image" or (item.kind == "file" and not is_pill(item) and item.progress == nil and item.failed == nil)
end

-- A chip for one thing carried.
function M.Chip(item: Item, row: State<Item>, props: Props): Node
	if item.kind == "image" then
		return image_tile(item, row, props)
	elseif is_pill(item) then
		return pill_chip(item, props)
	elseif is_tile(item) then
		return file_tile(item, props)
	end
	return row_chip(item, row, props)
end

local BADGE, FOLD, NOTES, PLAIN = "\0comments", "\0fold", "\0notes", "\0plain"

local function cell_size(item: Item): (number, number)
	if item.id == BADGE then
		return CELL_W, BADGE_H
	elseif item.id == NOTES or item.id == PLAIN then
		return NOTE_W, NOTE_H
	elseif item.kind == "image" then
		return IMAGE_TILE, IMAGE_TILE
	elseif is_pill(item) then
		return CELL_W, PILL_H
	elseif is_tile(item) then
		return FILE_TILE, FILE_TILE
	end
	return CELL_W, n("zeron.composer.chip.h", 40)
end

-- (Zeron's `strip_rows` with a limit, so the fold needs no layout.)
local function fitting(cells: { Item }, wide: number, tall: number): number?
	local used, row_h, total = 0, 0, PAD_TOP
	local gap = n("zeron.composer.chip.gap", 8)
	for i, cell in cells do
		local w, h = cell_size(cell)
		if i > 1 and used + gap + w > wide then
			total += row_h + gap
			used, row_h = w, h
		else
			used = if i == 1 then w else used + gap + w
			row_h = math.max(row_h, h)
		end
		if total + row_h > tall then
			return math.max(i - 1, 1)
		end
	end
	return nil
end

local function window_h(): number?
	local window = (ui :: any).window
	local state = if type(window) == "table" or type(window) == "userdata" then window.size else nil
	local ok, size = pcall(function(): any
		return if state then state:get() else nil
	end)
	local h = if ok and type(size) == "table" then size.height else nil
	return if type(h) == "number" and h > 1 then h else nil
end

M.Strip = ui.component(function(props: Props): Node
	local all = props.items
	local opened = ui.use_state(false)
	local width = ui.use_state(0)
	local shots = ui.computed(function(): { Item }
		local out: { Item } = {}
		for _, it in all:get() do
			if it.kind == "appshot" then
				table.insert(out, it)
			end
		end
		return out
	end)
	local comments = ui.computed(function(): { Item }
		local out: { Item } = {}
		for _, it in all:get() do
			if it.kind == "comment" then
				table.insert(out, it)
			end
		end
		return out
	end)
	local function quotes_of(notes: boolean): State<{ Item }>
		return ui.computed(function(): { Item }
			local out: { Item } = {}
			for _, it in all:get() do
				if it.kind == "quote" and (it.comment ~= nil) == notes then
					table.insert(out, it)
				end
			end
			return out
		end)
	end
	local annotations, selections = quotes_of(true), quotes_of(false)
	local cells = ui.computed(function(): { Item }
		local tiles: { Item } = {}
		local rows: { Item } = {}
		local badge = false
		local notes, plain = false, false
		for _, it in all:get() do
			if it.kind == "comment" then
				badge = true
			elseif it.kind == "pick" then
				continue
			elseif it.kind == "quote" then
				notes = notes or it.comment ~= nil
				plain = plain or it.comment == nil
			elseif it.kind ~= "appshot" then
				table.insert(if is_tile(it) then tiles else rows, it)
			end
		end
		for _, it in rows do
			table.insert(tiles, it)
		end
		if notes then
			table.insert(tiles, { id = NOTES, kind = "quote", name = NOTES } :: Item)
		end
		if plain then
			table.insert(tiles, { id = PLAIN, kind = "quote", name = PLAIN } :: Item)
		end
		if badge then
			local cell: Item = { id = BADGE, kind = "comment", name = BADGE }
			table.insert(tiles, cell)
		end
		return tiles
	end)
	local fit = ui.computed(function(): number?
		local wide, high = width:get(), window_h()
		if wide <= 0 or not high then
			return nil
		end
		return fitting(cells:get(), wide - 2 * n("zeron.composer.chip.inset", 16), math.max(high - WINDOW_KEEPS, IMAGE_TILE + PAD_TOP))
	end)
	local shown = ui.computed(function(): { Item }
		local list = cells:get()
		local count = fit:get()
		if not count then
			return list
		end
		local out: { Item } = {}
		for i, it in list do
			if opened:get() or i <= count then
				table.insert(out, it)
			end
		end
		local fold: Item = { id = FOLD, kind = "text", name = FOLD }
		table.insert(out, fold)
		return out
	end)
	local function hidden(): number
		return math.max(0, #cells:get() - (fit:get() or 0))
	end
	ui.effect(function()
		if not fit:get() then
			opened:set(false)
		end
	end)
	local h = n("zeron.composer.chip.h", 40)
	-- A chip follows its item (its progress climbing, its failure): zgx's
	-- animated rows get no state of their own, so each finds its item in
	-- the list by its id.
	local function follow(item: Item): State<Item>
		return ui.computed(function(): Item
			for _, it in all:get() do
				if it.id == item.id then
					return it
				end
			end
			return item
		end)
	end
	return ui.column {
		w_full = true,
		on_resize = function(size: Size)
			if math.abs(width:peek() - size.w) >= 0.5 then
				width:set(size.w)
			end
		end,
		ui.show(
			function(): boolean
				return #shots:get() > 0
			end,
			ui.scroll_x {
				w_full = true,
				h = PAD_TOP + SHOT_H,
				px = "$zeron.composer.chip.inset",
				pt = "$space.3",
				ui.each(shots, function(item: Item): Node
					return appshot_tile(item, props)
				end, {
					key = function(item: Item): string
						return item.id
					end,
					direction = "row",
					gap = "$zeron.composer.chip.gap",
					animate = { duration = 220, ease = "outCubic" },
					enter = if motion.reduced_now()
						then { opacity = { 0, 1 }, duration = 80 }
						else { opacity = { 0, 1 }, y = { 8, 0 }, duration = 240, ease = "outExpo" },
					exit = { opacity = 0, duration = 140, ease = "outCubic" },
				}),
			}
		),
		ui.show(
			function(): boolean
				return #cells:get() > 0
			end,
			ui.row {
				w_full = true,
				flex_wrap = true,
				items = "end",
				gap = "$zeron.composer.chip.gap",
				px = "$zeron.composer.chip.inset",
				pt = "$space.3",
				ui.each(shown, function(item: Item): Node
					if item.id == BADGE then
						return comments_badge(comments, props)
					elseif item.id == NOTES then
						return quotes_chip(annotations, true, props)
					elseif item.id == PLAIN then
						return quotes_chip(selections, false, props)
					elseif item.id == FOLD then
						return ui.button {
							label = function()
								return if opened:get()
									then locale.s("ds.composer_cargo.show_fewer")
									else locale.s("ds.composer_cargo.show_more", { n = hidden() })
							end,
							expanded = opened,
							h = h,
							px = "$space.2",
							shrink = 0,
							rounded = "$zeron.composer.chip.radius",
							bg = "#00000000",
							color = "$text.tertiary",
							text_size = "$font.size.sm",
							weight = "$font.weight.medium",
							cursor = "pointer",
							hover = { bg = "$hover", color = "$text.primary" },
							active = { bg = "$pressed", color = "$text.primary" },
							focus = control.ring,
							on_click = function()
								opened:set(not opened:peek())
							end,
							ui.text {
								function()
									return if opened:get() then locale.s("ds.composer_cargo.show_less") else locale.s("ds.composer_cargo.more", { n = hidden() })
								end,
								tabular = true,
							},
						}
					end
					return M.Chip(item, follow(item), props)
				end, {
					key = function(item: Item): string
						return item.id
					end,
					direction = "row",
					flex_wrap = true,
					items = "end",
					gap = "$zeron.composer.chip.gap",
					animate = { duration = 220, ease = "outCubic" },
					enter = { opacity = { 0, 1 }, scale = { 0.9, 1 }, duration = 220, ease = "outCubic" },
					exit = { opacity = 0, scale = 0.9, duration = 140, ease = "outCubic" },
				}),
			}
		),
	}
end)

return M
