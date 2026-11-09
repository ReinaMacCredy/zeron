-- A chat's compactions as Zeron's transcript tells them (crates/ui/src/
-- transcript/compaction_card.rs): a divider whose label follows one
-- compaction through, queued, checking (the classifier's agent and the
-- seconds so far), compacting (Carry over or New task, the context, the
-- time against the usual), then what came of it (Compacted, pre to post and
-- how long; Not compacted, kept or no answer; in the background, Swapped
-- in). A settled label opens a panel under it: what the agent kept and the
-- classifier's view, what it saw and answered. Tokens, never dollars.
--
--   Divider { m = fn, live = fn }  a classifier's or a compaction's row
--                                  (`live`: the wait, while this row is the
--                                  one compacting)
--   Wait { wait = fn }             the wait in the trailer's place
--   Chip { label = fn }            the composer's line while it checks or
--                                  compacts
--   Resume { offer = fn, ... }     "Resume with less context", over the
--                                  composer of a large idle Claude chat
--   head(m, wait, elapsed)         the label's pieces, as drawn
--   words(m)                       a settled label's words (what Find finds)
--
-- Motion: the rules sweep the accent while a phase runs (a 1400 ms loop
-- drawn at 30 frames a second, only while it shows), fill green and fade
-- once it settles (320 ms, then a 400 ms fade); the icon crossfades and the
-- words roll as the phase changes; a running phase's clock ticks once a
-- second. Settled and still, nothing draws.

local Icon = require("@self/icon")
local Mark = require("@self/mark")
local RollText = require("@self/roll_text")
local Button = require("@self/button")
local status = require("@self/status")
local motion = require("@self/motion")
local locale = require("@self/locale")

local M = {}

