-- The deck: what waits on the agent, as the composer's own top rather than
-- trays behind it. The composer's frame (its glass, edge, corner and
-- shadow) holds the deck and the pill as one surface, a rule between them,
-- so it grows upward as there is more to say. It holds the queue (their own
-- words, sent next), what is running (now) and the plan (the whole).
--
-- With one of them there, the header reads as its title; with more, the
-- header is tabs, as ⌘K's scopes, the open one on a thumb that springs from
-- tab to tab. A tab opens its rows under the header, a click on the open one
-- (or the chevron) folds them. Nothing opens itself: folded, what waits is a
-- pill floating over the composer, a tap opening it.
--
--   Dock { queue = queued, todos = plan, open = tab, … }
--
-- The composer gives it the queue and the plan only (1B: the pill row above
-- it carries agents, shells and questions). Their rows are
-- ds/composer_dock's (queue.rs, todo_panel.rs), and with only those two the
-- folded pill reads as one line: "≡ 2 queued · Plan 3/5". What is running
-- and alerts are upstream's tabs, kept for its merges and its gallery: a
-- running task's row with Stop, Stop all in the header, subagents behind
-- "N finished" (`kind = "agent"`, opened with `on_open_task`).
-- A list longer than the deck's cap scrolls, its edges fading as there is
-- more past them.

local Icon = require("@self/icon")
local IconButtonModule = require("@self/icon_button")
local IconButton = IconButtonModule.IconButton
local Button = require("@self/button")
local FadeText = require("@self/fade_text")
local RollText = require("@self/roll_text")
local glass = require("@self/glass")
local motion = require("@self/motion")
local status = require("@self/status")
local material = require("@self/material")
local control = require("@self/control")
local locale = require("@self/locale")
local dock = require("@self/composer_dock")

local M = {}

export type Queued = dock.Queued
export type TodoState = dock.TodoState
export type Todo = dock.Todo
export type TaskKind = "terminal" | "agent" | "process"
export type Task = {
	id: string,
	kind: TaskKind,
	name: string,
	-- A word on it, after its name (":5173", "reading 14 files").
	detail: string?,
	-- When it started, in `os.time()` seconds.
	since: number?,
	-- Done: how it ended ("exited 0", a subagent's "done").
	ended: string?,
	-- A subagent that ended badly: its mark says so.
	failed: boolean?,
}
export type Tray = "alerts" | "todos" | "agents" | "tasks" | "queue"
-- Something to know about the chat (a context nearly full, a rate limit, a
-- server that failed to start): how serious, what it is, a word more, and
-- a thing to do about it (its button's label).
export type AlertTone = "info" | "warning" | "danger"
-- `spinner`: still being worked on (reconnecting), the working ring in its
-- glyph's place. `dismissable = false`: it stays until what it says is
-- over (not connected), no × of its own.
export type Alert = { id: string, tone: AlertTone, title: string, detail: string?, action: string?, spinner: boolean?, dismissable: boolean? }
export type Props = {
	queue: State<{ Queued }>?,
	-- Whether the agent takes a word mid-turn (a row's action is then Steer); nil: not known yet.
	can_steer: (boolean | () -> boolean?)?,
	on_queue_send: ((id: string) -> ())?,
	on_queue_steer_all: (() -> ())?,
	-- The pill holds nothing, so the primary modifier and Enter would act on the last row.
	composer_empty: (() -> boolean)?,
	-- Where a row's pictures read from (a chat on another device).
	pictures: any?,
	on_queue_edit: ((id: string) -> ())?,
	on_queue_remove: ((id: string) -> ())?,
	on_queue_reorder: ((from: number, to: number) -> ())?,
	-- A row being edited: saving it, and letting it go.
	on_queue_save: ((id: string) -> ())?,
	on_queue_cancel: ((id: string) -> ())?,
	todos: State<{ Todo }>?,
	-- The turn is live (the step in progress wears the working ring).
	live: (() -> boolean)?,
	on_dismiss_todos: (() -> ())?,
	tasks: State<{ Task }>?,
	on_stop_task: ((id: string) -> ())?,
	on_stop_all: (() -> ())?,
	-- A subagent's row opens it (a tab of the panel). Its row is a plain
	-- listing without this.
	on_open_task: ((id: string) -> ())?,
	-- Which tray is open; the dock keeps its own when not given.
	open: State<Tray?>?,
	-- Short of room: rows lose their thumbnails, actions become glyphs.
	narrow: (() -> boolean)?,
	-- What to know about the chat, each in its tone's colours; its action,
	-- and letting one go.
	alerts: State<{ Alert }>?,
	on_alert_action: ((id: string) -> ())?,
	on_dismiss_alert: ((id: string) -> ())?,
	-- Another pill floating over the composer's middle ("Scroll to end"):
	-- its width while it shows. The folded pill moves aside for it.
	aside: (() -> number?)?,
	-- The folded pill's width while it shows, nil while it doesn't (so the
	-- other pill can move aside for it in turn).
	on_pill_width: ((w: number?) -> ())?,
	-- The width it is centred in, and the room to keep clear at its right
	-- (the side chat's tab): the folded pill stops short of it.
	width: (() -> number)?,
	clear_right: (() -> number)?,
	-- Whether the other pill stands on the folded pill's line at all (false: it stands higher, and this one stays put).
	pairs: (() -> boolean)?,
	-- Where the folded pill's width is kept (a composer made again lays it out where it stood).
	pill_w: State<number>?,
}

-- The trays' geometry, in points, read as the dock is built (the theme's).
local function n(token: string, fallback: number): number
	local v = ui.theme.get(token)
	return if type(v) == "number" then v else fallback
end

-- How a tray folds and opens (the dropdown's fold).
local FOLD_MS = 240
local FOLD: TweenOptions = { duration = FOLD_MS, ease = "out(3)" }

-- What folds: the body, clipped to nothing while its tray is folded, eased
-- open to its content's height (never past the trays' cap: past it the
-- body scrolls), fading as it goes.
-- Each tray's body height as last laid out, by tray: a tray made again (a
-- reload) opens at it from its first frame rather than growing open again.
local SEEN = ui.state("composer_tray_heights", {} :: { [string]: number })

-- How deep a tray list's edges fade: each as far as there is more past it
-- that way, up to this, easing in with the scroll rather than popping on.
local FADE_DEPTH = 16

-- A scroll's edge fades (the sidebar's `edge_fades`, ds/sidebar): the top as
-- deep as it is scrolled, the bottom as deep as there is left to scroll,
-- each up to `depth`; none at either end. (`over`: how far the content runs
-- past its view.)
local function edge_fades(offset: State<number>, over: () -> number, depth: number): () -> { number }
	return function(): { number }
		local o = offset:get()
		return { math.min(o, depth), math.min(math.max(0, over() - o), depth) }
	end
end

local function folding(open: () -> boolean, cap: number, body: Node, key: string): Node
	local inner = ui.ref()
	local offset = ui.use_state(0)
	local function want(): number
		local h = inner.height or 0
		if h <= 0 then
			-- (Not yet laid out: as it was.)
			h = SEEN:get()[key] or 0
		end
		return math.min(h, cap)
	end
	-- Folded, the body leaves the layout once the fold has played (a scroll
	-- of no height would be a warning); opening, it is laid out at once and
	-- the clip eases to it.
	local laid = ui.use_state(open())
	local leaving: (() -> ())? = nil
	ui.effect(function()
		if open() then
			if leaving then
				leaving()
				leaving = nil
			end
			laid:set(true)
		elseif laid:peek() then
			leaving = ui.after(FOLD_MS + 40, function()
				leaving = nil
				laid:set(false)
			end)
		end
	end)
	return ui.column {
		w_full = true,
		shrink = 0,
		overflow = "hidden",
		-- (Folded to a point, not nothing: a clip of no height holding a
		-- scroll is a layout warning while the fold plays.)
		h = ui.tween(ui.computed(function(): number
			return if open() then want() else 1
		end), FOLD),
		opacity = ui.tween(ui.computed(function(): number
			return if open() then 1 else 0
		end), { duration = 180 }),
		-- The scroll fills the clip: as tall as the body, or the cap, and
		-- past the cap the body scrolls in it.
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 1,
			hidden = function()
				return not laid:get()
			end,
			overscroll = "contain",
			offset = offset,
			fade = edge_fades(offset, function(): number
				return math.max(0, (inner.height or 0) - want())
			end, FADE_DEPTH),
			ui.column {
				ref = inner,
				w_full = true,
				shrink = 0,
				on_resize = function(size: { w: number, h: number })
					if math.abs((SEEN:peek()[key] or 0) - size.h) >= 1 then
						SEEN:set_key(key, size.h)
					end
				end,
				body,
			},
		},
	}
