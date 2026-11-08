-- The trays behind the composer: what waits on the agent, stacked behind
-- the pill as pages behind a page. Each tray is the menus' glass with
-- rounded top corners, tucked under the one below it (and the last under
-- the pill), and each sits a step further in than the one below, so the
-- stack recedes rather than towers. From the pill upward, by what the
-- person will act on soonest: the queue (their own words, sent next), what
-- is running (now), the plan (the whole).
--
-- The queue's rows stand stacked, no head of their own, as gpui's. Of the
-- others one is open at a time; the other folds to its header, a line that
-- still says what matters: "Todo 3/7 · Porting the queue's reorder",
-- "Running 2 · cargo test · 2m 14s". A tray whose work is done folds itself
-- (the plan once every step is done and the turn is idle). A plan that arrives
-- while nothing is open, opens; what is running stays folded.
--
--   Dock { queue = queued, todos = plan, tasks = running, open = tray, … }
--
-- The queue's rows are Zeron's (crates/ui/src/queue.rs): a grip, its first
-- line, its thumbnails, and at its end Remove, Edit and one primary action:
-- "Send now" for a message with cargo (it interrupts), "Steer" where the
-- agent takes a word mid-turn, else "Send next". Rows drag into a new order
-- (or Alt+↑/↓). A row being edited says so. The plan's rows are the todo
-- panel's (todo_panel.rs): done, in progress (the working ring while the
-- turn is live), pending; a long plan folds to the step in progress and its
-- neighbours, "N earlier" and "N later" opening the rest and hiding it again.
-- What is running is ours: a row a task or terminal, how long it has run,
-- Stop at its end, Stop all in the header.

local Icon = require("@self/icon")
local IconButtonModule = require("@self/icon_button")
local IconButton = IconButtonModule.IconButton
local Button = require("@self/button")
local FadeText = require("@self/fade_text")
local status = require("@self/status")
local material = require("@self/material")
local control = require("@self/control")
local cargo = require("@self/composer_cargo")
local locale = require("@self/locale")
local Lightbox = require("@self/lightbox")
local Tooltip = require("@self/tooltip")
local ReplyPicture = require("@self/reply_picture")

local M = {}

export type Queued = {
	id: string,
	text: string,
	cargo: { cargo.Item }?,
	-- Being edited in the composer (its text is there, not here).
	editing: boolean?,
	-- Held elsewhere: by whom ("Editing on devbox").
	locked: string?,
	review: boolean?,
	sending: boolean?,
	removing: boolean?,
	saving: boolean?,
	after_compaction: boolean?,
	key: string?,
}
export type TodoState = "done" | "doing" | "pending"
export type Todo = { id: string, text: string, state: TodoState }
export type TaskKind = "terminal" | "agent" | "process"
export type Task = {
	id: string,
	kind: TaskKind,
	name: string,
	-- A word on it, after its name (":5173", "reading 14 files").
	detail: string?,
	-- When it started, in `os.time()` seconds.
	since: number?,
	-- Done: how it ended ("exited 0").
	ended: string?,
}
export type Tray = "todos" | "tasks" | "queue"
export type Props = {
	queue: State<{ Queued }>?,
	-- Whether the agent takes a word mid-turn (a row's action is then Steer); nil: not known yet.
	can_steer: (boolean | () -> boolean?)?,
	on_queue_send: ((id: string) -> ())?,
	on_queue_steer_all: (() -> ())?,
	-- The pill holds nothing, so the primary modifier and Enter would act on the last row.
	composer_empty: (() -> boolean)?,
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
	-- Which tray is open; the dock keeps its own when not given.
	open: State<Tray?>?,
	-- Short of room: one thumbnail, actions become glyphs.
	narrow: (() -> boolean)?,
	-- Where a row's pictures read from (a chat on another device); without it, each path as it is.
	pictures: ReplyPicture.Pictures?,
}

function M.steerable(q: Queued): boolean
	return not (q.sending or q.locked or q.review or q.editing or q.removing or (q.cargo and #q.cargo > 0))
end

local TEXT = ui.raw(12.5, "queue.rs QUEUE_TEXT_SIZE")
local SMALL = ui.raw(11.5, "queue.rs primary action, todo_panel.rs fold row")
local EDGE_FADE = 24
local MAC = string.find(ui.shortcut_text("cmd"), "⌘", 1, true) ~= nil

local function tray_cap(fallback: number): number
	local window = (ui :: any).window
	local size = if window then window.size else nil
	local v = if size then size:get() else nil
	local h = if type(v) == "table" and type(v.height) == "number" then v.height else 0
	return if h > 1 then math.floor(h * 0.3) else fallback
end

-- The trays' geometry, in points, read as the dock is built (the theme's).
local function n(token: string, fallback: number): number
	local v = ui.theme.get(token)
	return if type(v) == "number" then v else fallback
end

-- How a tray folds and opens (the dropdown's fold).
local FOLD_MS = 240
local FOLD: TweenOptions = { duration = FOLD_MS, ease = "out(3)" }
-- Over how many steps a long plan folds, and to how many.
local PLAN_FOLD_PAST, PLAN_FOLD_TO = 6, 3

-- A faint ring (a pending step), or the accent's ring with a dot (the step
-- in progress while the turn is idle), `size` across.
local function step_ring(size: number, doing: boolean): Node
	return ui.canvas {
		w = size,
		h = size,
		shrink = 0,
		draw = function(c, w, h)
			local r = w / 2 - 0.5
			if doing then
				c:stroke(ui.path():arc(w / 2, h / 2, r, 0, 360), "$accent", 1)
				c:fill(ui.path():arc(w / 2, h / 2, 2, 0, 360), "$accent")
			else
				c:stroke(ui.path():arc(w / 2, h / 2, r, 0, 360), "$text.tertiary", 1)
			end
		end,
	}
end

-- What folds: the body, clipped to nothing while its tray is folded, eased
-- open to its content's height (never past the trays' cap: past it the
-- body scrolls), fading as it goes.
-- Each tray's body height as last laid out, by tray: a tray made again (a
-- reload) opens at it from its first frame rather than growing open again.
local SEEN = ui.state("composer_tray_heights", {} :: { [string]: number })

