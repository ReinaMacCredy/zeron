-- A chat's compactions as Zeron's transcript tells them (crates/ui/src/
-- transcript/compaction_card.rs): a divider whose label follows one
-- compaction through, queued, compacting (its clock), then landed in the
-- entry's own words (who compacted), pre to post and how long. A landed
-- label that has more to say opens a panel under it: what the agent was
-- asked to keep and the summary it wrote. Tokens, never dollars.
--
--   Divider { m = fn, id }         a compaction's row
--   Wait { wait = fn }             the wait in the trailer's place
--   Chip { label = fn }            the composer's line while it compacts
--   Resume { offer = fn, ... }     "Resume with less context", over the
--                                  composer of a large idle Claude chat
--   head(m)                        a landed label's pieces, as drawn
--   words(m)                       its words (what Find finds)
--
-- Motion: the rules sweep the accent while it compacts (a 1400 ms loop
-- drawn at 30 frames a second, only while it shows), fill green and fade
-- once it lands (320 ms, then a 400 ms fade); the icon crossfades and the
-- words roll as the phase changes; the clock ticks once a second. The panel
-- opens to its height (220 ms). Settled and still, nothing draws.

local Icon = require("@self/icon")
local RollText = require("@self/roll_text")
local Button = require("@self/button")
local status = require("@self/status")
local motion = require("@self/motion")
local locale = require("@self/locale")

local M = {}

export type Kept = { pre: number?, post: number?, ms: number?, summary: string?, focus: string? }
export type Wait = { phase: "queued" | "compacting" }
export type Continued = { note: string?, limit: boolean?, at: string? }
export type Head = {
	icon: "spinner" | "done" | "queued",
	title: string,
	meta: string?,
	continued: string?,
	chevron: boolean,
}

-- gpui's `compact_count`: 940, 212.0k, 1.25M.
function M.count(n: number): string
	if n < 1000 then
		return string.format("%d", n)
	elseif n < 1000000 then
		return string.format("%.1fk", n / 1000)
	end
	return string.format("%.2fM", n / 1000000)
end

local function s(key: string, args: { [string]: any }?): string
	return locale.s(`ds.compaction.{key}`, args)
end