end

-- A tab of the deck: its glyph, its name (a message's key, or a function
-- giving one: a tab whose name follows what it holds), its count, the line
-- that stands for it while the deck is folded, whatever it adds at the
-- header's end, its rows, and whether it has anything to show.
type Tab = {
	id: Tray,
	glyph: Node,
	title: string | () -> string,
	count: () -> string,
	summary: () -> string,
	trail: { Node }?,
	body: Node,
	present: () -> boolean,
	-- Its glyph again, for the folded pill (a node lives in one place).
	pill_glyph: Node,
	-- What it reads as in the folded pill, as one line ("2 queued", "Plan 3/5").
	pill_text: (() -> string)?,
	-- A tab that says something's wrong: its tone (the worst it holds),
	-- which colours it, in the pill on its tone's wash.
	tone: (() -> AlertTone)?,
}

-- The deck's tabs: a capsule each, quiet until touched; the one open sits
-- on a thumb that springs from tab to tab (the segmented control's), there
-- only while there are tabs to choose between and one is open.
-- A tab's look: on the thumb (full ink, no wash of its own over it), alone
-- (a title: full ink, washed as it is touched), or one of several not open
-- (quieter, coming up as it is touched).
local tab_ink = ui.variants {
	base = { bg = "#00000000" },
	variants = {
		look = {
			thumb = { color = "$text.primary", hover = { bg = "#00000000" }, active = { bg = "#00000000" } },
			title = { color = "$text.primary", hover = { bg = "$hover" }, active = { bg = "$pressed" } },
			quiet = { color = "$text.secondary", hover = { bg = "$hover", color = "$text.primary" }, active = { bg = "$pressed", color = "$text.primary" } },
		},
	},
	defaults = { look = "quiet" },
}
local THUMB_SPRING: SpringOptions = { stiffness = 380, damping = 30 }
-- The folded pill: its height and the capsules' inset in it (the "Scroll
-- to end" pill's height, ds/transcript), and how far over the composer it
-- floats (that pill's six).
local PILL_H, PILL_PAD = 30, 3
-- Between it and "Scroll to end", side by side.
local PILL_PAIR_GAP = 8
M.PILL_PAIR_GAP = PILL_PAIR_GAP
M.PILL_H, M.PILL_GAP = PILL_H, 6

-- The deck, in two parts. Folded, a pill floating over the conversation
-- just above the composer (the "Scroll to end" pill's glass and height):
-- each thing waiting its glyph and count, a tap opening it. Open, the
-- composer's own top: a header of tabs (one alone reads as a title), the
-- open tab's rows, a rule, then the pill. Returns the top, the pill,
-- whether it shows, and its left edge at rest (from the width's left).
local function deck(tabs: { Tab }, open: State<Tray?>, geometry: { [string]: number }, narrow: () -> boolean, props: Props): (Node, Node, () -> boolean, () -> number?, () -> boolean)
	local aside, on_pill_width = props.aside, props.on_pill_width
	local inset, tab_h, gap = geometry.inset, geometry.tab, geometry.tab_gap
	local function present(): { Tab }
		local out: { Tab } = {}
		for _, t in tabs do
			if t.present() then
				table.insert(out, t)
			end
		end
		return out
	end
	local function any(): boolean
		return #present() > 0
	end
	local function many(): boolean
		return #present() > 1
	end
	-- The tab the header stands for: the one open, else the one open last,
	-- else the first there.
	local last = ui.use_state(nil :: Tray?)
	ui.effect(function()
		local o = open:get()
		if o then
			last:set(o)
		end
	end)
	local function current(): Tray?
		local list = present()
		local want = open:get() or last:get()
		for _, t in list do
			if t.id == want then
				return t.id
			end
		end
		return if list[1] then list[1].id else nil
	end
	local function is_open(): boolean
		return open:get() ~= nil and any()
	end
	-- The top is laid out while open, and until its fold has played.
	local laid = ui.use_state(is_open())
	local leaving: (() -> ())? = nil
	ui.effect(function()
		if is_open() then
			if leaving then
				leaving()
				leaving = nil
			end
			laid:set(true)
		elseif laid:peek() then
			leaving = ui.after(FOLD_MS + 40, function()
				leaving = nil
				laid:set(false)
			end)
		end
	end)

	local refs: { [Tray]: any } = {}
	local tab_nodes: { Node } = {}
	for _, t in tabs do
		local id = t.id
		local ref = ui.ref()
		refs[id] = ref
		local function chosen(): boolean
			return open:get() == id and many()
		end
		local function title_key(): string
			local k = t.title
			return if type(k) == "function" then k() else k
		end
		table.insert(
			tab_nodes,
			ui.show(
				t.present,
				ui.button {
					ref = ref,
					h = tab_h,
					shrink = 0,
					pl = "$space.2",
					pr = ui.raw(10, "a step past the count, so it sits off the capsule's curve"),
					gap = "$zeron.space.tight",
					items = "center",
					rounded = ui.raw(tab_h / 2, "a capsule: the frame's corner less its edge and the header's inset"),
					cursor = "pointer",
					role = "tab",
					selected = chosen,
					expanded = function(): boolean
						return open:get() == id
					end,
					label = function()
						return locale.s(if open:get() == id then "ds.composer_dock.fold" else "ds.composer_dock.open", { title = locale.s(title_key()) })
					end,
					focus = control.ring,
					variants = tab_ink {
						look = function(): string
							return if chosen() then "thumb" elseif not many() then "title" else "quiet"
						end,
					},
					on_click = function()
						open:set(if open:peek() == id then nil else id)
					end,
					ui.div {
						w = 18,
						h = 18,
						shrink = 0,
						items = "center",
						justify = "center",
						color = function(): string
							return if open:get() == id or not many() then "$text.secondary" else "$text.tertiary"
						end,
						t.glyph,
					},
					ui.show(function(): boolean
						return not (narrow() and many())
					end, ui.text {
						function(): string
							return locale.s(title_key())
						end,
						shrink = 0,
						text_size = "$font.size.md",
						weight = "$font.weight.medium",
					}),
					RollText {
						t.count,
						shrink = 0,
						tabular = true,
						text_size = "$font.size.md",
						color = if t.tone
							then function(): string
								return `$zeron.tone.{(t.tone :: () -> AlertTone)()}.ink`
							end
							else "$text.tertiary",
					},
				}
			)
		)
	end
	-- The thumb under the open tab: where it starts and how wide, from the
	-- widths of the tabs there before it.
	local function thumb_box(): (number, number)
		local cur = open:get()
		local x = 0
		for _, t in present() do
			local w = refs[t.id].width or 0
			if t.id == cur then
				return x, w
			end
			x += w + gap
		end
		return 0, 0
	end
	local thumb = ui.div {
		absolute = true,
		left = 0,
		top = 0,
		h = tab_h,
		rounded = ui.raw(tab_h / 2, "the tab's capsule"),
		bg = "$zeron.composer.deck.thumb",
		pointer_events = "none",
		opacity = ui.tween(ui.computed(function(): number
			return if is_open() and many() then 1 else 0
		end), { duration = 160 }),
		x = motion.spring(ui.computed(function(): number
			local x = thumb_box()
			return x
		end), THUMB_SPRING),
		w = motion.spring(ui.computed(function(): number
			local _, w = thumb_box()
			return w
		end), THUMB_SPRING),
	}
	local strip: { [any]: any } = {
		relative = true,
		h = tab_h,
		shrink = 1,
		min_w = 0,
		gap = "$zeron.space.hair",
		items = "center",
		role = "tablist",
		thumb,
	}
	for _, node in tab_nodes do
		table.insert(strip, node)
	end

	-- Past the tabs, a click folds it.
	local spacer = ui.div {
		grow = 1,
		h_full = true,
		cursor = "pointer",
		on_click = function()
			open:set(nil)
		end,
	}
	-- At the header's end: what the tab it stands for adds (Stop all, the
	-- plan's dismiss), then the chevron that opens it or folds it.
	local trail: { Child } = {}
	for _, t in tabs do
		if t.trail and #t.trail > 0 then
			local id = t.id
			table.insert(trail, ui.show(function(): boolean
				return current() == id
			end, ui.row { shrink = 0, items = "center", gap = "$space.1", table.unpack(t.trail) }))
		end
	end
	local chevron = ui.div {
		w = 28,
		h = 28,
		shrink = 0,
		items = "center",
		justify = "center",
		rounded = ui.raw(14, "round, as the pill's controls"),
		color = "$text.tertiary",
		cursor = "pointer",
		role = "button",
		focusable = true,
		label = locale.t("ds.composer_dock.fold_all"),
		hover = { bg = "$hover", color = "$text.secondary" },
		active = { bg = "$pressed", color = "$text.secondary" },
		focus = control.ring,
		on_click = function()
			open:set(nil)
		end,
		Icon { name = "alt-arrow-down", size = 14 },
	}
	local head_row: { [any]: any } = {
		h = tab_h,
		shrink = 0,
		gap = "$space.2",
		items = "center",
		ui.row(strip),
		spacer,
	}
	for _, c in trail do
		table.insert(head_row, c)
	end
	table.insert(head_row, chevron)

	-- The rows of the tab it stands for.
	local bodies: { [string]: Node } = {}
	for _, t in tabs do
		bodies[t.id] = ui.column {
			w_full = true,
			px = ui.raw(inset, "the header's inset: rows' glyphs under the tabs'"),
			pb = ui.raw(inset - 4, "the header's inset, less the rows' own"),
			t.body,
		}
	end
	bodies.none = ui.div {}
	local body = folding(is_open, geometry.body, ui.switch(function(): string
		return current() or "none"
	end, bodies), "deck")

	local top = ui.column {
		w_full = true,
		shrink = 0,
		hidden = function(): boolean
			return not laid:get()
		end,
		overflow = "hidden",
		-- The header grows in from nothing as something arrives (the frame
		-- above the pill rising with it), and goes the same way.
		ui.column {
			w_full = true,
			shrink = 0,
			overflow = "hidden",
			h = ui.tween(ui.computed(function(): number
				return if is_open() then tab_h + inset * 2 else 0
			end), FOLD),
			opacity = ui.tween(ui.computed(function(): number
				return if is_open() then 1 else 0
			end), { duration = 180 }),
			ui.column {
				w_full = true,
				p = ui.raw(inset, "the header's inset: its capsules concentric with the frame"),
				ui.row(head_row),
			},
		},
		body,
		-- The rule between it and the pill, edge to edge (a menu's).
		ui.div {
			h = 1,
			w_full = true,
			shrink = 0,
			bg = "$border.subtle",
			aria_hidden = true,
			opacity = ui.tween(ui.computed(function(): number
				return if is_open() then 1 else 0
			end), { duration = 180 }),
		},
	}

	-- The pill: each thing waiting, its glyph and count (one alone, its name
	-- too), a tap opening that one; the chevron opens the one open last.
	-- Tabs that each read as words (the queue's, the plan's) make one line
	-- instead: the list's glyph, then each, "·" between, no chevron.
	-- (Its width, as laid out: how far it moves aside.)
	local pill_w = props.pill_w or ui.use_state(0)
	local function words(): boolean
		for _, t in present() do
			if not t.pill_text then
				return false
			end
		end
		return true
	end
	local function pill(): Node
		local p = glass.style({
			h = PILL_H,
			px = ui.raw(PILL_PAD, "the capsules' inset in the pill"),
			gap = "$zeron.space.hair",
			items = "center",
			shrink = 0,
			rounded = ui.raw(PILL_H / 2, "a capsule, as the Scroll to end pill"),
			role = "toolbar",
			label = locale.t("ds.composer_dock.waiting"),
		}, "menu")
		p.shadow = "$shadow.md"
		local inner = PILL_H - PILL_PAD * 2
		table.insert(p, ui.show(words, ui.div {
			w = 16,
			h = 16,
			ml = "$space.2",
			shrink = 0,
			items = "center",
			justify = "center",
			color = "$text.secondary",
			Icon { name = "list", size = 14 },
		}))
		for i, t in tabs do
			local id = t.id
			local function title_key(): string
				local k = t.title
				return if type(k) == "function" then k() else k
			end
			local function after_another(): boolean
				for j = 1, i - 1 do
					if tabs[j].present() then
						return true
					end
				end
				return false
			end
			table.insert(p, ui.show(function(): boolean
				return words() and t.present() and after_another()
			end, ui.text { "·", shrink = 0, text_size = "$font.size.sm", color = "$text.tertiary", aria_hidden = true }))
			local tone = t.tone
			local said = t.pill_text
			local glyph_and_count = ui.row {
				gap = "$zeron.space.tight",
				items = "center",
				shrink = 0,
				ui.div { w = 16, h = 16, shrink = 0, items = "center", justify = "center", t.pill_glyph },
				ui.show(function(): boolean
					return not many()
				end, ui.text {
					function(): string
						return locale.s(title_key())
					end,
					shrink = 0,
					text_size = "$font.size.sm",
					weight = "$font.weight.medium",
					color = "$text.primary",
				}),
				RollText {
					t.count,
					shrink = 0,
					tabular = true,
					text_size = "$font.size.sm",
					weight = "$font.weight.medium",
					color = if tone
						then function(): string
							return `$zeron.tone.{tone()}.ink`
						end
						else nil,
				},
			}
			table.insert(p, ui.show(t.present, ui.button {
				relative = true,
				h = inner,
				pl = "$space.2",
				pr = "$space.2",
				items = "center",
				shrink = 0,
				rounded = ui.raw(inner / 2, "concentric with the pill: its corner less the inset"),
				bg = "#00000000",
				color = "$text.secondary",
				cursor = "pointer",
				hover = { bg = "$hover", color = "$text.primary" },
				active = { bg = "$pressed", color = "$text.primary" },
				focus = control.ring,
				label = function()
					return locale.s("ds.composer_dock.open", { title = locale.s(title_key()) })
				end,
				on_click = function()
					open:set(id)
				end,
				-- (Something wrong: on its tone's wash, under the touch's.)
				if tone
					then ui.div {
						absolute = true,
						left = 0,
						right = 0,
						top = 0,
						bottom = 0,
						rounded = ui.raw(inner / 2, "the capsule's"),
						pointer_events = "none",
						bg = function(): string
							return `$zeron.tone.{tone()}.wash`
						end,
					}
					else nil,
				if said
					then ui.show(words, ui.text {
						said,
						shrink = 0,
						tabular = true,
						text_size = "$font.size.sm",
						weight = "$font.weight.medium",
						color = "$text.primary",
					}, glyph_and_count)
					else glyph_and_count,
			}))
		end
		table.insert(p, ui.show(function(): boolean
			return not words()
		end, ui.button {
			w = inner,
			h = inner,
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = ui.raw(inner / 2, "round, as the capsules beside it"),
			bg = "#00000000",
			color = "$text.tertiary",
			cursor = "pointer",
			hover = { bg = "$hover", color = "$text.primary" },
			active = { bg = "$pressed", color = "$text.primary" },
			focus = control.ring,
			label = locale.t("ds.composer_dock.open_all"),
			on_click = function()
				open:set(current())
			end,
			Icon { name = "alt-arrow-up", size = 14 },
		}))
		p.on_resize = function(size: { w: number, h: number })
			if math.abs(pill_w:peek() - size.w) >= 1 then
				pill_w:set(size.w)
			end
		end
		return ui.row(p)
	end
	-- Centred over the composer; while another pill floats there ("Scroll
	-- to end", `aside` its width), the two glide apart into a centred pair,
	-- this one on the right (the other moves left by as much: `on_pill_width`
	-- says how wide this one is while it shows). Short of room before
	-- `clear_right`, the pair moves left, never past the width's left edge;
	-- with no room for the pair at all, the other stands higher (`crowded`)
	-- and this one alone stops short of `clear_right`. Returns its offset
	-- from the centre, the width the other moves aside for (nil: it stands
	-- centred), and whether the pair is crowded.
	local function placed(): (number, number?, boolean)
		local other = if aside then aside() else nil
		local w = if props.width then props.width() else 0
		local pw = pill_w:get()
		local clear = if props.clear_right then props.clear_right() else 0
		local beside = props.pairs == nil or props.pairs()
		if w <= 0 or pw <= 0 then
			local want = if beside and other and other > 0 then (other + PILL_PAIR_GAP) / 2 else 0
			return want, if beside then pw else nil, false
		end
		local room = if clear > 0 then w - clear else w
		local centred = (w - pw) / 2
		local alone = math.max(math.min(0, room - pw - centred), -centred)
		if not beside then
			return alone, nil, false
		end
		if not other or other <= 0 then
			return alone, pw, false
		end
		local total = other + PILL_PAIR_GAP + pw
		if total > room then
			return alone, nil, true
		end
		local left = math.max(0, math.min((w - total) / 2, room - total))
		return left + other + PILL_PAIR_GAP - centred, w - other - 2 * left - PILL_PAIR_GAP, false
	end
	local floating = ui.div {
		shrink = 0,
		x = motion.spring(ui.computed(function(): number
			return (placed())
		end), THUMB_SPRING),
		ui.presence(function(): string?
			return if any() and not is_open() then "pill" else nil
		end, pill, {
			enter = { opacity = { 0, 1 }, y = { 8, 0 }, duration = 180, ease = "outQuad" },
			exit = { opacity = 0, y = 8, duration = 120 },
		}),
	}
	if on_pill_width then
		local report = on_pill_width
		ui.effect(function()
			local w = pill_w:get()
			local _, wide = placed()
			report(if any() and not is_open() and w > 0 then wide else nil)
		end)
	end
	return top, floating, function(): boolean
		return any() and not is_open()
	end, function(): number?
		local w = if props.width then props.width() else 0
		local pw = pill_w:get()
		if not any() or is_open() or w <= 0 or pw <= 0 then
			return nil
		end
		return (w - pw) / 2 + (placed())
	end, function(): boolean
		local _, _, crowded = placed()
		return any() and not is_open() and crowded
	end
end

-- A running task or terminal: its glyph, its name, a word on it, how long,
-- and Stop. A subagent's is its mark (working, or how it ended), its title
-- and, ended, how; with `on_open_task` the whole row opens it, and it has no
-- Stop (the host stops a process, not a subagent).
-- A command as one line: its line breaks as spaces (a script of several
-- lines would otherwise stand as several rows in a one-line slot).
local function one_line(text: string): string
	return (string.gsub(string.gsub(text, "%s*[\r\n]+%s*", " "), "^%s+", ""))
end

local function task_row(t: Task, row: State<Task>, now: State<number>, props: Props, height: number): Node
	local function ended(): boolean
		return row:get().ended ~= nil
	end
	local agent = t.kind == "agent"
	local glyph: Icon.Name = if t.kind == "terminal" then "terminal" elseif agent then "bot" else "action-play"
	local opens = agent and props.on_open_task ~= nil
	local function open()
		if props.on_open_task then
			props.on_open_task(t.id)
		end
	end
	local p: { [any]: any } = {
		h = height,
		pl = "$space.2",
		pr = "$space.1",
		gap = "$space.2",
		items = "center",
		-- Concentric with the tray, as its head is.
		rounded = "$zeron.composer.tray.row_radius",
		role = if opens then "button" else "listitem",
		hover = { bg = "$hover" },
	}
	if opens then
		p.label = locale.t("ds.composer_dock.open_agent", { name = t.name })
		p.cursor = "pointer"
		p.focus = control.ring
		p.active = { bg = "$pressed" }
		p.on_click = open
	end
	local parts: { Child } = {
		ui.div {
			w = 18,
			h = 18,
			shrink = 0,
			items = "center",
			justify = "center",
			color = function()
				return if ended() then "$text.tertiary" else "$text.secondary"
			end,
			if agent
				then ui.switch(function(): string
					local r = row:get()
					return if not r.ended then "working" elseif r.failed then "failed" else "done"
				end, {
					working = status.StatusIcon { kind = "working", size = 14 },
					done = status.StatusIcon { kind = "done", size = 14 },
					failed = status.StatusIcon { kind = "failed", size = 14 },
				})
				else Icon { name = glyph, size = 14 },
		},
		FadeText {
			t.name,
			literal = true,
			shrink = 1,
			min_w = 48,
			font = if agent then nil else "$font.mono",
			text_size = if agent then "$font.size.md" else "$font.size.sm",
			color = function()
				return if ended() then "$text.tertiary" else "$text.primary"
			end,
		},
		-- The word on it takes what the name leaves (from nothing, not from
		-- its own width): a long command beside it would otherwise take the
		-- larger share of the shrinking and cut the name to a few letters.
		if t.detail
			then FadeText { one_line(t.detail), literal = true, grow = 1, basis = 0, text_size = "$font.size.sm", color = "$text.tertiary" }
			else ui.div { grow = 1 },
		ui.text {
			function(): string
				local e = row:get().ended
				if e then
					return e
				end
				local since = row:get().since
				return if since then status.elapsed(now:get() - since) else ""
			end,
			shrink = 0,
			tabular = true,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		},
	}
	if not agent then
		table.insert(
			parts,
			ui.show(
				function()
					return not ended()
				end,
				IconButton {
					icon = "stop",
					label = locale.t("ds.composer_dock.stop_named", { name = t.name }),
					tooltip = locale.s("ds.composer_dock.stop"),
					size = "sm",
					tone = "quiet",
					on_click = function()
						if props.on_stop_task then
							props.on_stop_task(t.id)
						end
					end,
				}
			)
		)
	else
		-- (Room where a process's Stop is, so the rows' ends line up.)
		table.insert(parts, ui.div { w = 6, shrink = 0 })
	end
	for _, c in parts do
		table.insert(p, c)
	end
	return ui.row(p)
end

-- An alert: its tone's glyph in its ink, what it is, a word more, its
-- action, and letting it go.
local function alert_row(a: Alert, row: State<Alert>, props: Props, height: number): Node
	local function ink(): string
		return `$zeron.tone.{row:get().tone}.ink`
	end
	local parts: { [any]: any } = {
		min_h = height,
		pl = "$space.2",
		pr = "$space.1",
		gap = "$space.2",
		items = "center",
		rounded = "$zeron.composer.tray.row_radius",
		role = "listitem",
		hover = { bg = "$hover" },
		ui.div {
			w = 18,
			h = 18,
			shrink = 0,
			items = "center",
			justify = "center",
			color = ink,
			ui.show(function(): boolean
				return row:get().spinner == true
			end, status.StatusIcon { kind = "working", size = 14 }, Icon {
				name = function(): Icon.Name
					return if row:get().tone == "info" then "info-circle" else "danger-triangle"
				end,
				size = 14,
			}),
		},
		FadeText {
			function(): string
				return row:get().title
			end,
			literal = true,
			shrink = 1,
			min_w = 48,
			text_size = "$font.size.md",
			color = "$text.primary",
		},
		FadeText {
			function(): string
				return row:get().detail or ""
			end,
			literal = true,
			grow = 1,
			basis = 0,
			text_size = "$font.size.sm",
			color = "$text.tertiary",
		},
	}
	if props.on_alert_action then
		table.insert(parts, ui.show(function(): boolean
			return row:get().action ~= nil
		end, Button {
			ui.text {
				function(): string
					return row:get().action or ""
				end,
			},
			size = "sm",
			tone = "ghost",
			h = 24,
			px = "$space.2",
			text_size = "$font.size.xs",
			color = ink,
			on_click = function()
				(props.on_alert_action :: (string) -> ())(a.id)
			end,
		}))
	end
	if props.on_dismiss_alert then
		table.insert(parts, ui.show(function(): boolean
			return row:get().dismissable ~= false
		end, IconButton {
			icon = "close",
			label = function(): string
				return locale.s("ds.composer_dock.dismiss_alert", { title = row:get().title })
			end,
			tooltip = locale.s("ds.composer_dock.dismiss"),
			size = "sm",
			tone = "quiet",
			on_click = function()
				(props.on_dismiss_alert :: (string) -> ())(a.id)
			end,
		}, ui.div { w = 6, shrink = 0 }))
	else
		table.insert(parts, ui.div { w = 6, shrink = 0 })
	end
	return ui.row(parts)
end

-- "3 finished": a row that opens the subagents that are done, or folds them.
local function finished_row(count: () -> number, open: () -> boolean, on_click: () -> ()): Node
	return ui.row {
		h = 28,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		cursor = "pointer",
		rounded = "$zeron.composer.tray.row_radius",
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		focus = control.ring,
		role = "button",
		expanded = open,
		label = function()
			return locale.s(if open() then "ds.composer_dock.hide_finished" else "ds.composer_dock.show_finished", { n = count() })
		end,
		on_click = on_click,
		ui.div { w = 18, shrink = 0, items = "center", justify = "center", color = "$text.tertiary", Icon { name = "check-circle", size = 14 } },
		ui.text {
			function()
				return locale.s("ds.composer_dock.n_finished", { n = count() })
			end,
			text_size = "$font.size.sm",
			color = "$text.tertiary",
		},
		ui.div { grow = 1 },
		ui.div {
			shrink = 0,
			color = "$text.tertiary",
			Icon {
				name = function(): Icon.Name
					return if open() then "alt-arrow-up" else "alt-arrow-down"
				end,
				size = 12,
			},
		},
	}
end

local function build(props: Props): ({ Tab }, State<Tray?>, { [string]: number }, () -> boolean)
	local geometry = {
		inset = n("zeron.composer.deck.inset", 10),
		tab = n("zeron.composer.deck.tab", 30),
		tab_gap = n("zeron.space.hair", 2),
		row = n("zeron.composer.tray.row", 32),
		body = n("zeron.composer.tray.body", 216),
	}
	local open: State<Tray?> = props.open or ui.use_state(nil :: Tray?)
	local narrow = props.narrow or function()
		return false
	end
	local live = props.live or function()
		return false
	end
	-- Which trays are there: the queue with rows, the plan with steps,
	-- tasks running.
	local function has_queue(): boolean
		return props.queue ~= nil and #props.queue:get() > 0
	end
	local function has_todos(): boolean
		return props.todos ~= nil and #props.todos:get() > 0
	end
	-- What runs: each tab's own `present` (made with the tab, below).
	local present_fns: { { id: Tray, present: () -> boolean } } = {}
	local function has(id: Tray): boolean
		for _, f in present_fns do
			if f.id == id then
				return f.present()
			end
		end
		-- (Its tab not made yet: as it was.)
		return props.tasks ~= nil
	end
	local function all_done(): boolean
		if not props.todos then
			return false
		end
		for _, t in props.todos:get() do
			if t.state ~= "done" then
				return false
			end
		end
		return true
	end
	-- One gone while open, closes.
	ui.effect(function()
		local now = open:get()
		if now == "queue" and not has_queue() then
			open:set(nil)
		elseif now == "todos" and not has_todos() then
			open:set(nil)
		elseif (now == "tasks" or now == "agents") and not has(now) then
			open:set(nil)
		elseif now == "alerts" and (props.alerts == nil or #props.alerts:get() == 0) then
			open:set(nil)
		end
	end)
	-- todo_panel.rs `observe`: the plan folds as it settles (every step done, the turn at rest), once, so opened by hand after that it stays.
	local settled_was = false
	ui.effect(function()
		local settled = has_todos() and all_done() and not live()
		if settled and not settled_was and open:peek() == "todos" then
			open:set(nil)
		end
		settled_was = settled
	end)
	-- Nothing opens itself: what arrives shows in the folded pill, and the
	-- person opens it if they want it.

	local tabs: { Tab } = {}

	-- What to know: first, as the most pressing.
	if props.alerts then
		local alerts = props.alerts
		local function worst(): AlertTone
			local w: AlertTone = "info"
			for _, x in alerts:get() do
				if x.tone == "danger" then
					return "danger"
				elseif x.tone == "warning" then
					w = "warning"
				end
			end
			return w
		end
		local function glyph(size: number): Node
			return ui.div {
				color = function(): string
					return `$zeron.tone.{worst()}.ink`
				end,
				Icon { name = "danger-triangle", size = size },
			}
		end
		local body = ui.column {
			w_full = true,
			pb = "$space.1",
			role = "list",
			label = locale.t("ds.composer_dock.alerts"),
			ui.each(alerts, function(x: Alert): Node
				local row = ui.computed(function(): Alert
					for _, y in alerts:get() do
						if y.id == x.id then
							return y
						end
					end
					return x
				end)
				return alert_row(x, row, props, geometry.row)
			end, {
				key = function(x: Alert): string
					return x.id
				end,
				animate = { duration = 220, ease = "outCubic" },
				enter = { opacity = { 0, 1 }, y = { -4, 0 }, duration = 200, ease = "outCubic" },
				exit = { opacity = 0, duration = 120 },
			}),
		}
		table.insert(tabs, {
			id = "alerts",
			glyph = glyph(16),
			pill_glyph = glyph(14),
			title = "ds.composer_dock.alerts",
			count = function()
				return tostring(#alerts:get())
			end,
			summary = function(): string
				local first = alerts:get()[1]
				return if first then first.title else ""
			end,
			body = body,
			present = function(): boolean
				return #alerts:get() > 0
			end,
			tone = worst,
		})
	end

	-- The queue.
	if props.queue then
		local queue = props.queue
		local body, steer_all = dock.queue_list(props :: any, queue, narrow, live, geometry.row)
		table.insert(tabs, {
			id = "queue",
			glyph = Icon { name = "list", size = 16 },
			pill_glyph = Icon { name = "list", size = 14 },
			title = "ds.composer_dock.queue",
			count = function()
				return tostring(#queue:get())
			end,
			pill_text = function(): string
				return locale.s("ds.composer_dock.n_queued", { n = #queue:get() })
			end,
			summary = function(): string
				local first = queue:get()[1]
				return if first then (string.gsub(first.text, "%s+", " ")) else ""
			end,
			trail = if steer_all then { steer_all } else nil,
			body = body,
			present = has_queue,
		})
	end

	-- The plan.
	if props.todos then
		local plan = dock.plan(props :: any, props.todos, live)
		table.insert(tabs, {
			id = "todos",
			glyph = ui.div {
				ui.show(all_done, ui.div { color = "$success", Icon { name = "check-circle", size = 16 } }, Icon { name = "checklist", size = 16 }),
			},
			pill_glyph = ui.div {
				ui.show(all_done, ui.div { color = "$success", Icon { name = "check-circle", size = 14 } }, Icon { name = "checklist", size = 14 }),
			},
			title = "ds.composer_dock.plan",
			count = function()
				local done, total = plan.counts()
				return `{done}/{total}`
			end,
			pill_text = function(): string
				local done, total = plan.counts()
				return locale.s("ds.composer_dock.plan_count", { count = `{done}/{total}` })
			end,
			summary = plan.summary,
			trail = plan.trail,
			body = plan.body,
			present = has_todos,
		})
	end

	-- What is running.
	if props.tasks then
		local tasks = props.tasks
		-- One clock for both tabs, ticking only while something runs (a timer
		-- made in an effect belongs to its run, and stops as it runs again).
		local now = ui.use_state(os.time())
		local any_running = ui.computed(function(): boolean
			for _, t in tasks:get() do
				if not t.ended then
					return true
				end
			end
			return false
		end)
		ui.effect(function()
			if not any_running:get() then
				return
			end
			now:set(os.time())
			ui.every(1000, function()
				now:set(os.time())
			end)
		end)
		-- A tab of what runs: the subagents (`agents`), or the processes and
		-- terminals (`tasks`).
		local function running_tab(id: Tray, agents: boolean)
			local function mine(t: Task): boolean
				return (t.kind == "agent") == agents
			end
			local function list(): { Task }
				local out: { Task } = {}
				for _, t in tasks:get() do
					if mine(t) then
						table.insert(out, t)
					end
				end
				return out
			end
			local function running(): { Task }
				local out: { Task } = {}
				for _, t in list() do
					if not t.ended then
						table.insert(out, t)
					end
				end
				return out
			end
			-- Whether the subagents that are done are listed (they fold behind
			-- one row while others run; with none running they are the list).
			local show_finished = ui.use_state(false)
			local function finished_count(): number
				if not agents then
					return 0
				end
				local n = 0
				for _, t in list() do
					if t.ended then
						n += 1
					end
				end
				return n
			end
			-- There while any runs; once none does, only while it is open
			-- (what just finished, a subagent to go back to, stays in reach).
			local function present(): boolean
				local l = list()
				for _, t in l do
					if not t.ended then
						return true
					end
				end
				return #l > 0 and open:get() == id
			end
			table.insert(present_fns, { id = id, present = present })
			-- What stands for it: the first running, how long, how many more.
			local function summary(): string
				local r = running()
				local first = r[1]
				if not first then
					return locale.s("ds.composer_dock.nothing_running")
				end
				local s = first.name
				if first.since and not agents then
					s ..= ` · {status.elapsed(now:get() - first.since)}`
				end
				if #r > 1 then
					s ..= ` · {locale.s("ds.composer_dock.more_running", { n = #r - 1 })}`
				end
				return s
			end
			local trail: { Node } = {}
			if not agents and props.on_stop_all then
				table.insert(trail, ui.show(function()
					return #running() > 0
				end, Button {
					locale.t("ds.composer_dock.stop_all"),
					size = "sm",
					tone = "ghost",
					h = 24,
					px = "$space.2",
					text_size = "$font.size.xs",
					on_click = function()
						(props.on_stop_all :: () -> ())()
					end,
				}))
			end
			-- The rows: what runs, in the order given; then (subagents) those
			-- done behind "N finished", listed outright when none runs.
			type Listed = { kind: "task", task: Task } | { kind: "finished" }
			local shown = ui.computed(function(): { Listed }
				local out: { Listed } = {}
				local done: { Task } = {}
				for _, t in list() do
					if agents and t.ended then
						table.insert(done, t)
					else
						table.insert(out, { kind = "task", task = t } :: Listed)
					end
				end
				if #done > 0 then
					local alone = #out == 0
					if not alone then
						table.insert(out, { kind = "finished" } :: Listed)
					end
					if alone or show_finished:get() then
						for _, t in done do
							table.insert(out, { kind = "task", task = t } :: Listed)
						end
					end
				end
				return out
			end)
			local body = ui.column {
				w_full = true,
				pb = "$space.1",
				ui.each(shown, function(item: Listed): Node
					if item.kind == "finished" then
						return finished_row(finished_count, function(): boolean
							return show_finished:get()
						end, function()
							show_finished:set(not show_finished:peek())
						end)
					end
					local t = item.task
					-- The row follows its task (zgx's animated rows get no state
					-- of their own): found in the list by its id.
					local row = ui.computed(function(): Task
						for _, x in tasks:get() do
							if x.id == t.id then
								return x
							end
						end
						return t
					end)
					return task_row(t, row, now, props, geometry.row)
				end, {
					key = function(item: Listed): string
						return if item.kind == "finished" then "\0finished" else item.task.id
					end,
					animate = { duration = 220, ease = "outCubic" },
					enter = { opacity = { 0, 1 }, y = { -4, 0 }, duration = 200, ease = "outCubic" },
					exit = { opacity = 0, duration = 120 },
				}),
			}
			-- Subagents: the right panel's side chat's glyph; what runs, the
			-- command line's.
			local glyph_name: Icon.Name = if agents then "side-chat" else "terminal"
			local name_key = if agents then "ds.composer_dock.subagents" else "ds.composer_dock.running"
			table.insert(tabs, {
				id = id,
				glyph = Icon { name = glyph_name, size = 16 },
				pill_glyph = Icon { name = glyph_name, size = 14 },
				-- (Nothing running, the tab held open: what finished.)
				title = function(): string
					return if #running() > 0 or agents then name_key else "ds.composer_dock.finished"
				end,
				count = function()
					local n = #running()
					return tostring(if n > 0 then n else (if agents then finished_count() else #list()))
				end,
				summary = summary,
				trail = trail,
				body = body,
				present = present,
			})
		end
		running_tab("agents", true)
		running_tab("tasks", false)
	end

	return tabs, open, geometry, narrow
end

-- The deck's two parts, for the composer: its top (inside the frame, over
-- the pill), and the folded pill (floating over the conversation); and
-- whether that pill shows, its left edge at rest, and whether the other
-- pill is too crowded beside it to stand on its line.
function M.parts(props: Props): (Node, Node, () -> boolean, () -> number?, () -> boolean)
	local tabs, open, geometry, narrow = build(props)
	return deck(tabs, open, geometry, narrow, props)
end

-- The deck alone (a fixture's): the pill over the top.
M.Dock = ui.component(function(props: Props): Node
	local top, pill = M.parts(props)
	return ui.column {
		w_full = true,
		shrink = 0,
		gap = ui.raw(M.PILL_GAP, "the pill's gap over the composer"),
		items = "start",
		pill,
		top,
	}
end)

return M