local function folding(open: () -> boolean, cap: () -> number, body: Node, key: string): Node
	local inner = ui.ref()
	local offset = ui.use_state(0)
	local function want(): number
		local h = inner.height or 0
		if h <= 0 then
			-- (Not yet laid out: as it was.)
			h = SEEN:get()[key] or 0
		end
		return math.min(h, cap())
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
			fade = function()
				local o = offset:get()
				local over = math.max(0, (inner.height or 0) - cap())
				return { math.min(o, EDGE_FADE), math.min(math.max(0, over - o), EDGE_FADE) }
			end,
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

-- A tray's header: its glyph, its name, its count, and (folded) the line
-- that stands for it; at its end whatever the tray adds, then the chevron.
-- A click anywhere on it opens the tray, or folds it.
type Head = {
	id: Tray,
	glyph: Node,
	-- (Its title's message's key.)
	title: string,
	count: (() -> string)?,
	summary: (() -> string)?,
	summary_faint: (() -> boolean)?,
	trail: { Node }?,
	says: { open: string, fold: string }?,
	on_toggle: (() -> ())?,
}
local function head(opts: Head, open: State<Tray?>, height: number): Node
	local function is_open(): boolean
		return open:get() == opts.id
	end
	local function said(): string
		local says = opts.says
		if says then
			return locale.s(if is_open() then says.fold else says.open)
		end
		return locale.s(if is_open() then "ds.composer_dock.fold" else "ds.composer_dock.open", { title = locale.s(opts.title) })
	end
	local titled: { [any]: any } = {
		gap = "$zeron.space.tight",
		items = "center",
		shrink = 0,
		ui.text {
			locale.t(opts.title),
			shrink = 0,
			text_size = TEXT,
			weight = "$font.weight.medium",
			color = "$text.primary",
		},
	}
	if opts.count then
		table.insert(titled, ui.text {
			opts.count,
			shrink = 0,
			tabular = true,
			text_size = TEXT,
			color = "$text.tertiary",
		})
	end
	local parts: { Child } = {
		ui.div { w = 14, h = 14, shrink = 0, items = "center", justify = "center", color = "$text.secondary", opts.glyph },
		ui.row(titled),
	}
	if opts.summary then
		-- Folded: what the tray would say, after its name; open, the room
		-- stays (the header's end holds still).
		table.insert(parts, ui.row {
			grow = 1,
			min_w = 0,
			items = "center",
			opacity = ui.tween(ui.computed(function(): number
				return if is_open() then 0 else 1
			end), { duration = 180 }),
			FadeText {
				opts.summary,
				grow = 1,
				text_size = TEXT,
				color = function()
					return if opts.summary_faint and opts.summary_faint() then "$text.tertiary" else "$text.secondary"
				end,
			},
		})
	else
		table.insert(parts, ui.div { grow = 1 })
	end
	for _, t in opts.trail or {} do
		table.insert(parts, t)
	end
	table.insert(parts, ui.div {
		w = 22,
		h = 22,
		shrink = 0,
		items = "center",
		justify = "center",
		color = "$text.secondary",
		opacity = 0.7,
		Icon {
			name = function(): Icon.Name
				return if is_open() then "alt-arrow-down" else "alt-arrow-up"
			end,
			size = 13,
		},
	})
	local p: { [any]: any } = {
		h = height,
		pl = "$space.2",
		pr = "$space.1",
		gap = "$space.2",
		items = "center",
		shrink = 0,
		-- Concentric with the tray's top corners, through its edge and inset.
		-- Concentric with the tray: its corner less its edge and padding.
		rounded = "$zeron.composer.tray.row_radius",
		cursor = "pointer",
		role = "button",
		label = said,
		tooltip = if opts.says then Tooltip { said } else nil,
		tooltip_delay = if opts.says then 350 else nil,
		expanded = is_open,
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		focus = control.ring,
		on_click = function()
			if opts.on_toggle then
				opts.on_toggle()
			end
			open:set(if is_open() then nil else opts.id)
		end,
	}
	for _, c in parts do
		table.insert(p, c)
	end
	return ui.row(p)
end

-- A tray: the glass, its top corners, its inset by level (1 just behind
-- the pill, 2 behind that…), tucked under what is below it.
local function tray(level: () -> number, head_node: Node, body: Node, open: () -> boolean, geometry: { [string]: number }, key: string): Node
	local function cap(): number
		return tray_cap(geometry.body)
	end
	local step, tuck, radius = geometry.step, geometry.tuck, geometry.radius
	return ui.column {
		shrink = 0,
		mx = function(): any
			return ui.raw(step * level(), "a step in for each tray under the pill")
		end,
		mb = ui.raw(-tuck, "tucked under the next tray, or the pill"),
		rounded_tl = ui.raw(radius, "the tray's corner, worked out from the pill's (geometry)"),
		rounded_tr = ui.raw(radius, "the tray's corner, worked out from the pill's (geometry)"),
		border = 1,
		border_sides = { 1, 1, 0, 1 },
		border_color = "$zeron.composer.tray.edge",
		bg = function()
			return if material.see_through() then "$zeron.composer.tray.tint" else "$zeron.composer.tray.solid"
		end,
		backdrop_blur = function()
			return if material.see_through() then "$zeron.composer.tray.blur" else 0
		end,
		px = "$space.1",
		pt = "$space.1",
		pb = ui.raw(tuck + 4, "the tuck, and the tray's own inset"),
		head_node,
		folding(open, cap, body, key),
	}
end

local function thumbs(now: () -> Queued, key: string, narrow: () -> boolean, props: Props): Node
	local function items(): { cargo.Item }
		return now().cargo or {}
	end
	local function shown(src: string): string
		if not props.pictures then
			return src
		end
		local p = props.pictures.resolve(src)
		return if p and p.state == "ready" and p.src then p.src else ""
	end
	local function images(): { Lightbox.Item }
		local out: { Lightbox.Item } = {}
		for _, it in items() do
			if it.kind == "image" and it.src then
				local src = shown(it.src)
				table.insert(out, { src = src, name = it.name, path = if string.sub(src, 1, 1) == "/" then src else nil })
			end
		end
		return out
	end
	type Entry = { key: string, item: cargo.Item?, picture: number?, tooltip: string?, n: number? }
	local entries = ui.computed(function(): { Entry }
		local list = items()
		local most = if narrow() then 1 else 2
		local out: { Entry } = {}
		local picture = 0
		for i, it in list do
			local is_picture = it.kind == "image" and it.src ~= nil
			if is_picture then
				picture += 1
			end
			if i <= most then
				table.insert(out, { key = `{i}:{it.name}`, item = it, picture = if is_picture then picture else nil })
			end
		end
		if #list > most then
			local names: { string } = {}
			for i = most + 1, #list do
				table.insert(names, list[i].name)
			end
			local rest = #list - most
			table.insert(out, { key = `+{rest}`, n = rest, tooltip = locale.s("ds.composer_dock.rest", { n = rest, names = table.concat(names, " · ") }) })
		end
		return out
	end)
	local function frame(child: Node): Node
		return ui.overlay {
			w_full = true,
			h_full = true,
			rounded = ui.raw(5, "queue.rs thumbnail frame"),
			overflow = "hidden",
			border = 1,
			border_color = "$zeron.composer.chip.edge",
			border_over = true,
			bg = "$hover",
			items = "center",
			justify = "center",
			hover = { border_color = "$accent" },
			child,
		}
	end
	local function clip(icon_name: Icon.Name, size: number): Node
		return ui.div { color = "$text.secondary", Icon { name = icon_name, size = size } }
	end
	local list = ui.each(entries, function(e: Entry): Node
		if e.n then
			return ui.row {
				w = 24,
				h = 24,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = ui.raw(7, "queue.rs row control"),
				bg = "$hover",
				tooltip = e.tooltip,
				label = locale.t("ds.composer_dock.rest_label", { tooltip = e.tooltip or "" }),
				ui.text { `+{e.n}`, tabular = true, text_size = "$font.size.xs", color = "$text.secondary" },
			}
		end
		local it = e.item :: cargo.Item
		if not e.picture then
			return ui.div {
				w = 40,
				h = 28,
				shrink = 0,
				label = it.name,
				tooltip = it.name,
				frame(clip(if it.kind == "annotation" then "cursor" else "document", 16)),
			}
		end
		if props.pictures then
			ui.effect(function()
				(props.pictures :: ReplyPicture.Pictures).fetch(it.src :: string)
			end)
		end
		return Lightbox.Thumb {
			group = `queued-{key}`,
			index = e.picture,
			items = images,
			radius = 5,
			label = locale.t("ds.composer_dock.open_named", { name = it.name }),
			tooltip = it.name,
			w = 40,
			h = 28,
			shrink = 0,
			rounded = ui.raw(5, "queue.rs thumbnail frame"),
			frame(ui.image {
				src = function(): string
					return shown(it.src :: string)
				end,
				fit = "cover",
				w_full = true,
				h_full = true,
				fallback = clip("paperclip", 14),
			}),
		}
	end, {
		key = function(e: Entry): string
			return e.key
		end,
		direction = "row",
		gap = "$space.1",
		items = "center",
		shrink = 0,
	})
	return list
end

-- A queued row.
local function queue_row(q: Queued, i: () -> number, last: () -> boolean, row: State<Queued>, props: Props, narrow: () -> boolean, held: () -> boolean, live: () -> boolean, height: number): Node
	local hovered = ui.use_state(false)
	local function now(): Queued
		return row:get()
	end
	local function has_cargo(): boolean
		local c = now().cargo
		return c ~= nil and #c > 0
	end
	local function steers(): boolean?
		local s = props.can_steer
		if type(s) == "function" then
			return s()
		end
		return s
	end
	local function unknown(): boolean
		return not has_cargo() and steers() == nil and live()
	end
	local function gated(): boolean
		local r = now()
		return r.locked ~= nil or r.review == true or r.removing == true
	end
	-- Gated or not known yet reads "Send now", disabled, as gpui's does.
	local function action(): string
		if has_cargo() or gated() or unknown() then
			return "send_now"
		end
		return if steers() == true then "steer" else "send_next"
	end
	local function can_act(): boolean
		return not (gated() or unknown() or now().sending == true)
	end
	local function words(): string
		return locale.s(`ds.composer_dock.{action()}`)
	end
	local function hint(): string
		if gated() or unknown() then
			return locale.s("ds.composer_dock.waiting_hint")
		end
		return locale.s(`ds.composer_dock.{action()}_hint`)
	end
	local function editing(): boolean
		return now().editing == true
	end
	local function saving(): boolean
		return now().saving == true
	end
	local function removing(): boolean
		return now().removing == true
	end
	local function inert(): boolean
		return removing() or now().sending == true
	end
	local function show_key(): boolean
		return last() and held() and can_act()
	end
	local function send()
		if props.on_queue_send then
			props.on_queue_send(now().id)
		end
	end
	local function line(): string
		if saving() then
			return locale.s("ds.composer_dock.saving")
		end
		if editing() then
			return locale.s("ds.composer_dock.editing")
		end
		local r = now()
		if r.locked then
			return r.locked
		end
		if r.review then
			return locale.s("ds.composer_dock.needs_review")
		end
		local text = string.gsub(r.text or "", "%s+", " ")
		if string.find(text, "%S") == nil and has_cargo() then
			for _, it in r.cargo :: { cargo.Item } do
				if it.kind ~= "image" then
					return locale.s("ds.composer_dock.attached_files")
				end
			end
			return locale.s("ds.composer_dock.attached_images")
		end
		return text
	end
	-- Remove and Edit stand there at all times, a little faint off the pointer.
	local function quiet_when(): number
		return if hovered:get() then 1 else 0.72
	end
	local function glyph_button(opts: { [string]: any }): Node
		opts.size = "sm"
		opts.w = 24
		opts.h = 24
		opts.rounded = ui.raw(7, "queue.rs row control")
		return IconButton(opts :: any)
	end
	local editing_side = ui.row {
		gap = ui.raw(3, "gpui's row actions"),
		shrink = 0,
		items = "center",
		glyph_button {
			label = locale.t("ds.composer_dock.save_to_queue"),
			tooltip = locale.s("ds.composer_dock.save_to_queue"),
			tooltip_delay = 350,
			disabled = saving,
			on_click = function()
				if props.on_queue_save then
					props.on_queue_save(now().id)
				end
			end,
			Icon { name = "check", size = 13 },
		},
		glyph_button {
			label = locale.t("ds.composer_dock.cancel"),
			tooltip = locale.s("ds.composer_dock.cancel"),
			tooltip_delay = 350,
			tone = "quiet",
			disabled = saving,
			on_click = function()
				if props.on_queue_cancel then
					props.on_queue_cancel(now().id)
				end
			end,
			Icon { name = "close", size = 13 },
		},
	}
	local function side(): Node
		local compact = glyph_button {
			label = words,
			tooltip = Tooltip { hint },
			tooltip_delay = 350,
			disabled = function()
				return not can_act()
			end,
			on_click = send,
			ui.show(
				show_key,
				ui.text { if MAC then "⌘↵" else "⌃↵", text_size = SMALL },
				Icon { name = "arrow-up", size = 13 }
			),
		}
		local primary: Node = ui.show(narrow, compact, Button {
			size = "xs",
			tone = "ghost",
			w = 72,
			h = 24,
			px = 0,
			rounded = ui.raw(7, "queue.rs row control"),
			label = words,
			tooltip = Tooltip { hint },
			tooltip_delay = 350,
			disabled = function()
				return not can_act()
			end,
			on_click = send,
			ui.text {
				function(): string
					if show_key() then
						return if MAC then "⌘ Enter" else "Ctrl Enter"
					end
					return words()
				end,
				text_size = SMALL,
			},
		})
		return ui.row {
			gap = ui.raw(3, "gpui's row actions"),
			shrink = 0,
			items = "center",
			ui.row {
				items = "center",
				opacity = ui.tween(ui.computed(function(): number
					return if hovered:get() then 1 else 0
				end), { duration = 120 }),
				glyph_button {
					label = locale.t("ds.composer_dock.edit"),
					tooltip = locale.s("ds.composer_dock.edit"),
					tooltip_delay = 350,
					tone = "quiet",
					disabled = inert,
					on_click = function()
						if props.on_queue_edit then
							props.on_queue_edit(now().id)
						end
					end,
					Icon { name = "pen", size = 13 },
				},
			},
			primary,
			ui.row {
				items = "center",
				opacity = ui.tween(ui.computed(quiet_when), { duration = 120 }),
				ui.show(
					removing,
					glyph_button {
						label = locale.t("ds.composer_dock.removing"),
						tooltip = locale.s("ds.composer_dock.removing"),
						tone = "quiet",
						disabled = true,
						Icon { name = "close", size = 13 },
					},
					glyph_button {
						label = locale.t("ds.composer_dock.remove"),
						tooltip = locale.s("ds.composer_dock.remove"),
						tooltip_delay = 350,
						tone = "quiet",
						disabled = function()
							return now().sending == true
						end,
						on_click = function()
							if props.on_queue_remove then
								props.on_queue_remove(now().id)
							end
						end,
						Icon { name = "close", size = 13 },
					}
				),
			},
		}
	end
	return ui.row {
		h = height,
		pl = "$space.1",
		pr = "$space.1",
		gap = function(): string
			return if narrow() then "$space.1" else "$space.2"
		end,
		items = "center",
		-- Concentric with the tray, as its head is.
		rounded = "$zeron.composer.tray.row_radius",
		role = "listitem",
		label = function()
			return `{i()}. {now().text}`
		end,
		opacity = function(): number
			return if removing() then 0.55 else 1
		end,
		bg = function()
			return if editing() then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		on_hover = function(inside: boolean)
			hovered:set(inside)
		end,
		ui.div {
			w = 14,
			h = 22,
			shrink = 0,
			items = "center",
			justify = "center",
			color = "$text.secondary",
			cursor = function(): string
				return if gated() or editing() then "default" else "pointer"
			end,
			opacity = function(): number
				return if gated() then 0.35 else 0.5
			end,
			ui.show(function()
				return not editing()
			end, Icon { name = "drag-handle", size = 13 }),
		},
		ui.text {
			function(): string
				return `{i()}.`
			end,
			shrink = 0,
			tabular = true,
			text_size = SMALL,
			color = "$text.tertiary",
		},
		FadeText {
			line,
			grow = 1,
			text_size = TEXT,
			color = function()
				return if editing() then "$text.tertiary" else "$text.primary"
			end,
		},
		ui.show(function()
			return now().after_compaction == true and not editing()
		end, ui.text {
			locale.t("ds.composer_dock.after_compaction"),
			shrink = 0,
			text_size = "$font.size.xs",
			color = "$text.secondary",
		}),
		ui.show(function()
			return has_cargo() and not editing()
		end, thumbs(now, q.key or q.id, narrow, props)),
		ui.show(editing, editing_side, side()),
	}
end

-- A step of the plan.
local function todo_row(t: Todo, row: State<Todo>, live: () -> boolean): Node
	local function state(): TodoState
		return row:get().state
	end
	local glyph = ui.div {
		w = 14,
		h = 17,
		shrink = 0,
		items = "center",
		justify = "center",
		ui.show(function()
			return state() == "done"
		end, ui.div { color = "$success", Icon { name = "check", size = 12 } }),
		ui.show(function()
			return state() == "doing" and live()
		end, status.StatusIcon { kind = "working", size = 12 }),
		ui.show(function()
			return state() == "doing" and not live()
		end, step_ring(10, true)),
		ui.show(function()
			return state() == "pending"
		end, step_ring(10, false)),
	}
	return ui.row {
		min_h = 26,
		px = "$space.2",
		py = "$space.1",
		gap = "$space.2",
		items = "start",
		role = "listitem",
		glyph,
		ui.text {
			t.text,
			wrap = true,
			grow = 1,
			min_w = 0,
			text_size = TEXT,
			line_height = ui.raw(17, "todo_panel.rs row"),
			weight = function()
				return if state() == "doing" then "$font.weight.medium" else "$font.weight.regular"
			end,
			color = function()
				local s = state()
				return if s == "done" then "$text.tertiary" elseif s == "doing" then "$text.primary" else "$text.secondary"
			end,
		},
	}
end

local function fold_row(count: () -> number, later: boolean, shown: () -> boolean, on_click: () -> ()): Node
	return ui.row {
		h = 24,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		cursor = "pointer",
		rounded = ui.raw(6, "todo_panel.rs fold row"),
		hover = { bg = "$hover" },
		role = "button",
		label = function()
			return locale.s("ds.composer_dock.show_steps", { n = count(), later = tostring(later), open = tostring(shown()) })
		end,
		on_click = on_click,
		ui.div {
			w = 14,
			shrink = 0,
			items = "center",
			justify = "center",
			color = "$text.tertiary",
			Icon {
				name = function(): Icon.Name
					return if later == shown() then "alt-arrow-up" else "alt-arrow-down"
				end,
				size = 12,
			},
		},
		ui.text {
			function()
				return locale.s("ds.composer_dock.steps", { n = count(), later = tostring(later) })
			end,
			text_size = SMALL,
			color = "$text.tertiary",
		},
	}
end

-- A running task or terminal: its glyph, its name, a word on it, how long,
-- and Stop.
local function task_row(t: Task, row: State<Task>, now: State<number>, props: Props, height: number): Node
	local function ended(): boolean
		return row:get().ended ~= nil
	end
	local glyph: Icon.Name = if t.kind == "terminal" then "terminal" elseif t.kind == "agent" then "bot" else "action-play"
	return ui.row {
		h = height,
		pl = "$space.2",
		pr = "$space.1",
		gap = "$space.2",
		items = "center",
		-- Concentric with the tray, as its head is.
		rounded = "$zeron.composer.tray.row_radius",
		role = "listitem",
		hover = { bg = "$hover" },
		ui.div {
			w = 18,
			h = 18,
			shrink = 0,
			items = "center",
			justify = "center",
			color = function()
				return if ended() then "$text.tertiary" else "$text.secondary"
			end,
			ui.show(function()
				return not ended() and t.kind == "agent"
			end, status.StatusIcon { kind = "working", size = 14 }, Icon { name = glyph, size = 14 }),
		},
		FadeText {
			t.name,
			literal = true,
			shrink = 1,
			min_w = 48,
			font = if t.kind == "agent" then nil else "$font.mono",
			text_size = if t.kind == "agent" then "$font.size.md" else "$font.size.sm",
			color = function()
				return if ended() then "$text.tertiary" else "$text.primary"
			end,
		},
		-- The word on it takes what the name leaves (from nothing, not from
		-- its own width): a long command beside it would otherwise take the
		-- larger share of the shrinking and cut the name to a few letters.
		if t.detail
			then FadeText { t.detail, literal = true, grow = 1, basis = 0, text_size = "$font.size.sm", color = "$text.tertiary" }
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
		),
	}