export type Kept = { pre: number?, post: number?, ms: number?, summary: string?, carried: string?, turns: number? }
export type Decision = {
	verdict: "new" | "carry" | "continue" | "failed",
	said: string?,
	by: string,
	ms: number,
	context: number,
	input: string,
	reply: string,
	stage2: string?,
	tokens_in: number?,
	tokens_out: number?,
	reason: string?,
	mark: string?,
	agent: string?,
	model: string?,
}
export type Wait = {
	phase: "queued" | "checking" | "compacting",
	agent_note: boolean?,
	mark: string?,
	agent: string?,
	model: string?,
	tag: ("carry" | "new")?,
	tokens: number?,
	row: string?,
	usual: number?,
}
export type Head = {
	icon: "spinner" | "done" | "kept" | "alert" | "decision" | "queued",
	title: string,
	tag: string?,
	ink: string?,
	meta: string?,
	chip: { mark: string?, name: string }?,
	chevron: boolean,
	live: boolean,
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

local function verdict_label(v: string): string
	return s(`verdict.{v}`)
end

local function asked(by: string): boolean
	return by == "stage1" or by == "stage2"
end

-- The verdict's tag and its ink: Swapped in for a copy; routine answers in
-- their own colour, the rest (not asked, no answer) in the warning's.
local function tag_of(d: Decision): (string, string)
	if d.by == "background_copy" then
		return s("swapped_in"), "$warning"
	end
	local routine = asked(d.by) or d.by == "turn_end"
	if not routine or d.verdict == "failed" then
		return verdict_label(d.verdict), "$warning"
	end
	local ink = if d.verdict == "new" then "$success" elseif d.verdict == "carry" then "$accent" else "$text.secondary"
	return verdict_label(d.verdict), ink
end

local function took(seconds: number, usual: number?): string
	local e = M.elapsed(math.max(0, math.floor(seconds)))
	return if usual then s("usually", { elapsed = e, usual = M.elapsed(usual) }) else e
end

local function sizes(k: Kept, context: number?): string
	local pre = context or k.pre
	local out = if pre then `· {M.count(pre)}` else ""
	if pre and k.post then
		out ..= ` → {M.count(k.post)}`
	end
	return out
end

local function has_kept(k: Kept?): boolean
	return k ~= nil and (k.summary ~= nil or k.carried ~= nil)
end

-- What the label says of a row (`wait`: the compaction it shows, live;
-- `elapsed`: seconds into it).
function M.head(m: any, wait: Wait?, elapsed: number?): Head
	local k: Kept? = m.kept
	if m.line == "compaction" then
		local kept = k or {}
		local meta: string? = nil
		if kept.pre then
			meta = sizes(kept, nil)
			if kept.ms then
				meta ..= ` · {M.elapsed(math.floor(kept.ms / 1000))}`
			end
		end
		local title = if meta == nil and (m.text or "") ~= "" then m.text else s("compacted")
		return { icon = "done", title = title, meta = meta, chevron = has_kept(k), live = false }
	end
	-- (An older engine's row with no record: its own words.)
	if not m.decision then
		return { icon = "decision", title = if (m.text or "") ~= "" then m.text else s("classifier"), chevron = false, live = false }
	end
	local d: Decision = m.decision
	local tag, ink = tag_of(d)
	local context = if k and k.pre then k.pre else d.context
	if wait and wait.phase == "compacting" and not k and (d.verdict == "new" or d.verdict == "carry") then
		return {
			icon = "spinner",
			title = s("compacting"),
			tag = tag,
			ink = ink,
			meta = `· {M.count(context)} · {took(elapsed or 0, wait.usual)}`,
			chevron = false,
			live = true,
		}
	end
	if k then
		local copy = d.by == "background_copy"
		local meta = sizes(k, context)
		if copy then
			local turns = k.turns or 0
			meta ..= ` · {if turns == 0 then s("nothing_carried") else s("turns_as_text", { n = turns })}`
		elseif k.ms then
			meta ..= ` · {M.elapsed(math.floor(k.ms / 1000))}`
		end
		return { icon = "done", title = if copy then s("background") else s("compacted"), tag = tag, ink = ink, meta = meta, chevron = true, live = false }
	end
	if d.verdict == "continue" then
		return { icon = "kept", title = s("not_compacted"), tag = tag, ink = ink, meta = `· {M.count(context)} · {s("context_kept")}`, chevron = true, live = false }
	elseif d.verdict == "failed" then
		return { icon = "alert", title = s("not_compacted"), tag = tag, ink = ink, meta = `· {M.count(context)} · {s("no_answer")}`, chevron = true, live = false }
	end
	return {
		icon = "decision",
		title = s("classifier"),
		tag = tag,
		ink = ink,
		meta = `· {M.count(context)} · {string.format("%.1fs", d.ms / 1000)}`,
		chevron = true,
		live = false,
	}
end

-- What the wait in the trailer's place says.
function M.wait_head(w: Wait, elapsed: number): Head
	if w.phase == "queued" then
		return { icon = "queued", title = s("queued"), meta = s("queued_meta"), chevron = false, live = false }
	elseif w.phase == "checking" then
		return {
			icon = "spinner",
			title = if w.agent_note then s("checking_note") else s("checking"),
			chip = { mark = w.mark, name = w.model or w.agent or "" },
			meta = `· {M.elapsed(math.floor(elapsed))}`,
			chevron = false,
			live = true,
		}
	end
	local tag = if w.tag then verdict_label(w.tag) else nil
	local meta = if w.tokens then `· {M.count(w.tokens)} · {took(elapsed, w.usual)}` else `· {took(elapsed, w.usual)}`
	return { icon = "spinner", title = s("compacting"), tag = tag, ink = "$accent", meta = meta, chevron = false, live = true }
end

function M.words(m: any): string
	local h = M.head(m, nil, nil)
	local out = h.title
	if h.tag then
		out ..= ` {h.tag}`
	end
	if h.meta then
		out ..= ` {h.meta}`
	end
	return out
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
	alert = { name = "lucide-circle-alert", ink = "$warning" },
	decision = { name = "lucide-list-filter", ink = "$text.secondary" },
	queued = { name = "lucide-circle-dashed", ink = "$text.tertiary" },
}

local function glyph(kind: string): Node
	if kind == "spinner" then
		return status.Spinner { size = 13, ink = "$text.secondary" }
	elseif kind == "kept" then
		return ui.text { "–", color = "$text.secondary", text_size = "$font.size.sm" }
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
			enter = { opacity = { 0, 1 }, scale = { 0.6, 1 }, duration = 200, ease = motion.EASE },
			exit = { opacity = 0, scale = 0.6, duration = 200 },
		}),
	}
