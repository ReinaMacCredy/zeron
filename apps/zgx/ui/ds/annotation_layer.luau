-- Text annotations over a scrolled text (a transcript, a rendered file), as
-- Zeron's annotation layer draws them (crates/ui/src/annotations.rs), offered
-- from upstream zgx's selection pill (ds/transcript's Action): a selection
-- settled shows a glass pill just under its end, "Mention", "Add to chat" and
-- "Ask in side chat", fading in with a small rise; a scroll takes it away
-- and it fades where it stood. Mention quotes the text into the draft at its
-- caret; Add to chat stages an annotation and opens a comment input over the
-- first selected line, focused; a numbered badge then stands over the
-- annotated line at the column's right edge, its comment in a tip under the
-- pointer. The text is washed only while its input is open (the code's
-- choice, not the doc's "stays highlighted").
--
-- An anchor is the holder the selection began in (a transcript's message
-- row), the first selected line's place in it, and the holder's size then: a
-- holder laid out at another width scales the place with its height.
--
-- Each holder draws its badges and their tips, so they scroll with it and go
-- when it isn't laid out; the pill and the input stand over the whole view,
-- the input placed again a frame after it scrolls.
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
local glass = require("@self/glass")
local control = require("@self/control")
local motion = require("@self/motion")
local HtmlPicks = require("@self/html_picks")

ui.theme.tokens {
	zeron = {
		annotation = {
			-- Opaque, as Zeron's `surface_fill` (ink 8% flattened on the page).
			fill = "over(alpha({text.primary}, 0.08), over({background}, {zeron.window}))",
			wash = { dark = "alpha({accent}, 0.5)", light = "alpha({accent}, 0.28)" },
		},
	},
}

local M = {}

-- The accent, deep enough for the white number on it in every preset (gpui's `deep_accent`).
local function badge_fill(): any
	ui.theme.current:get()
	local accent = ui.theme.get("accent")
	if type(accent) ~= "string" or #accent < 7 then
		return "$accent"
	end
	return ui.raw(HtmlPicks.capped(string.sub(accent, 1, 7), 0.55, 0.56), "the theme's accent, deepened")
end

export type Anchor = { key: string, dy: number, x: number, w: number, lh: number, hw: number?, hh: number? }
export type Note = { n: number, id: string, text: string, comment: string, source: string?, anchor: Anchor? }
export type Notes = {
	list: () -> { Note },
	add: (text: string, anchor: Anchor?) -> string?,
	comment: (id: string, text: string, focus: boolean) -> (),
	remove: (id: string) -> (),
	side: ((text: string) -> ())?,
	mention: ((q: { text: string, who: string? }) -> ())?,
	-- Typing with one of the pill's words holding the keys: the composer takes them (type to focus).
	keys: (() -> ())?,
	edit: () -> string?,
	edited: () -> (),
}

-- Upstream's selection pill: its words' height and its own (the bar's
-- even inset and its border, each side), what it keeps from the selection.
local WORD, PILL_H, CLEAR = 26, 32, 6
local EDITOR_W, EDITOR_H, EDIT_R, EDITOR_GAP, ACTION = 296, 45, 17, 7, 27
local BADGE, TAIL, OVER_LINE, STEP_GAP = 30, 4, 28.5, 2
local TIP_H, TIP_R, TIP_GAP, TIP_MAX, TIP_CHARS = 32, 11, 11.5, 280, 36
local EDGE, TEXT, LINE, MAX_LINES = 8, 13, 20, 6
local WATCH = 200

-- The layers made, each its pill's keys; the last settled is the one shown.
local layers: { [any]: (e: any) -> any } = {}
local settled = 0

-- A key the window's root heard with nothing holding the keyboard (the
-- selection's press leaves focus parked): the pill's Escape and Tab while
-- one shows. The pill never takes the keys itself, so typing, a paste, and
-- a split's focus go on as with no pill.
function M.key(e: any): boolean
	local best: any, at = nil, -1
	for _, key in layers do
		local n = key(nil)
		if n and n > at then
			best, at = key, n
		end
	end
	return best ~= nil and best(e) == true
end

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
	-- The room the pill leaves at the view's foot (what floats there).
	floor: (() -> number)?,
	-- Who said a selection, for Mention's line.
	who: ((s: any) -> string?)?,
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
	-- changes size (once laid out there), while the input shows. (The pill
	-- stays where it was put, and a scroll takes it away.)
	local tick = ui.state(`annotation_tick.{opts.name}`, 0)
	local function anchor_shown(): Anchor?
		local e = editor:get()
		if e then
			local n = note_of(e.id)
			return if n then n.anchor else nil
		end
		return nil
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

	-- The pill put away. (A word holding the keys lets go of them first:
	-- removed, its focus would fall to the next control, a composer's button
	-- or another split pane's composer, which then takes the split's focus.)
	local bar: Ref? = nil
	local words: { Ref } = {}
	local holding = false
	-- The pill whose offered selection ended while a word of it held the keys: it goes once they are let go.
	local ended: number? = nil
	local function away()
		local el = if bar then bar:peek() else nil
		if el then
			el:blur()
		end
		pending:set(nil)
	end

	-- A scroll takes the pill away: it no longer stands by what it offers.
	-- (The offset is read only while it shows: no work a scroll frame otherwise.)
	ui.effect(function()
		local p = pending:get()
		if p and opts.scroll and math.abs(opts.scroll() - p.offset) > 1 then
			away()
		end
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
		away()
		ui.batch(function()
			draft:set(comment)
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

	-- What ends an offered selection that isn't the host's (a page's), and the pill it was offered with.
	local offered_end: (() -> ())?, offered_gen = nil, 0
	-- The selection ended: the host's, and an offered one's own.
	local function unselect(p: any)
		SelectionMenu.clear()
		if offered_end and offered_gen == p.gen then
			offered_end()
		end
		offered_end = nil
	end
	local function add()
		local p = pending:peek()
		if not p or not notes then
			return
		end
		away()
		unselect(p)
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
		away()
		unselect(p)
		notes.side(p.text)
	end
	local function mention()
		local p = pending:peek()
		if not p or not notes or not notes.mention then
			return
		end
		away()
		unselect(p)
		notes.mention({ text = p.text, who = p.who })
	end

	-- Where the pill stands for a selection, in the view: under its last
	-- line, ending with it; else over its first, starting with it; else at
	-- the view's foot (upstream's `place`).
	local function place_of(s: any): any
		local el = opts.view:peek()
		local b, t = s.bottom or s.top, s.top or s.bottom
		if not el or el.x == nil or el.y == nil or not el.height or not b or not t then
			return nil
		end
		local floor = el.height - (if opts.floor then opts.floor() else EDGE) - PILL_H
		local below = b.y + b.h + CLEAR - el.y
		if below <= floor then
			return { y = below, right = b.x + b.w - el.x }
		end
		local above = t.y - CLEAR - PILL_H - el.y
		if above >= (if opts.top_inset then opts.top_inset() else 0) + EDGE then
			return { y = above, left = t.x - el.x }
		end
		return { y = floor, right = b.x + b.w - el.x }
	end
	local function pending_of(s: any, text_of: ((s: any) -> string)?): any
		local a = anchor_of(s)
		if not a then
			return nil
		end
		settled += 1
		return {
			seq = settled,
			text = if text_of then text_of(s) else s.text,
			who = if opts.who then opts.who(s) else nil,
			anchor = a,
			at = place_of(s),
			offset = if opts.scroll then opts.scroll() else 0,
			-- (Motion as the setting is when it shows.)
			still = motion.reduced_now(),
			gen = (function(): number
				made += 1
				return made
			end)(),
		}
	end

	-- A press let go: the selection now, its text as `text_of` reads it.
	function L.settle(text_of: ((s: any) -> string)?)
		pinned:set(nil)
		local s = if notes then SelectionMenu.read() else nil
		local p = if s then pending_of(s, text_of) else nil
		if not p or not p.at then
			pending:set(nil)
			return
		end
		pending:set(p)
	end
	function L.offer(s: any)
		pinned:set(nil)
		local p = if notes then pending_of(s) else nil
		if p then
			p.offered = true
			offered_end, offered_gen, ended = s.clear, p.gen, nil
		end
		pending:set(if p and p.at then p else nil)
	end
	-- An offered selection ended where it was made. (A press on one of the pill's words takes the keys from a
	-- page, and its selection with them: the pill stays while the word holds them, for the click to land.)
	function L.clear()
		local p = pending:peek()
		if holding and p and p.offered then
			ended = p.gen
			return
		end
		away()
	end
	-- The window's key, while this layer's pill shows (`M.key`; with no key,
	-- when it settled).
	local function key(e: any): any
		local p = pending:peek()
		if not p or editor:peek() then
			return nil
		end
		-- (A word holding the keys has them first, and Tab goes on from it.)
		if holding then
			return nil
		end
		if e == nil then
			return p.seq
		end
		if e.key == "Escape" and not (e.shift or e.cmd or e.alt or e.ctrl or e.meta) then
			away()
			return true
		end
		-- (Into its words: the first, Shift: the last.)
		if e.key == "Tab" and not (e.cmd or e.alt or e.ctrl or e.meta) then
			for i = if e.shift then #words else 1, if e.shift then 1 else #words, if e.shift then -1 else 1 do
				local el = words[i]:peek()
				if el and el:focus() then
					return true
				end
			end
		end
		return false
	end
	layers[key] = key
	ui.on_cleanup(function()
		layers[key] = nil
	end)
	function L.press()
		if pinned:peek() then
			pinned:set(nil)
		end
	end
	function L.shown(): boolean
		return pending:get() ~= nil or editor:get() ~= nil
	end
	-- What the selection's menu offers, the pill's own words.
	function L.offers(s: any, text_of: ((s: any) -> string)?): { any }
		if not notes then
			return {}
		end
		local function take(act: () -> ())
			return function()
				local p = pending_of(s, text_of)
				if p then
					pending:set(p)
					act()
				end
			end
		end
		pending:set(nil)
		local out: { any } = {}
		if notes.mention then
			table.insert(out, { label = locale.s("ds.annotation.mention"), icon = "chat-round-line", on_select = take(mention) })
		end
		table.insert(out, { label = locale.s("ds.annotation.add_to_chat"), icon = "chat-round-line", on_select = take(add) })
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

	-- A word on the pill: a pill in a pill, its curve parallel to the bar's.
	local function word(id: string, label: Label, act: () -> (), ref: Ref): Node
		return ui.button {
			id = id,
			ref = ref,
			label = label,
			h = WORD,
			px = "$zeron.space.snug",
			rounded = "$radius.full",
			bg = "#00000000",
			color = "$text.primary",
			text_size = "$font.size.sm",
			weight = "$font.weight.medium",
			cursor = "pointer",
			hover = { bg = "$hover" },
			active = { bg = "$pressed" },
			focus = control.ring,
			on_focus = function()
				holding = true
			end,
			on_blur = function()
				holding = false
				if ended then
					-- (A turn on: Tab's next word takes the keys first.)
					ui.after(0, function()
						local p = pending:peek()
						if ended and not holding then
							if p and p.gen == ended then
								away()
							end
							ended = nil
						end
					end)
				end
			end,
			on_click = act,
			label,
		}
	end

	local Pill = ui.component(function(props: { gen: number }): Node
		local pill = ui.ref()
		local mine = { ui.ref(), ui.ref(), ui.ref() }
		-- (The pill shown last: where the keys go, and what lets go of them.)
		bar, words, holding = pill, {}, false
		if notes and notes.mention then
			table.insert(words, mine[1])
		end
		table.insert(words, mine[2])
		if notes and notes.side then
			table.insert(words, mine[3])
		end
		-- However the selection went (a press elsewhere, its row rebuilt), the pill goes with it.
		ui.every(WATCH, function()
			if pending:peek() and pending:peek().offered then
				return false
			end
			if SelectionMenu.read() == nil then
				if pending:peek() and pending:peek().gen == props.gen then
					away()
				end
				return false
			end
			return true
		end)
		-- Where it stands: its selection's place, so it fades out there.
		local stood: any = nil
		local function standing(): any
			local p = pending:get()
			stood = if p and p.gen == props.gen then p.at else stood
			return stood
		end
		local box: { [any]: any } = {
			id = "annotation-pill",
			ref = pill,
			absolute = true,
			top = function(): number
				local at = standing()
				return if at then at.y else -1000
			end,
			left = function(): number
				local at = standing()
				if not at then
					return 0
				end
				local w = pill.width or 0
				local x = if at.left then at.left else at.right - w
				return math.clamp(x, EDGE, math.max(EDGE, (opts.view.width or 0) - w - EDGE))
			end,
			gap = "$zeron.space.hair",
			items = "center",
			shadow = "$shadow.md",
			selectable = false,
			-- A key on a word Tab reached (Space and Enter press it).
			on_key = function(e: any): boolean
				if e.key == "Escape" and not (e.shift or e.cmd or e.alt or e.ctrl) then
					away()
					return true
				end
				-- (A letter or a paste: the composer takes the keys, and the
				-- window then gives it the text or the paste, as type to focus.
				-- Space, " " here, presses the word.)
				local paste = e.key == "v" and (e.cmd or e.ctrl) and not (e.shift or e.alt)
				local letter = utf8.len(e.key) == 1 and e.key ~= " " and not (e.cmd or e.ctrl or e.meta)
				if notes and notes.keys and not e["repeat"] and (paste or letter) then
					notes.keys()
				end
				return false
			end,
			if notes and notes.mention then word("annotation-mention", locale.t("ds.annotation.mention"), mention, mine[1]) else nil,
			word("annotation-add-to-chat", locale.t("ds.annotation.add_to_chat"), add, mine[2]),
			if notes and notes.side then word("annotation-ask-side-chat", locale.t("ds.annotation.ask_side"), side, mine[3]) else nil,
		}
		control.contain(glass.style(box, "menu"), { radius = "$radius.full", inset = "$zeron.space.hair", border = 1 })
		return ui.row(box)
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
			-- The window sent to the back blurs the field too: the comment stays open for the way back.
			if ui.window.active:peek() == false then
				return
			end
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
		local function comment(): string
			local n = note_of(props.note)
			return if n then n.comment else ""
		end
		local wide = (utf8.len(comment()) or 0) > TIP_CHARS
		-- (Placed by its laid-out size, which a wrapped comment only has once laid out.)
		local size = ui.use_state(nil :: { w: number, h: number }?)
		local function place(): (number, number)
			local n = note_of(props.note)
			local a = n and n.anchor
			if not a then
				return 0, -1000
			end
			local ref = opts.holder(a.key)
			local ox = offset_in_view(ref)
			local badge_left = (if ref then ref.width or 0 else 0) - BADGE - step_of(a.key, props.note) * (BADGE + STEP_GAP)
			local sz = size:get()
			local w = if sz and sz.w > 0 then sz.w else (if wide then TIP_MAX else 120)
			local h = if sz and sz.h > 0 then sz.h else TIP_H
			local vw = opts.view.width or 0
			local left = math.clamp(badge_left + BADGE / 2 - w / 2, EDGE - ox, math.max(EDGE - ox, vw - w - EDGE - ox))
			return left, dy_now(a, ref) - OVER_LINE - TIP_GAP - h
		end
		return ui.row {
			id = "annotation-tip",
			on_resize = function(sz: any)
				size:set({ w = sz.w, h = sz.h })
			end,
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
			bg = badge_fill,
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

	-- Over the view (above every row): the pill, and the comment's input.
	function L.Overlay(): Node?
		if not notes then
			return nil
		end
		local function over(): { [string]: any }
			return { absolute = true, left = 0, right = 0, top = 0, bottom = 0, overflow = "hidden", pointer_events = "none" }
		end
		-- Upstream's: a fade and a small rise in (no rise with motion reduced), a fade out where it stood.
		local function pill(reduced: boolean): Node
			local box = over()
			box.enter = if reduced
				then { opacity = { 0, 1 }, duration = 160, ease = "outQuad" }
				else { opacity = { 0, 1 }, y = { 4, 0 }, duration = 160, ease = "outQuad" }
			box.exit = { opacity = 0, duration = 90 }
			return ui.presence(function(): string?
				-- (Each selection its own: one let go again fades where it stood as the next comes in.)
				local p = pending:get()
				return if p and not editor:get() and p.still == reduced then `pill:{p.gen}` else nil
			end, function(which: string): Node
				return Pill { gen = tonumber(string.sub(which, 6)) :: number }
			end, box)
		end
		local editor_box = over()
		editor_box.exit = { opacity = 0, duration = 80 }
		return ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			pointer_events = "none",
			pill(false),
			pill(true),
			ui.presence(function(): string?
				local e = editor:get()
				return if e then `{e.gen}:{if e.editing then "edit" else "new"}` else nil
			end, function(which: string): Node
				return Editor { editing = string.find(which, "edit", 1, true) ~= nil }
			end, editor_box),
		}
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