end

-- The primary modifier held with the pill empty: the newest row says its keys.
function M.key_held(props: Props): () -> boolean
	local modifiers = ui.modifiers
	local function key_held(): boolean
		if not props.composer_empty or not props.composer_empty() then
			return false
		end
		local primary, other = modifiers.control, modifiers.meta
		if MAC then
			primary, other = modifiers.meta, modifiers.control
		end
		return primary:get() and not (modifiers.shift:get() or other:get() or modifiers.alt:get())
	end
	return key_held
end

-- The plan's rows (todo_panel.rs), with what its header says: the deck's Plan tab.
export type Plan = { body: Node, counts: () -> (number, number, number?), summary: () -> string, finished: () -> boolean, trail: { Node }, glyph: Node }
function M.plan(props: Props, todos: State<{ Todo }>, live: () -> boolean): Plan
	local function finished(): boolean
		for _, t in todos:get() do
			if t.state ~= "done" then
				return false
			end
		end
		return #todos:get() > 0
	end
	local show_earlier = ui.use_state(false)
	local show_later = ui.use_state(false)
	local function counts(): (number, number, number?)
		local done, total, current = 0, 0, nil
		for i, t in todos:get() do
			total += 1
			if t.state == "done" then
				done += 1
			elseif t.state == "doing" and current == nil then
				current = i
			end
		end
		return done, total, current
	end
	local function window(): (number, number)
		local list = todos:get()
		local _, total, current = counts()
		if total <= PLAN_FOLD_PAST then
			return 1, total
		end
		local focus = current
		if not focus then
			for i, t in list do
				if t.state ~= "done" then
					focus = i
					break
				end
			end
		end
		local from = math.clamp((focus or total) - 1, 1, total - PLAN_FOLD_TO + 1)
		return from, from + PLAN_FOLD_TO - 1
	end
	type Shown = { kind: "step", todo: Todo } | { kind: "fold", later: boolean }
	local shown = ui.computed(function(): { Shown }
		local list = todos:get()
		local from, to = window()
		local out: { Shown } = {}
		if from > 1 then
			table.insert(out, { kind = "fold", later = false } :: Shown)
		end
		for i = if show_earlier:get() then 1 else from, to do
			table.insert(out, { kind = "step", todo = list[i] } :: Shown)
		end
		if to < #list then
			if show_later:get() then
				for i = to + 1, #list do
					table.insert(out, { kind = "step", todo = list[i] } :: Shown)
				end
			end
			table.insert(out, { kind = "fold", later = true } :: Shown)
		end
		return out
	end)
	local function summary(): string
		local list = todos:get()
		for _, t in list do
			if t.state == "doing" then
				return t.text
			end
		end
		for _, t in list do
			if t.state ~= "done" then
				return t.text
			end
		end
		return locale.s("ds.composer_dock.all_done")
	end
	local trail: { Node } = {}
	if props.on_dismiss_todos then
		table.insert(trail, ui.show(function()
			return not live()
		end, IconButton {
			label = locale.t("ds.composer_dock.dismiss_todos"),
			tooltip = locale.s("ds.composer_dock.dismiss"),
			tooltip_delay = 350,
			size = "sm",
			tone = "quiet",
			w = 24,
			h = 24,
			rounded = ui.raw(5, "todo_panel.rs dismiss"),
			on_click = function()
				(props.on_dismiss_todos :: () -> ())()
			end,
			Icon { name = "close", size = 11 },
		}))
	end
	local glyph = ui.div {
		ui.show(finished, ui.div { color = "$success", Icon { name = "check", size = 14 } }, Icon { name = "checklist", size = 14 }),
	}
	local body = ui.column {
		w_full = true,
		pt = ui.raw(2, "todo_panel.rs rows area"),
		pb = ui.raw(6, "todo_panel.rs rows area"),
		ui.each(shown, function(s: Shown): Node
			if s.kind == "fold" then
				local later = s.later
				local revealed = if later then show_later else show_earlier
				return fold_row(function()
					local from, to = window()
					return if later then #todos:get() - to else from - 1
				end, later, function()
					return revealed:get()
				end, function()
					revealed:set(not revealed:peek())
				end)
			end
			local t = s.todo
			-- The row follows its step (its state changes under it).
			local row = ui.computed(function(): Todo
				for _, x in todos:get() do
					if x.id == t.id then
						return x
					end
				end
				return t
			end)
			return todo_row(t, row, live)
		end, {
			key = function(s: Shown): string
				return if s.kind == "fold" then (if s.later then "\0later" else "\0earlier") else s.todo.id
			end,
			animate = { duration = 150, ease = "outCubic" },
			enter = { opacity = { 0, 1 }, duration = 150, ease = "outCubic" },
		}),
	}
	return { body = body, counts = counts, summary = summary, finished = finished, trail = trail, glyph = glyph }