end

-- The label between the rules: icon, words, the agent's chip, the tag, the
-- meta, a chevron when it opens. `on_toggle`: a settled one that opens.
local function label(head: () -> Head, open: (() -> boolean)?, on_toggle: (() -> ())?, id: string?): Node
	local function tag(): string?
		return head().tag
	end
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
		ui.presence(function(): string?
			local c = head().chip
			return if c then `{c.mark or ""}:{c.name}` else nil
		end, function(): Node
			local c = head().chip :: any
			return ui.row {
				gap = ui.raw(4, "gpui's compaction card"),
				items = "center",
				shrink = 0,
				text_size = "$font.size.xs",
				color = "$text.secondary",
				if c.mark then ui.div { aria_hidden = true, Mark { name = c.mark, size = 13, bare = true } } else nil,
				ui.text { c.name, literal = true },
			}
		end, { enter = { opacity = { 0, 1 }, duration = 200 }, exit = { opacity = 0, duration = 120 } }),
		ui.presence(tag, function(): Node
			local function ink(): string
				return head().ink or "$accent"
			end
			return ui.div {
				class = "compaction-tag",
				relative = true,
				shrink = 0,
				px = ui.raw(8, "gpui's compaction card"),
				py = ui.raw(1, "gpui's compaction card"),
				rounded = "$radius.full",
				text_size = "$font.size.xs",
				color = ink :: any,
				ui.div {
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					bottom = 0,
					rounded = "$radius.full",
					border = 1,
					border_color = ink :: any,
					opacity = ui.raw(0.4, "gpui's tag edge"),
					pointer_events = "none",
				},
				ui.text {
					function(): string
						return tag() or ""
					end,
				},
			}
		end, { enter = { opacity = { 0, 1 }, duration = 200 }, exit = { opacity = 0, duration = 120 } }),
		ui.show(function(): boolean
			return head().meta ~= nil
		end, RollText {
			function(): string
				return head().meta or ""
			end,
			min_w = 0,
			tabular = true,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		}),
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
		shrink = 0,
		max_w = "80%",
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
			local h = head()
			return M.words_of(h)
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

function M.words_of(h: Head): string
	local out = h.title
	if h.tag then
		out ..= ` {h.tag}`
	end
	if h.meta then
		out ..= ` {h.meta}`
	end
	return out
end

local function line(head: () -> Head, live: () -> boolean, fill: State<number>, fading: State<boolean>, open: (() -> boolean)?, on_toggle: (() -> ())?, id: string?): Node
	return ui.row {
		class = "compaction-line",
		w_full = true,
		h = 24,
		gap = "$space.2",
		items = "center",
		rule(true, live, fill, fading),
		label(head, open, on_toggle, id),
		rule(false, live, fill, fading),
	}
end

-- A section of the open panel: its small heading over its fields.
local function section(title: string, children: { Node }, first: boolean?): Node
	local col: { [any]: any } = {
		w_full = true,
		gap = ui.raw(6, "gpui's compaction card"),
		pl = ui.raw(14, "gpui's compaction card"),
		pr = ui.raw(12, "gpui's compaction card"),
		pt = ui.raw(8, "gpui's compaction card"),
		pb = ui.raw(10, "gpui's compaction card"),
		-- (The first's top edge is the frame's own.)
		border_sides = if first then nil else { 1, 0, 0, 0 },
		border_color = "$border.subtle",
		text_size = "$font.size.sm",
		ui.text { title, text_size = "$font.size.xs", weight = "$font.weight.semibold", color = "$text.secondary" },
	}
	for _, c in children do
		table.insert(col, c)
	end
	return ui.column(col)
end