-- gpui's `format_elapsed` (zeron_proto::transcript): 53s, 1m 5s, 2h 3m, 1d 4h.
function M.elapsed(seconds: number): string
	local t = math.max(0, math.floor(seconds))
	if t < 60 then
		return locale.s("time.short.seconds", { n = t })
	elseif t < 3600 then
		return s("minutes_seconds", { m = t // 60, s = t % 60 })
	elseif t < 86400 then
		return locale.s("time.short.hours_minutes", { h = t // 3600, m = (t % 3600) // 60 })
	end
	return s("days_hours", { d = t // 86400, h = (t % 86400) // 3600 })
end

-- A running compaction's clock: 0:07, 1:12.
local function clock(seconds: number): string
	local t = math.max(0, math.floor(seconds))
	return string.format("%d:%02d", t // 60, t % 60)
end

local function kept_of(m: any): Kept
	return if type(m.kept) == "table" then m.kept else {}
end

local function continued_of(m: any): Continued?
	return if type(m.continued) == "table" then m.continued else nil
end

-- (A limit reset's focus is the engine's, not something anyone asked.)
local function focus_of(m: any): string?
	return if m.limit_reset then nil else kept_of(m).focus
end

-- What a landed compaction's label says: the entry's words, then its sizes
-- and how long it took.
function M.head(m: any): Head
	local k = kept_of(m)
	local meta: string? = nil
	if k.pre then
		meta = `· {M.count(k.pre)}`
		if k.post then
			meta ..= ` → {M.count(k.post)}`
		end
		if k.ms then
			meta ..= ` · {M.elapsed(math.floor(k.ms / 1000))}`
		end
	end
	local title = if (m.text or "") ~= "" and not m.limit_reset then m.text else s("compacted")
	local c = continued_of(m)
	return {
		icon = "done",
		title = title,
		meta = meta,
		continued = if c then s(if c.limit then "continued_after_limit" else "continued") else nil,
		chevron = k.summary ~= nil or focus_of(m) ~= nil or (c ~= nil and (c.note ~= nil or c.at ~= nil)),
	}
end

-- What the wait in the trailer's place says.
function M.wait_head(w: Wait, elapsed: number): Head
	if w.phase == "queued" then
		return { icon = "queued", title = s("queued"), meta = s("queued_meta"), chevron = false }
	end
	return { icon = "spinner", title = s("compacting"), meta = `· {clock(elapsed)}`, chevron = false }
end

function M.words_of(h: Head): string
	local words = if h.meta then `{h.title} {h.meta}` else h.title
	return if h.continued then `{words} · {h.continued}` else words
end

function M.words(m: any): string
	return M.words_of(M.head(m))
end

-- (`ui.theme.get` gives "#rrggbb"; the streak fades to its own hue, clear.)
local function clear(hex: string): string
	return if #hex == 7 then `{hex}00` else "#00000000"
end

-- A node whose loop plays only while it can be seen, and not at all with
-- motion reduced (ds/status's).
local function looping(props: { [string]: any }, start: (el: Element) -> any): Node
	local held = setmetatable({}, { __mode = "v" }) :: { anim: any }
	props.on_visible = function(visible: boolean, el: Element)
		if visible then
			if not held.anim and not motion.reduced_now() then
				held.anim = start(el)
			end
		elseif held.anim then
			held.anim:cancel()
			held.anim = nil
		end
	end
	return ui.div(props)
end

-- One rule beside the label: a hairline; while live, the accent sweeping
-- out from the label; once settled, a green fill from the label that fades.
local function rule(left: boolean, live: () -> boolean, fill: State<number>, fading: State<boolean>): Node
	local track = ui.ref()
	return ui.div {
		ref = track,
		grow = 1,
		min_w = 24,
		h = 1,
		relative = true,
		overflow = "hidden",
		bg = "$border.subtle",
		aria_hidden = true,
		ui.show(live, looping({
			absolute = true,
			top = 0,
			left = 0,
			h = 1,
			w = "40%",
			gradient = function()
				local a = ui.theme.get("accent")
				return { angle = 90, stops = if left then { clear(a), a } else { a, clear(a) } }
			end,
		}, function(el)
			local w = track.width or 0
			local from, to = if left then w else -w * 0.4, if left then -w * 0.4 else w
			return anime.animate(el, { x = { from, to }, duration = 1400, ease = "inOut(2)", loop = true, frameRate = 30 })
		end)),
		ui.div {
			absolute = true,
			top = 0,
			h = 1,
			left = if left then nil else 0,
			right = if left then 0 else nil,
			bg = "$success",
			w = ui.tween(ui.computed(function(): number
				return fill:get() * (track.width or 0)
			end), { duration = 320, ease = "outQuint" }),
			opacity = ui.tween(ui.computed(function(): number
				return if fading:get() then 0 elseif fill:get() > 0 then 0.7 else 0
			end), { duration = 400 }),
		},
	}
end

local ICONS: { [string]: { name: string, ink: string } } = {
	done = { name = "lucide-check", ink = "$success" },
	queued = { name = "lucide-circle-dashed", ink = "$text.tertiary" },
}

local function glyph(kind: string): Node
	if kind == "spinner" then
		return status.Spinner { size = 13, ink = "$text.secondary" }
	end
	local i = ICONS[kind]
	return ui.div { color = i.ink, Icon { name = i.name :: any, size = 13 } }
end

-- The label's icon: the new one in as the old one goes, each scaled.
local function label_icon(kind: () -> string): Node
	return ui.div {
		w = 14,
		h = 14,
		shrink = 0,
		relative = true,
		ui.presence(kind, function(k: string): Node
			return ui.div { absolute = true, left = 0, top = 0, w = 14, h = 14, items = "center", justify = "center", glyph(k) }
		end, {
			enter = motion.transition { opacity = { 0, 1 }, scale = { 0.6, 1 }, duration = 200, ease = motion.EASE },
			exit = motion.transition { opacity = 0, scale = 0.6, duration = 200 },
		}),
	}
end

local function continued(head: () -> Head, still: () -> boolean, id: string?): Node
	return ui.show(function(): boolean
		return head().continued ~= nil
	end, ui.component(function(): Node
		local quiet = still()
		local reduced = motion.reduced_now()
		local natural = ui.use_state(0)
		local open = ui.use_state(quiet)
		local lit = ui.use_state(quiet)
		local landed = ui.use_state(quiet or reduced)
		if not quiet and reduced then
			ui.after(0, function()
				lit:set(true)
			end)
		end
		local function grow(w: number)
			if open:peek() or w <= 0 then
				return
			end
			natural:set(w)
			open:set(true)
			ui.after(60, function()
				lit:set(true)
			end)
			ui.after(200, function()
				landed:set(true)
			end)
		end
		return ui.row {
			id = if id then `{id}-continued` else nil,
			class = "compaction-continued",
			min_w = 0,
			overflow = "hidden",
			max_w = if quiet or reduced
				then nil
				else ui.tween(ui.computed(function(): number
					return if open:get() then natural:get() else 0
				end), { duration = 200, ease = "cubicBezier(.2,.8,.2,1)" }),
			opacity = if quiet
				then nil
				else ui.tween(ui.computed(function(): number
					return if lit:get() then 1 else 0
				end), { duration = if reduced then motion.FADE else 150, ease = motion.EASE_OUT }),
			ui.row {
				gap = ui.raw(5, "the mockup's segment"),
				items = "center",
				min_w = 0,
				shrink = function(): number
					return if landed:get() then 1 else 0
				end,
				text_size = ui.raw(11.5, "the mockup's segment"),
				color = "$text.secondary",
				on_resize = if quiet or reduced
					then nil
					else function(size: { w: number, h: number })
						grow(size.w)
					end,
				ui.text { "·", shrink = 0 },
				ui.div { color = "$text.tertiary", shrink = 0, aria_hidden = true, Icon { name = "refresh", size = 11 } },
				ui.text {
					function(): string
						return head().continued or ""
					end,
					min_w = 0,
					shrink = function(): number
						return if landed:get() then 1 else 0
					end,
					truncate = function(): boolean
						return landed:get()
					end,
				},
			},
		}
	end)())
end

-- The label between the rules: icon, words, the meta, what the chat
-- continued with, a chevron when it opens. `on_toggle`: a landed one that opens.
local function label(head: () -> Head, open: (() -> boolean)?, on_toggle: (() -> ())?, id: string?, still: (() -> boolean)?): Node
	local pieces = {
		gap = ui.raw(7, "gpui's label gaps"),
		items = "center",
		min_w = 0,
		label_icon(function(): string
			return head().icon
		end),
		RollText {
			function(): string
				return head().title
			end,
			shrink = 0,
			text_size = "$font.size.sm",
			color = "$text.secondary",
		},
		ui.show(function(): boolean
			return head().meta ~= nil
		end, RollText {
			function(): string
				return head().meta or ""
			end,
			min_w = 0,
			shrink = function(): number
				return if head().continued then 0 else 1
			end,
			tabular = true,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		}),
		if still then continued(head, still, id) else nil,
		ui.show(function(): boolean
			return head().chevron and open ~= nil
		end, ui.div {
			aria_hidden = true,
			color = "$text.tertiary",
			Icon {
				name = function(): any
					return if open and open() then "lucide-chevron-down" else "chevron-right"
				end,
				size = 12,
			},
		}),
	}
	local box: { [any]: any } = {
		id = id,
		class = "compaction-label",
		px = ui.raw(6, "gpui's compaction card"),
		py = ui.raw(2, "gpui's compaction card"),
		rounded = ui.raw(6, "gpui's compaction card"),
		overflow = "hidden",
		selectable = false,
	}
	for k, v in pieces do
		box[k] = v
	end
	if on_toggle then
		box.role = "button"
		box.label = function(): string
			return M.words_of(head())
		end
		box.expanded = function(): boolean
			return open ~= nil and open()
		end
		box.cursor = "pointer"
		box.focusable = true
		box.hover = { bg = "$hover" }
		box.on_click = on_toggle
		box.on_key = function(e: any): boolean
			if e.key == "Enter" or e.key == " " or e.key == "Space" then
				on_toggle()
				return true
			end
			return false
		end
		return ui.row(box)
	end
	box.role = "status"
	return ui.row(box)
end

local function line(head: () -> Head, live: () -> boolean, fill: State<number>, fading: State<boolean>, open: (() -> boolean)?, on_toggle: (() -> ())?, id: string?, still: (() -> boolean)?): Node
	return ui.row {
		class = "compaction-line",
		w_full = true,
		h = 24,
		gap = "$space.2",
		items = "center",
		rule(true, live, fill, fading),
		label(head, open, on_toggle, id, still),
		rule(false, live, fill, fading),
	}
end

local function field(key: string, value: () -> string?): Node
	return ui.show(function(): boolean
		return value() ~= nil
	end, ui.row {
		gap = ui.raw(14, "the mockup's panel columns"),
		w_full = true,
		items = "start",
		ui.text { s(key), w = 96, shrink = 0, color = "$text.secondary" },
		ui.text {
			function(): string
				return value() or ""
			end,
			literal = true,
			wrap = true,
			selectable = true,
			grow = 1,
			min_w = 0,
			color = "$text.primary",
		},
	})
end

-- Kept across remounts: which panels are open and their heights, so a row built again opens at its height.
local opened = ui.state("compaction_opened", {} :: { [string]: boolean })
local heights = ui.state("compaction_panel_h", {} :: { [string]: number })
local wait_went = ui.state("compaction_wait_went", 0)

local function panel(id: string, m: () -> any, open: () -> boolean): Node
	-- Laid out only from opening until folded away, so a closed one isn't read or found.
	local laid = ui.use_state(open())
	ui.effect(function()
		if open() then
			laid:set(true)
		elseif laid:peek() then
			ui.after(math.max(motion.ms(220), motion.FADE) + 40, function()
				laid:set(false)
			end)
		end
	end)
	return ui.column {
		id = `{id}-panel`,
		w_full = true,
		overflow = "hidden",
		h = motion.tween(ui.computed(function(): number
			return if open() then (heights:get()[id] or 0) + 6 else 0
		end), { duration = 220, ease = motion.sidebar.ease }),
		opacity = if motion.reduced_now()
			then ui.tween(ui.computed(function(): number
				return if open() then 1 else 0
			end), { duration = motion.FADE })
			else nil,
		ui.show(function(): boolean
			return laid:get()
		end, ui.column {
			class = "compaction-panel",
			w_full = true,
			shrink = 0,
			mt = ui.raw(6, "gpui's compaction card"),
			px = ui.raw(12, "the mockup's panel box"),
			py = ui.raw(10, "the mockup's panel box"),
			gap = ui.raw(6, "the mockup's panel rows"),
			rounded = ui.raw(10, "gpui's compaction card"),
			border = 1,
			border_color = "$border.subtle",
			bg = "$surface.raised",
			text_size = "$font.size.sm",
			on_resize = function(size: { w: number, h: number })
				if math.abs((heights:peek()[id] or 0) - size.h) >= 1 then
					heights:set_key(id, size.h)
				end
			end,
			field("focus_label", function(): string?
				return focus_of(m())
			end),
			field("summary_label", function(): string?
				return kept_of(m()).summary
			end),
			field("continued_label", function(): string?
				local c = continued_of(m())
				return c and c.note
			end),
			field("limit_reset_label", function(): string?
				local c = continued_of(m())
				return if c and c.limit then c.at else nil
			end),
		}),
	}
end

-- How many clocks tick now (a test's look at the idle cost).
M.clocks = 0

-- A seconds clock that ticks while it shows.
local function ticking(since: () -> number): () -> number
	local now = ui.use_state(ui.now())
	M.clocks += 1
	ui.on_cleanup(function()
		M.clocks -= 1
	end)
	ui.every(1000, function()
		now:set(ui.now())
	end)
	return function(): number
		return math.max(0, (now:get() - since()) / 1000)
	end
end

local function never(): boolean
	return false
end

M.Divider = ui.component(function(props: { m: () -> any, id: string }): Node
	local id = props.id
	local fill = ui.use_state(0)
	local fading = ui.use_state(false)
	local filled = false
	local function settle()
		if filled or motion.reduced_now() then
			return
		end
		filled = true
		fill:set(1)
		ui.after(450, function()
			fading:set(true)
		end)
		ui.after(900, function()
			fill:set(0)
			fading:set(false)
		end)
	end
	-- A compaction landing as its wait goes fills its rules once, whichever
	-- the host says first: the row (its wait goes after) or the wait's end.
	local born = ui.now()
	local seen = wait_went:peek()
	if born - seen < 5000 then
		ui.after(0, settle)
	end
	ui.effect(function()
		local went = wait_went:get()
		if went ~= seen then
			seen = went
			if went - born < 5000 then
				settle()
			end
		end
	end)
	local function head(): Head
		return M.head(props.m())
	end
	local function is_open(): boolean
		return opened:get()[id] == true and head().chevron
	end
	local function toggle()
		if not head().chevron then
			return
		end
		opened:set_key(id, if is_open() then nil else true)
	end
	local built_with = head().continued ~= nil
	local function still(): boolean
		local was = built_with
		built_with = false
		return was
	end
	return ui.column {
		w_full = true,
		line(head, never, fill, fading, is_open, toggle, `{id}-head`, still),
		panel(id, props.m, is_open),
	}
end)

-- The wait in the trailer's place: queued after the agent's turn (still,
-- no clock), compacting (the rules sweep, the clock ticks).
M.Wait = ui.component(function(props: { wait: () -> Wait? }): Node
	local fill = ui.use_state(0)
	local fading = ui.use_state(false)
	ui.on_cleanup(function()
		wait_went:set(ui.now())
	end)
	-- (Going, it keeps the phase it showed.)
	local shown = "queued"
	local function phase(): string
		local w = props.wait()
		if w then
			shown = w.phase
		end
		return shown
	end
	local function live(): boolean
		return phase() == "compacting"
	end
	local since = ui.use_state(ui.now())
	local last_phase = phase()
	ui.effect(function()
		local p = phase()
		if p ~= last_phase then
			last_phase = p
			since:set(ui.now())
		end
	end)
	return ui.column {
		id = "compaction-wait",
		w_full = true,
		pt = "$space.4",
		ui.presence(function(): string
			return if live() then "live" else "queued"
		end, function(kind: string): Node
			if kind == "queued" then
				return line(function(): Head
					return M.wait_head({ phase = "queued" }, 0)
				end, live, fill, fading)
			end
			return ui.component(function(): Node
				local seconds = ticking(function(): number
					return since:get()
				end)
				return line(function(): Head
					return M.wait_head({ phase = "compacting" }, seconds())
				end, live, fill, fading)
			end)()
		end),
	}
end)

-- The composer's line while its chat compacts: the same word as the
-- divider (gpui's `composer_label`), none while it is only queued.
function M.chip_label(w: Wait?): string?
	return if w and w.phase == "compacting" then s("compacting") else nil
end

function M.Chip(props: { label: () -> string? }): Node
	return ui.presence(function(): string?
		return if props.label() then "chip" else nil
	end, function(): Node
		local last = props.label() or ""
		return ui.row {
			id = "composer-compacting",
			role = "status",
			mx = ui.raw(8, "gpui's compaction card"),
			-- (Padding, not margins: a sized presence holds its content at its border box, so a margin squeezed the line under the frame.)
			pt = ui.raw(6, "gpui's compaction card"),
			pb = ui.raw(8, "gpui's composer gap under it (composer.rs SPACE_SM)"),
			gap = ui.raw(6, "gpui's compaction card"),
			items = "center",
			text_size = "$font.size.sm",
			color = "$text.secondary",
			status.Spinner { size = 12, ink = "$text.secondary" },
			RollText {
				function(): string
					last = props.label() or last
					return last
				end,
			},
		}
	end, { w_full = true, enter = motion.transition { opacity = { 0, 1 }, duration = 150 }, exit = { opacity = 0, duration = 120 } })
end

export type Offer = { tokens: string, idle: string, at: string }

-- "Resume with less context": the prompt cache of a large Claude chat has
-- expired; Compact, or Keep full history (for this quiet spell only).
function M.Resume(props: { offer: () -> Offer?, on_compact: () -> (), on_keep: () -> () }): Node
	return ui.presence(function(): string?
		local o = props.offer()
		return if o then "offer" else nil
	end, function(): Node
		local last = props.offer() :: Offer
		local function now(): Offer
			last = props.offer() or last
			return last
		end
		return ui.column {
			w_full = true,
			px = ui.raw(4, "gpui's compaction card"),
			pt = ui.raw(6, "gpui's compaction card"),
			pb = ui.raw(2, "gpui's compaction card"),
			ui.column {
				id = "composer-resume",
				role = "group",
				label = s("resume_title"),
				px = ui.raw(12, "gpui's compaction card"),
				py = ui.raw(10, "gpui's compaction card"),
				gap = ui.raw(8, "gpui's compaction card"),
				rounded = ui.raw(12, "gpui's compaction card"),
				border = 1,
				border_color = "$border.subtle",
				bg = "$surface.raised",
				ui.row {
					gap = ui.raw(8, "gpui's compaction card"),
					items = "center",
					ui.div { color = "$accent", Icon { name = "collapse-arrows", size = 14 } },
					ui.text { s("resume_title"), text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.primary" },
				},
				ui.text {
					function(): string
						local o = now()
						return s("resume_body", { tokens = o.tokens, idle = o.idle })
					end,
					text_size = "$font.size.sm",
					color = "$text.secondary",
				},
				ui.row {
					gap = ui.raw(6, "gpui's compaction card"),
					Button { s("resume_compact"), id = "composer-resume-compact", tone = "primary", size = "sm", on_click = props.on_compact },
					Button { s("resume_keep"), id = "composer-resume-keep", tone = "ghost", size = "sm", on_click = props.on_keep },
				},
			},
		}
	end, { w_full = true, enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 180, ease = "outCubic" }, exit = { opacity = 0, duration = 120 } })
end

return M
