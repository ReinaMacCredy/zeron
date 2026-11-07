-- Text annotations over a scrolled text (a transcript, a rendered file), as
-- Zeron's annotation layer draws them (crates/ui/src/annotations.rs): a
-- selection settled offers a pill, "Add to chat" and "Ask in side chat",
-- over its first line; Add to chat stages an annotation and opens a comment
-- input there, focused; a numbered badge then stands over the annotated line
-- at the column's right edge, its comment in a tip under the pointer. The
-- text is washed only while its input is open (the code's choice, not the
-- doc's "stays highlighted").
--
-- An anchor is the holder the selection began in (a transcript's message
-- row), the first selected line's place in it, and the holder's size then: a
-- holder laid out at another width scales the place with its height.
--
-- Each holder draws its badges and their tips, so they scroll with it and go
-- when it isn't laid out; the pill and the input stand over the whole view,
-- placed again a frame after it scrolls.
--
--   local layer = Layer.new { name = "...", notes = notes, view = list_ref,
--   	holder_at = function(y) return key, ref end, holder = function(key) return ref end }
--   layer.settle(text_of)  -- a press let go: the selection's pill
--   layer.Overlay()        -- over the view: the pill, or the input
--   layer.Here(key)        -- in the holder (relative): its badges and tips
--   layer.wash(key)        -- the text to wash in the holder, or nil

local Icon = require("@self/icon")
local SelectionMenu = require("@self/selection_menu")
local locale = require("@self/locale")

ui.theme.tokens {
	zeron = {
		annotation = {
			-- Opaque, as Zeron's `surface_fill` (ink 8% flattened on the page).
			fill = "over(alpha({text.primary}, 0.08), {background})",
			badge = "{accent}",
			wash = { dark = "alpha({accent}, 0.5)", light = "alpha({accent}, 0.28)" },
		},
	},
}

local M = {}

export type Anchor = { key: string, dy: number, x: number, w: number, lh: number, hw: number?, hh: number? }
export type Note = { n: number, id: string, text: string, comment: string, source: string?, anchor: Anchor? }
export type Notes = {
	list: () -> { Note },
	add: (text: string, anchor: Anchor?) -> string?,
	comment: (id: string, text: string, focus: boolean) -> (),
	remove: (id: string) -> (),
	side: ((text: string) -> ())?,
	edit: () -> string?,
	edited: () -> (),
}

local PILL_H, PILL_R, SEG_PAD, PILL_GAP = 30, 14, 8, 8
local EDITOR_W, EDITOR_H, EDIT_R, EDITOR_GAP, ACTION = 296, 45, 17, 7, 27
local BADGE, TAIL, OVER_LINE, STEP_GAP = 30, 4, 28.5, 2
local TIP_H, TIP_R, TIP_GAP, TIP_MAX, TIP_CHARS = 32, 11, 11.5, 280, 36
local EDGE, TEXT, LINE, MAX_LINES = 8, 13, 20, 6
local WATCH = 200

function M.wash_color(): string
	return if string.find(ui.theme.kind(), "dark", 1, true) then "$zeron.annotation.wash.dark" else "$zeron.annotation.wash.light"
end

function M.new(opts: {
	name: string,
	notes: Notes?,
	view: Ref,
	holder_at: (y: number) -> (string?, Ref?),
	holder: (key: string) -> Ref?,
	top_inset: (() -> number)?,
	scroll: (() -> number)?,
	reveal: ((a: Anchor, over: number) -> ())?,
}): { [string]: any }
	local L = {}
	local notes = opts.notes
	local pending = ui.state(`annotation_pending.{opts.name}`, nil :: any)
	local editor = ui.state(`annotation_editor.{opts.name}`, nil :: any)
	local draft = ui.state(`annotation_draft.{opts.name}`, "")
	local hovered = ui.state(`annotation_hovered.{opts.name}`, nil :: string?)
	local pinned = ui.state(`annotation_pinned.{opts.name}`, nil :: string?)
	local made = 0

	local function list(): { Note }
		return if notes then notes.list() else {}
	end
	local function note_of(id: string): Note?
		for _, n in list() do
			if n.id == id then
				return n
			end
		end
		return nil
	end

	-- The first selected line in the holder it began in.
	local function anchor_of(s: any): Anchor?
		local t = s.top or s.bottom
		if not t then
			return nil
		end
		local key, ref = opts.holder_at(t.y + t.h / 2)
		local el = if ref then ref:peek() else nil
		if not key or not el or el.x == nil or el.y == nil then
			return nil
		end
		return { key = key, dy = t.y - el.y, x = t.x - el.x, w = t.w, lh = t.h, hw = el.width, hh = el.height }
	end

	local function dy_now(a: Anchor, ref: Ref?): number
		local w, h = if ref then ref.width else nil, if ref then ref.height else nil
		if a.hw and a.hh and a.hh > 0 and w and h and math.abs(w - a.hw) >= 0.5 then
			return a.dy * (h / a.hh)
		end
		return a.dy
	end

	-- The holder's place in the view (followed as it scrolls), x and y.
	local function offset_in_view(ref: Ref?): (number, number)
		if opts.scroll then
			opts.scroll()
		end
		local el, view = if ref then ref:get() else nil, opts.view:get()
		if not el or not view or el.x == nil or view.x == nil then
			return 0, 0
		end
		return el.x - view.x, el.y - view.y
	end

	-- In the holder: over the anchor's line, centred on it; under it with no
	-- room above in the view; kept inside the view's sides.
	-- Placed again a frame after the view scrolls or the anchor's holder
	-- changes size (once laid out there), while the pill or the input shows.
	local tick = ui.state(`annotation_tick.{opts.name}`, 0)
	local function anchor_shown(): Anchor?
		local e = editor:get()
		if e then
			local n = note_of(e.id)
			return if n then n.anchor else nil
		end
		local p = pending:get()
		return if p then p.anchor else nil
	end
	ui.effect(function()
		local a = anchor_shown()
		if not a then
			return
		end
		if opts.scroll then
			opts.scroll()
		end
		local _ = opts.view.width, opts.view.height
		local ref = opts.holder(a.key)
		if ref then
			local _ = ref.width, ref.height
		end
		ui.after(0, function()
			tick:set(tick:peek() + 1)
		end)
	end)

	-- In the view: over the anchor's line, centred on it; under it with no
	-- room above; kept inside the view's sides. Off the view while its
	-- holder isn't laid out.
	local function over_line(a: Anchor, w: number, h: number, gap: number): (number, number)
		tick:get()
		local ref = opts.holder(a.key)
		local el, view = if ref then ref:peek() else nil, opts.view:peek()
		if not el or not view or el.x == nil or view.x == nil then
			return 0, -10000
		end
		local ox, oy = el.x - view.x, el.y - view.y
		local y = oy + dy_now(a, ref)
		local vw = view.width or 0
		local left = math.clamp(ox + a.x + a.w / 2 - w / 2, EDGE, math.max(EDGE, vw - w - EDGE))
		local above = y - gap - h
		local top = (if opts.top_inset then opts.top_inset() else 0) + EDGE
		return left, if above < top then y + a.lh + gap else above
	end

	-- Each badge's step left from the column's edge: one on another's line moves over (Zeron's 32).
	local function step_of(key: string, id: string): number
		local placed: { { step: number, dy: number } } = {}
		for _, n in list() do
			local a = n.anchor
			if a and a.key == key then
				local step = 0
				local moved = true
				while moved do
					moved = false
					for _, p in placed do
						if p.step == step and math.abs(p.dy - a.dy) < BADGE / 2 then
							step += 1
							moved = true
						end
					end
				end
				if n.id == id then
					return step
				end
				table.insert(placed, { step = step, dy = a.dy })
			end
		end
		return 0
	end

	local function open_editor(id: string, editing: boolean, comment: string)
		local was = editor:peek()
		if was and notes then
			editor:set(nil)
			notes.comment(was.id, draft:peek(), false)
		end
		made += 1
		ui.batch(function()
			draft:set(comment)
			pending:set(nil)
			pinned:set(nil)
			hovered:set(nil)
			editor:set { id = id, editing = editing, gen = made }
		end)
	end
	local function commit(focus: boolean)
		local e = editor:peek()
		if not e or not notes then
			return
		end
		editor:set(nil)
		notes.comment(e.id, draft:peek(), focus)
	end
	local function cancel()
		local e = editor:peek()
		if not e or not notes then
			return
		end
		editor:set(nil)
		if e.editing then
			pinned:set(e.id)
		else
			notes.remove(e.id)
		end
	end
	local function delete()
		local e = editor:peek()
		if not e or not notes then
			return
		end
		editor:set(nil)
		notes.remove(e.id)
	end
	L.commit = commit

	local function add()
		local p = pending:peek()
		if not p or not notes then
			return
		end
		pending:set(nil)
		SelectionMenu.clear()
		local id = notes.add(p.text, p.anchor)
		if id then
			open_editor(id, false, "")
		end
	end
	local function side()
		local p = pending:peek()
		if not p or not notes or not notes.side then
			return
		end
		pending:set(nil)
		SelectionMenu.clear()
		notes.side(p.text)
	end

	-- A press let go: the selection now, its text as `text_of` reads it.
	function L.settle(text_of: ((s: any) -> string)?)
		pinned:set(nil)
		local s = if notes then SelectionMenu.read() else nil
		local a = if s then anchor_of(s) else nil
		if not s or not a then
			pending:set(nil)
			return
		end
		pending:set { text = if text_of then text_of(s) else s.text, anchor = a }
	end
	function L.clear()
		pending:set(nil)
	end
	function L.press()
		if pinned:peek() then
			pinned:set(nil)
		end
	end
	function L.shown(): boolean
		return pending:get() ~= nil or editor:get() ~= nil
	end
	-- What the selection's menu offers, the pill's own two.
	function L.offers(s: any, text_of: ((s: any) -> string)?): { any }
		if not notes then
			return {}
		end
		local function take(act: () -> ())
			return function()
				local a = anchor_of(s)
				if a then
					pending:set { text = if text_of then text_of(s) else s.text, anchor = a }
					act()
				end
			end
		end
		pending:set(nil)
		local out: { any } = { { label = locale.s("ds.annotation.add_to_chat"), icon = "chat-round-line", on_select = take(add) } }
		if notes.side then
			table.insert(out, { label = locale.s("ds.annotation.ask_side"), icon = "chat-round-line", on_select = take(side) })
		end
		return out
	end
	-- The text washed in holder `key`: the annotation whose input is open there.
	function L.wash(key: string): string?
		local e = editor:get()
		if not e then
			return nil
		end
		local n = note_of(e.id)
		return if n and n.anchor and n.anchor.key == key then n.text else nil
	end

	if notes then
		local ns = notes :: Notes
		-- Its annotation gone (the chip's ×, a send): the input goes with it.
		ui.effect(function()
			local e = editor:get()
			if e and not note_of(e.id) then
				editor:set(nil)
			end
		end)
		-- A chip's pencil: this surface's annotation opened again, its line brought into view.
		ui.effect(function()
			local id = ns.edit()
			if not id then
				return
			end
			ui.after(0, function()
				local n = note_of(id)
				if not n or not n.anchor then
					return
				end
				ns.edited()
				open_editor(n.id, true, n.comment)
				if opts.reveal then
					opts.reveal(n.anchor, EDITOR_H + EDITOR_GAP + 40)
				end
			end)
		end)
		-- (A chat switched away from, the page made again: the comment kept.)
		ui.on_cleanup(function()
			commit(false)
		end)
	end

	local function segment(id: string, label: Label, act: () -> ()): Node
		return ui.button {
			id = id,
			label = label,
			h_full = true,
			px = ui.raw(SEG_PAD, "Zeron's pill segment"),
			items = "center",
			bg = "#00000000",
			rounded = 0,
			color = "$text.primary",
			text_size = ui.raw(TEXT, "Zeron's annotation text"),
			cursor = "pointer",
			hover = { bg = "$hover" },
			active = { bg = "$pressed" },
			on_click = act,
			label,
		}
	end

	local Pill = ui.component(function(): Node
		local pill = ui.ref()
		ui.every(WATCH, function()
			if SelectionMenu.read() == nil then
				pending:set(nil)
				return false
			end
			return true
		end)
		local stood: any = nil
		local function place(): (number, number)
			local p = pending:get() or stood
			stood = p
			if not p then
				return 0, -1000
			end
			return over_line(p.anchor, pill.width or 200, PILL_H, PILL_GAP)
		end
		return ui.row {
			id = "annotation-pill",
			ref = pill,
			absolute = true,
			left = function(): number
				local x = place()
				return x
			end,
			top = function(): number
				local _, y = place()
				return y
			end,
			h = PILL_H,
			items = "center",
			rounded = ui.raw(PILL_R, "Zeron's annotation pill"),
			border = 1,
			border_color = "$border",
			bg = "$zeron.annotation.fill",
			overflow = "hidden",
			shadow = "$shadow.md",
			selectable = false,
			segment("annotation-add-to-chat", locale.t("ds.annotation.add_to_chat"), add),
			if notes and notes.side then ui.div { w = 1, h_full = true, bg = "$border" } else nil,
			if notes and notes.side then segment("annotation-ask-side-chat", locale.t("ds.annotation.ask_side"), side) else nil,
		}
	end)

	-- A press on the comment card's own buttons (its field keeps the comment open).
	local pressing = false
	local function round_action(id: string, label: Label, icon: string, solid: boolean, act: () -> ()): Node
		return ui.button {
			id = id,
			label = label,
			w = ACTION,
			h = ACTION,
			p = 0,
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = "$radius.full",
			bg = if solid then "$text.primary" else "#00000000",
			color = if solid then "$background" else "$text.secondary",
			cursor = "pointer",
			hover = { bg = if solid then "$text.primary" else "$hover" },
			focusable = false,
			on_press = function()
				pressing = true
			end,
			on_click = act,
			Icon { name = icon :: any, size = if solid then 14 else 16 },
		}
	end
	local function word_action(id: string, label: Label, solid: boolean, act: () -> ()): Node
		return ui.button {
			id = id,
			label = label,
			h = ACTION,
			px = ui.raw(if solid then 8.5 else 8, "Zeron's annotation buttons"),
			shrink = 0,
			items = "center",
			rounded = "$radius.full",
			border = if solid then 0 else 1,
			border_color = "$border",
			bg = if solid then "$text.primary" else "#00000000",
			color = if solid then "$background" else "$text.primary",
			text_size = ui.raw(TEXT, "Zeron's annotation text"),
			cursor = "pointer",
			hover = { bg = if solid then "$text.primary" else "$hover" },
			focusable = false,
			on_press = function()
				pressing = true
			end,
			on_click = act,
			label,
		}
	end

	local Editor = ui.component(function(props: { editing: boolean }): Node
		local box = ui.ref()
		local measure = ui.ref()
		local input = ui.ref()
		local function height(): number
			return math.clamp(measure.height or LINE, LINE, LINE * MAX_LINES)
		end
		-- A press on the card's buttons moves the keys off the field; only one outside it commits.
		local function left()
			ui.after(0, function()
				if pressing then
					pressing = false
					local el = input:peek()
					if el then
						el:focus()
					end
					return
				end
				commit(false)
			end)
		end
		local field = ui.textarea {
			id = "annotation-comment",
			value = draft,
			label = locale.t("ds.annotation.comment"),
			placeholder = locale.t("ds.annotation.placeholder"),
			placeholder_color = "$zeron.field.placeholder",
			caret_color = "$accent",
			wrap = true,
			w_full = true,
			min_w = 0,
			h_full = true,
			px = 0,
			py = 0,
			bg = "#00000000",
			border = 0,
			rounded = 0,
			focus = { border = 0, outline = 0 },
			color = "$text.primary",
			text_size = ui.raw(TEXT, "Zeron's annotation text"),
			line_height = ui.raw(LINE, "Zeron's annotation line"),
			ref = input,
			on_mount = function(el: any)
				el:focus()
			end,
			on_blur = left,
			on_key = function(e: any): boolean
				if e.key == "Escape" then
					cancel()
					return true
				end
				if e.key == "Enter" and not e.shift and not e.alt then
					commit(true)
					return true
				end
				return false
			end,
		}
		local sizer = ui.text {
			ref = measure,
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			opacity = 0,
			pointer_events = "none",
			aria_hidden = true,
			wrap = true,
			text_size = ui.raw(TEXT, "Zeron's annotation text"),
			line_height = ui.raw(LINE, "Zeron's annotation line"),
			function(): string
				return draft:get() .. "\u{200B}"
			end,
		}
		local well = ui.div { grow = 1, min_w = 0, relative = true, h = ui.computed(height), sizer, field }
		local card: { [any]: any }
		if props.editing then
			card = {
				w = EDITOR_W,
				pt = ui.raw(13, "Zeron's edit card"),
				pb = ui.raw(11, "Zeron's edit card"),
				pl = ui.raw(17, "Zeron's edit card"),
				pr = ui.raw(13, "Zeron's edit card"),
				gap = ui.raw(12, "Zeron's edit card"),
				rounded = ui.raw(EDIT_R, "Zeron's edit card"),
				well,
				ui.row {
					w_full = true,
					items = "center",
					ml = ui.raw(-7.5, "the trash's glyph under the text's start"),
					round_action("annotation-delete", locale.t("ds.annotation.delete"), "trash-bin-minimalistic", false, delete),
					ui.div { grow = 1 },
					word_action("annotation-cancel", locale.t("common.cancel"), false, cancel),
					ui.div { w = 6 },
					word_action("annotation-save", locale.t("common.save"), true, function()
						commit(true)
					end),
				},
			}
		else
			card = {
				w = EDITOR_W,
				min_h = EDITOR_H,
				pl = ui.raw(15, "Zeron's comment pill"),
				pr = ui.raw(9, "Zeron's comment pill"),
				py = ui.raw((EDITOR_H - ACTION) / 2, "the check's inset"),
				gap = ui.raw(8.5, "Zeron's comment pill"),
				items = "center",
				rounded = ui.raw(EDITOR_H / 2, "Zeron's comment pill"),
				well,
				round_action("annotation-check", locale.t("ds.annotation.add"), "check", true, function()
					commit(true)
				end),
			}
		end
		card.id = "annotation-editor"
		card.ref = box
		card.bg = "$zeron.annotation.fill"
		card.shadow = "$shadow.md"
		card.selectable = false
		card.absolute = true
		card.opacity = ui.tween(ui.computed(function(): number
			return if box.height then 1 else 0
		end), { duration = 125, ease = "outCubic" })
		local function place(): (number, number)
			local e = editor:get()
			local n = if e then note_of(e.id) else nil
			if not n or not n.anchor then
				return 0, -1000
			end
			return over_line(n.anchor, EDITOR_W, box.height or EDITOR_H, EDITOR_GAP)
		end
		card.left = function(): number
			local x = place()
			return x
		end
		card.top = function(): number
			local _, y = place()
			return y
		end
		return if props.editing then ui.column(card) else ui.row(card)
	end)

	local Tip = ui.component(function(props: { note: string }): Node
		local box = ui.ref()
		local function comment(): string
			local n = note_of(props.note)
			return if n then n.comment else ""
		end
		local function place(): (number, number)
			local n = note_of(props.note)
			local a = n and n.anchor
			if not a then
				return 0, -1000
			end
			local ref = opts.holder(a.key)
			local ox = offset_in_view(ref)
			local badge_left = (if ref then ref.width or 0 else 0) - BADGE - step_of(a.key, props.note) * (BADGE + STEP_GAP)
			local w = box.width or 120
			local vw = opts.view.width or 0
			local left = math.clamp(badge_left + BADGE / 2 - w / 2, EDGE - ox, math.max(EDGE - ox, vw - w - EDGE - ox))
			return left, dy_now(a, ref) - OVER_LINE - TIP_GAP - (box.height or TIP_H)
		end
		local wide = (utf8.len(comment()) or 0) > TIP_CHARS
		return ui.row {
			id = "annotation-tip",
			ref = box,
			absolute = true,
			left = function(): number
				local x = place()
				return x
			end,
			top = function(): number
				local _, y = place()
				return y
			end,
			w = if wide then TIP_MAX else nil,
			min_h = TIP_H,
			items = "center",
			px = ui.raw(9, "Zeron's annotation tip"),
			py = ui.raw(6, "Zeron's annotation tip"),
			rounded = ui.raw(TIP_R, "Zeron's annotation tip"),
			border = 1,
			border_color = "$border",
			bg = "$zeron.annotation.fill",
			shadow = "$shadow.md",
			pointer_events = "none",
			selectable = false,
			ui.text {
				comment,
				literal = true,
				wrap = wide,
				text_size = ui.raw(TEXT, "Zeron's annotation text"),
				line_height = ui.raw(LINE, "Zeron's annotation line"),
				color = "$text.primary",
			},
		}
	end)

	local function badge(n: Note, key: string): Node
		local function top(): number
			local ref = opts.holder(key)
			local a = n.anchor :: Anchor
			return (if ref then dy_now(a, ref) else a.dy) - OVER_LINE
		end
		return ui.row {
			id = `annotation-badge-{n.n}`,
			role = "img",
			label = locale.t("ds.annotation.badge", { n = n.n }),
			absolute = true,
			top = top,
			right = function(): number
				return step_of(key, n.id) * (BADGE + STEP_GAP)
			end,
			w = BADGE,
			h = BADGE,
			items = "center",
			justify = "center",
			rounded_tl = ui.raw(BADGE / 2, "Zeron's badge, round but for its tail"),
			rounded_tr = ui.raw(BADGE / 2, "Zeron's badge, round but for its tail"),
			rounded_br = ui.raw(BADGE / 2, "Zeron's badge, round but for its tail"),
			rounded_bl = ui.raw(TAIL, "the badge's tail"),
			border = 1,
			border_color = ui.raw("#ffffff", "Zeron's badge edge, both themes"),
			bg = "$zeron.annotation.badge",
			selectable = false,
			on_hover = function(inside: boolean)
				if inside then
					hovered:set(n.id)
				elseif hovered:peek() == n.id then
					hovered:set(nil)
				end
			end,
			ui.text {
				tostring(n.n),
				text_size = ui.raw(TEXT, "Zeron's annotation text"),
				weight = "$font.weight.semibold",
				color = ui.raw("#ffffff", "on the badge, both themes"),
			},
		}
	end

	-- Over the view (above every row): the pill, or the comment's input.
	function L.Overlay(): Node?
		if not notes then
			return nil
		end
		return ui.presence(function(): string?
			local e = editor:get()
			if e then
				return `{e.gen}:{if e.editing then "edit" else "new"}`
			end
			return if pending:get() then "pill" else nil
		end, function(which: string): Node
			if which == "pill" then
				return Pill()
			end
			return Editor { editing = string.find(which, "edit", 1, true) ~= nil }
		end, {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			overflow = "hidden",
			pointer_events = "none",
			exit = { opacity = 0, duration = 80 },
		})
	end

	-- What stands on holder `key`'s text: its badges, and a badge's tip.
	function L.Here(key: string): Node?
		if not notes then
			return nil
		end
		local here = ui.computed(function(): { Note }
			local out: { Note } = {}
			for _, n in list() do
				if n.anchor and n.anchor.key == key then
					table.insert(out, n)
				end
			end
			return out
		end)
		return ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			h = 0,
			ui.each(here, function(n: Note): Node
				return badge(n, key)
			end, {
				key = function(n: Note): string
					return `{n.id}:{n.n}`
				end,
			}),
			ui.presence(function(): string?
				local e = editor:get()
				local id = hovered:get() or pinned:get()
				if not id or (e and e.id == id) then
					return nil
				end
				local n = note_of(id)
				return if n and n.comment ~= "" and n.anchor and n.anchor.key == key then id else nil
			end, function(id: string): Node
				return Tip { note = id }
			end, { absolute = true, left = 0, top = 0, w = 0, h = 0 }),
		}
	end

	return L
end

return M