local function text_box(text: string, mono: boolean?, edge: string?): Node
	-- (A long one scrolls in gpui's height; a short one is as tall as it is.)
	local _, breaks = string.gsub(text, "\n", "")
	local long = breaks >= 10 or #text > 1200
	local box = if long then ui.scroll else ui.column
	return box {
		w_full = true,
		h = if long then (if mono then 224 else 240) else nil,
		ui.text {
			text,
			literal = true,
			wrap = true,
			selectable = true,
			font = if mono then "$font.mono" else nil,
			text_size = if mono then "$font.size.xs" else "$font.size.sm",
			color = if mono then "$text.tertiary" else "$text.primary",
		},
		pl = ui.raw(10, "gpui's text box inset"),
		pr = if mono then ui.raw(10, "gpui's mono box") else 0,
		py = if mono then ui.raw(10, "gpui's mono box") else 0,
		border_sides = if mono then nil else { 0, 0, 0, 2 },
		border = if mono then 1 else nil,
		rounded = if mono then ui.raw(8, "gpui's mono box") else nil,
		border_color = edge or "$border.subtle",
	}
end

local function sub(text: string): Node
	return ui.text { text, pt = ui.raw(4, "gpui's compaction card"), color = "$text.secondary" }
end

local function field(key: string, value: string): Node
	return ui.row {
		gap = ui.raw(10, "gpui's compaction card"),
		w_full = true,
		items = "start",
		ui.text { key, w = 96, shrink = 0, color = "$text.secondary" },
		ui.text { value, literal = true, wrap = true, grow = 1, min_w = 0, color = "$text.primary" },
	}
end

function M.kept_section(k: Kept?): Node?
	if not has_kept(k) then
		return nil
	end
	local kept = k :: Kept
	local children: { Node } = {}
	if kept.summary then
		table.insert(children, ui.column { gap = ui.raw(4, "gpui's compaction card"), w_full = true, sub(s("summary_label")), text_box(kept.summary) })
	end
	if kept.carried then
		table.insert(children, ui.column { gap = ui.raw(4, "gpui's compaction card"), w_full = true, sub(s("carried_label")), text_box(kept.carried) })
	else
		table.insert(children, sub(s("nothing_carried_long")))
	end
	return section(s("kept_heading"), children, true)
end

local function classifier_section(d: Decision, saw_open: State<boolean>, first: boolean): Node
	local decision = verdict_label(d.verdict)
	if d.said then
		decision = s("said", { verdict = decision, said = verdict_label(d.said) })
	end
	local by = s(`by.{d.by}`)
	local model = if d.agent and d.model then `{d.agent} · {d.model}` else d.model or d.agent
	local decided = if asked(d.by) and model then `{model} · {by} · {string.format("%.1fs", d.ms / 1000)}` else by
	local reason = d.reason or (if not asked(d.by) then by else nil)
	local children: { Node } = { field(s("decision"), decision) }
	if reason then
		table.insert(children, field(s("reason"), reason))
	end
	table.insert(children, field(s("decided_by"), decided))
	table.insert(children, field(s("context_label"), s("context_tokens", { n = M.count(d.context) })))
	if d.tokens_in and d.tokens_out then
		table.insert(children, field(s("tokens"), s("usage", { input = M.count(d.tokens_in), output = M.count(d.tokens_out) })))
	end
	if d.input ~= "" then
		table.insert(children, ui.button {
			class = "compaction-saw",
			label = if asked(d.by) then s("saw") else s("would_see"),
			expanded = function(): boolean
				return saw_open:get()
			end,
			self = "start",
			pt = ui.raw(4, "gpui's compaction card"),
			gap = ui.raw(4, "gpui's compaction card"),
			items = "center",
			bg = "#00000000",
			hover = { color = "$text.primary", bg = "#00000000" },
			active = { bg = "#00000000" },
			color = "$text.secondary",
			on_click = function()
				saw_open:set(not saw_open:peek())
			end,
			Icon {
				name = function(): any
					return if saw_open:get() then "lucide-chevron-down" else "chevron-right"
				end,
				size = 11,
			},
			ui.text { if asked(d.by) then s("saw") else s("would_see") },
		})
		table.insert(children, ui.show(function(): boolean
			return saw_open:get()
		end, ui.column {
			gap = ui.raw(10, "gpui's compaction card"),
			w_full = true,
			text_box(d.input, true),
			if not asked(d.by) then nil
				elseif d.stage2 then ui.row {
					gap = ui.raw(10, "gpui's compaction card"),
					w_full = true,
					items = "start",
					ui.column { grow = 1, min_w = 0, gap = ui.raw(6, "gpui's compaction card"), ui.text { s("first_look"), color = "$text.secondary" }, text_box(d.reply, true, if d.by == "stage1" then "$accent" else nil) },
					ui.column { grow = 1, min_w = 0, gap = ui.raw(6, "gpui's compaction card"), ui.text { s("second_look"), color = "$text.secondary" }, text_box(d.stage2, true, if d.by == "stage2" then "$accent" else nil) },
				}
				else ui.column { gap = ui.raw(6, "gpui's compaction card"), w_full = true, ui.text { s("reply"), color = "$text.secondary" }, text_box(d.reply, true) },
		}))
	end
	return section(s("classifier_heading"), children, first)
end

local function panel(children: { Node? }): Node
	local col: { [any]: any } = {
		class = "compaction-panel",
		w_full = true,
		mt = ui.raw(6, "gpui's compaction card"),
		rounded = ui.raw(10, "gpui's compaction card"),
		border = 1,
		border_color = "$border.subtle",
		bg = "$surface.raised",
	}
	local inner: { [any]: any } = { w_full = true }
	for _, c in children do
		if c then
			table.insert(inner, c)
		end
	end
	table.insert(col, ui.column(inner))
	return ui.column(col)
end

-- Which panels the person opened, kept while the chat is shown again
-- (a click elsewhere never folds one).
local opened = ui.state("compaction_opened", {} :: { [string]: boolean })
local saws = ui.state("compaction_saw", {} :: { [string]: boolean })
-- When a wait last went (the compaction it showed is landing).
local wait_went = 0

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

M.Divider = ui.component(function(props: { m: () -> any, live: (() -> Wait?)?, id: string }): Node
	local id = props.id
	local fill = ui.use_state(0)
	local fading = ui.use_state(false)
	local function live_wait(): Wait?
		local w = if props.live then props.live() else nil
		return if w and w.phase == "compacting" and w.row == id then w else nil
	end
	local function live(): boolean
		local w = live_wait()
		return w ~= nil and props.m().kept == nil
	end
	local function settle()
		if motion.reduced_now() then
			return
		end
		fill:set(1)
		ui.after(450, function()
			fading:set(true)
		end)
		ui.after(900, function()
			fill:set(0)
			fading:set(false)
		end)
	end
	-- A compaction landing just as its wait went (or as this row stopped
	-- compacting) fills its rules once.
	if props.m().line == "compaction" and ui.now() - wait_went < 5000 then
		ui.after(0, settle)
	end
	local was_live = live()
	ui.effect(function()
		local now = live()
		if was_live and not now then
			settle()
		end
		was_live = now
	end)
	local since = ui.use_state(ui.now())
	ui.effect(function()
		if live() then
			since:set(ui.now())
		end
	end)
	local function head(): Head
		local w = live_wait()
		return M.head(props.m(), w, if w then (ui.now() - since:peek()) / 1000 else nil)
	end
	local function is_open(): boolean
		return opened:get()[id] == true and not live()
	end
	local saw_open = ui.computed(function(): boolean
		return saws:get()[id] == true
	end)
	local saw_state = {
		get = function()
			return saw_open:get()
		end,
		peek = function()
			return saw_open:peek()
		end,
		set = function(_, v: boolean)
			saws:set_key(id, v or nil)
		end,
	}
	local function toggle()
		if live() or not head().chevron then
			return
		end
		opened:set_key(id, if is_open() then nil else true)
	end
	return ui.column {
		w_full = true,
		ui.presence(function(): string
			return if live() then "live" else "settled"
		end, function(kind: string): Node
			if kind == "live" then
				return ui.component(function(): Node
					local seconds = ticking(function(): number
						return since:get()
					end)
					return line(function(): Head
						return M.head(props.m(), live_wait(), seconds())
					end, live, fill, fading, nil, nil, `{id}-head`)
				end)()
			end
			return line(head, live, fill, fading, is_open, toggle, `{id}-head`)
		end),
		ui.presence(function(): string?
			return if is_open() then "open" else nil
		end, function(): Node
			local m = props.m()
			local kept = M.kept_section(m.kept)
			return panel {
				kept,
				if m.decision then classifier_section(m.decision, saw_state :: any, kept == nil) else nil,
			}
		end, {
			w_full = true,
			enter = { opacity = { 0, 1 }, y = { -4, 0 }, duration = 150, ease = motion.EASE },
			exit = { opacity = 0, duration = 120 },
		}),
	}
end)

-- The wait in the trailer's place: queued after the agent's turn (still,
-- no clock), checking or compacting (the rules sweep, the clock ticks).
M.Wait = ui.component(function(props: { wait: () -> Wait? }): Node
	local fill = ui.use_state(0)
	local fading = ui.use_state(false)
	ui.on_cleanup(function()
		wait_went = ui.now()
	end)
	local function phase(): string
		local w = props.wait()
		return if w then w.phase else "queued"
	end
	local function live(): boolean
		return phase() ~= "queued"
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
					local w = props.wait() or { phase = "queued" } :: any
					return M.wait_head(w, 0)
				end, live, fill, fading)
			end
			return ui.component(function(): Node
				local seconds = ticking(function(): number
					return since:get()
				end)
				return line(function(): Head
					local w = props.wait() or { phase = "compacting" } :: any
					return M.wait_head(w, seconds())
				end, live, fill, fading)
			end)()
		end),
	}