end

-- The queue's rows, and Steer all while it can do something: the deck's Queue tab.
function M.queue_list(props: Props, queue: State<{ Queued }>, narrow: () -> boolean, live: () -> boolean, height: number): (Node, Node?)
	local key_held = M.key_held(props)
	local body = ui.column {
		w_full = true,
		role = "list",
		label = locale.t("ds.composer_dock.queued_messages"),
		ui.each(queue, function(q: Queued): Node
			local key = q.key or q.id
			local function same(x: Queued): boolean
				return (x.key or x.id) == key
			end
			local row = ui.computed(function(): Queued
				for _, x in queue:get() do
					if same(x) then
						return x
					end
				end
				return q
			end)
			local function index(): number
				for i, x in queue:get() do
					if same(x) then
						return i
					end
				end
				return 0
			end
			local function last(): boolean
				local list = queue:get()
				return list[#list] ~= nil and same(list[#list])
			end
			return queue_row(q, index, last, row, props, narrow, key_held, live, height)
		end, {
			key = function(q: Queued): string
				return q.key or q.id
			end,
			gap = "$space.1",
			animate = { duration = 150, ease = "outCubic" },
			reorder = props.on_queue_reorder,
			item_label = function(q: Queued): string
				return q.text
			end,
		}),
	}
	-- Rows stacked with no head of their own, always open, as gpui's. Steer all
	-- (upstream #1150, not gpui's) stands over them while it can do something.
	local steer_all: Node? = nil
	if props.on_queue_steer_all then
		local function can_steer_all(): boolean
			local s = props.can_steer
			local steers = if type(s) == "function" then s() else s
			if steers ~= true or not live() then
				return false
			end
			local n = 0
			for _, q in queue:get() do
				if M.steerable(q) then
					n += 1
				end
			end
			return n >= 2
		end
		steer_all = ui.show(can_steer_all, ui.row {
			w_full = true,
			justify = "end",
			Button {
				locale.t("ds.composer_dock.steer_all"),
				size = "xs",
				tone = "ghost",
				h = 24,
				px = "$space.2",
				text_size = SMALL,
				tooltip = locale.s("ds.composer_dock.steer_all_hint"),
				tooltip_delay = 350,
				on_click = function()
					(props.on_queue_steer_all :: () -> ())()
				end,
			},
		})
	end
	return body, steer_all
end

M.Dock = ui.component(function(props: Props): Node
	local geometry = {
		radius = n("zeron.composer.tray.radius", 16),
		step = n("zeron.composer.tray.step", 16),
		tuck = n("zeron.composer.tray.tuck", 10),
		header = n("zeron.composer.tray.header", 32),
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
	local function opened(id: Tray): () -> boolean
		return function(): boolean
			return open:get() == id
		end
	end
	-- Which trays are there: the queue with rows, the plan with steps,
	-- tasks running.
	local function has_queue(): boolean
		return props.queue ~= nil and #props.queue:get() > 0
	end
	local function has_todos(): boolean
		return props.todos ~= nil and #props.todos:get() > 0
	end
	local function has_tasks(): boolean
		return props.tasks ~= nil and #props.tasks:get() > 0
	end
	-- Each tray's level: how many trays sit between it and the pill, and one.
	local function queue_level(): number
		return 1
	end
	local function tasks_level(): number
		return 1 + (if has_queue() then 1 else 0)
	end
	local function todos_level(): number
		return 1 + (if has_queue() then 1 else 0) + (if has_tasks() then 1 else 0)
	end
	-- todo_panel.rs `observe`: settling drops a choice once, so a later manual open holds.
	local function finished(): boolean
		if not props.todos or #props.todos:get() == 0 then
			return false
		end
		for _, t in props.todos:get() do
			if t.state ~= "done" then
				return false
			end
		end
		return true
	end
	local todos_chosen = false
	ui.effect(function()
		local now = open:get()
		if now == "queue" then
			open:set(nil)
		elseif now == "todos" and not has_todos() then
			open:set(nil)
		elseif now == "tasks" and not has_tasks() then
			open:set(nil)
		end
	end)
	-- What is running doesn't: it stays folded to its header ("Running 3 ·
	-- cargo test · 2m") until opened, as a session can hold a score of
	-- processes for hours. Opened, it stays open while any run.
	local seen = { working = false, finished = false, settled = false }
	ui.effect(function()
		local done = finished()
		local working = has_todos() and not done
		local settled = done and not live()
		local arrived: Tray? = if working and not seen.working and not todos_chosen then "todos" else nil
		local into_settled = settled and not seen.settled
		local into_finished = done and not seen.finished
		seen.working, seen.finished, seen.settled = working, done, settled
		if into_settled then
			todos_chosen = false
		end
		if (into_settled or into_finished) and not todos_chosen and open:peek() == "todos" then
			open:set(nil)
		end
		if arrived and open:peek() == nil then
			open:set(arrived)
		end
	end)

	local trays: { Node } = {}

	-- The plan.
	if props.todos then
		local plan = M.plan(props, props.todos, live)
		local counts, summary, trail, glyph, body = plan.counts, plan.summary, plan.trail, plan.glyph, plan.body
		table.insert(
			trays,
			ui.show(
				has_todos,
				tray(todos_level, head({
					id = "todos",
					glyph = glyph,
					title = "ds.composer_dock.todo",
					count = function()
						local done, total = counts()
						return `{done}/{total}`
					end,
					summary = summary,
					summary_faint = finished,
					trail = trail,
					says = { open = "ds.composer_dock.todo_expand", fold = "ds.composer_dock.todo_collapse" },
					on_toggle = function()
						todos_chosen = true
					end,
				}, open, geometry.header), body, opened("todos"), geometry, "todos")
			)
		)
	end

	-- What is running.
	if props.tasks then
		local tasks = props.tasks
		local now = ui.use_state(os.time())
		local function running(): { Task }
			local out: { Task } = {}
			for _, t in tasks:get() do
				if not t.ended then
					table.insert(out, t)
				end
			end
			return out
		end
		-- The clock ticks only while something runs (a timer made in an
		-- effect belongs to its run, and stops as it runs again).
		local any_running = ui.computed(function(): boolean
			return #running() > 0
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
		local function summary(): string
			local list = running()
			local first = list[1]
			if not first then
				return locale.s("ds.composer_dock.nothing_running")
			end
			local s = first.name
			if first.since then
				s ..= ` · {status.elapsed(now:get() - first.since)}`
			end
			if #list > 1 then
				s ..= ` · {locale.s("ds.composer_dock.more_running", { n = #list - 1 })}`
			end
			return s
		end
		local trail: { Node } = {}
		if props.on_stop_all then
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
		local body = ui.column {
			w_full = true,
			pb = "$space.1",
			ui.each(tasks, function(t: Task): Node
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
				key = function(t: Task): string
					return t.id
				end,
				animate = { duration = 220, ease = "outCubic" },
				enter = { opacity = { 0, 1 }, y = { -4, 0 }, duration = 200, ease = "outCubic" },
				exit = { opacity = 0, duration = 120 },
			}),
		}
		table.insert(
			trays,
			ui.show(
				has_tasks,
				tray(tasks_level, head({
					id = "tasks",
					glyph = Icon { name = "action-play", size = 14 },
					title = "ds.composer_dock.running",
					count = function()
						return tostring(#running())
					end,
					summary = summary,
					trail = trail,
				}, open, geometry.header), body, opened("tasks"), geometry, "tasks")
			)
		)
	end

	-- The queue.
	if props.queue then
		local body, steer_all = M.queue_list(props, props.queue, narrow, live, geometry.row)
		table.insert(
			trays,
			ui.show(
				has_queue,
				tray(queue_level, steer_all, body, function(): boolean
					return true
				end, geometry, "queue")
			)
		)
	end

	return ui.column {
		w_full = true,
		shrink = 0,
		-- Each tray is painted after the one above it, so the lower tray
		-- covers the tuck of the one behind.
		table.unpack(trays),
	}
end)

return M