end)

-- The composer's line while its chat checks or compacts: the same words as
-- the divider (gpui's `composer_label`), the compaction writing nothing
-- until it lands.
function M.chip_label(w: Wait?): string?
	if not w or w.phase == "queued" then
		return nil
	elseif w.phase == "compacting" then
		return s("compacting")
	end
	return if w.agent_note then s("checking_note") else s("checking")
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
			mt = ui.raw(6, "gpui's compaction card"),
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
	end, { w_full = true, enter = { opacity = { 0, 1 }, duration = 150 }, exit = { opacity = 0, duration = 120 } })
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
			id = "composer-resume",
			role = "group",
			label = s("resume_title"),
			mx = ui.raw(4, "gpui's compaction card"),
			mt = ui.raw(6, "gpui's compaction card"),
			mb = ui.raw(2, "gpui's compaction card"),
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
		}
	end, { w_full = true, enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 180, ease = "outCubic" }, exit = { opacity = 0, duration = 120 } })
end

-- The footer's chip while a copy of the chat compacts in the background
-- (gpui's pickers.rs `copy_chip`): its clock while it runs (once a second),
-- then why it hasn't swapped in yet. Warning-toned.
function M.copy_label(copy: any, now_s: number, working: boolean): string?
	if not copy then
		return nil
	elseif not copy.ready then
		local t = math.max(0, math.floor(now_s - copy.since))
		return s("copy_running", { clock = string.format("%d:%02d", t // 60, t % 60) })
	end
	return if working then s("copy_ready_turn") else s("copy_ready_background")
end

M.CopyChip = ui.component(function(props: { plan: () -> any, working: () -> boolean }): Node
	local now = ui.use_state(ui.now())
	local function copy(): any
		local p = props.plan()
		return if p then p.copy or nil else nil
	end
	ui.effect(function()
		local c = copy()
		if c and not c.ready then
			now:set(ui.now())
			ui.every(1000, function()
				now:set(ui.now())
			end)
		end
	end)
	return ui.show(function(): boolean
		return copy() ~= nil
	end, ui.row {
		id = "composer-copy-chip",
		shrink = 0,
		h = 20,
		px = ui.raw(7, "gpui's copy chip"),
		gap = ui.raw(5, "gpui's copy chip"),
		items = "center",
		rounded = "$radius.md",
		bg = "$zeron.tone.warning.wash",
		color = "$warning",
		text_size = "$font.size.xs",
		Icon {
			name = function(): any
				local c = copy()
				return if c and c.ready then "lucide-check" else "lucide-circle-dashed"
			end,
			size = 12,
		},
		ui.text {
			function(): string
				return M.copy_label(copy(), now:get() / 1000, props.working()) or ""
			end,
			tabular = true,
		},
	})
end)

return M
