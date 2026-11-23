-- How full a model's context is, as the composer's ring shows it and its
-- card says it (Zeron's crates/ui/src/context_usage.rs): what the agent
-- reports (tokens in it, the window's size, either missing), how full that
-- is, the ink it reads in (the text's own, amber from three quarters, red
-- from nine tenths), and the card's content.
--
--   context_usage.fraction(u)  0 to 1, or nil while it can't be said
--   context_usage.lines(u)     the card's two lines, as Zeron writes them
--   context_usage.ink(f)       the ring's ink at `f`
--   Card { usage, stats?, working?, plan?, compaction? }  the ring's card;
--                              given the chat's stats (`chat.stats`), its
--                              last turn's rate and output and the prompt
--                              cache's time left; given the account's plan,
--                              its windows; given the chat's compaction
--                              readout, the bar's marks and the next
--                              compaction, with Compact now while idle
--   Chip { usage, stats?, working?, plan?, compaction?, side }  the footer's
--                              one chip: the next compaction's word, ring,
--                              percent, " · ", the cache's clock
--   PlanChip { plan, side }    the plan's window nearest its limit, only
--                              from 80% (PLAN_NEAR)

local locale = require("@self/locale")
local Menu = require("@self/menu")
local HoverCard = require("@self/hover_card")
local progress = require("@self/progress")
local SessionStats = require("@self/session_stats")
local Icon = require("@self/icon")
local ProviderCard = require("@self/provider_card")
local AgentMark = require("@self/agent_mark")
local Button = require("@self/button")
local status = require("@self/status")
local motion = require("@self/motion")
local Tooltip = require("@self/tooltip")
local RollText = require("@self/roll_text")

export type Usage = { tokens: number?, window: number? }
export type Sized = { label: string, n: number, held: boolean }
export type Marks = { floor: number?, warn: number?, auto: number? }
export type Next = { icon: "spin" | "agent" | "clock" | "warn" | "off", title: string, detail: string?, chip: string? }
export type Compaction = {
	marks: () -> Marks?,
	next: () -> Next?,
	on_compact: (() -> ())?,
	busy: (() -> boolean)?,
}
-- The account in use's plan: its agent (the model picker's harness: name
-- and mark), its plan's name, and its windows (each 0 to 1, when it starts
-- over in seconds).
export type Window = { label: string, used: number, resets: number? }
export type Plan = { agent: { id: string, label: string, mark: any?, icon: string? }, plan: string?, windows: { Window } }

local M = {}

-- From here a plan's window shows on the footer (Zeron's account ring turns amber).
M.PLAN_NEAR = 0.8

local NUDGE, NUDGE_MS = 3, 180

-- A count of tokens, its thousands grouped ("408,389").
function M.grouped(n: number): string
	local digits = string.format("%d", math.max(0, math.floor(n + 0.5)))
	local out = string.reverse((string.gsub(string.reverse(digits), "(%d%d%d)", "%1,")))
	return (string.gsub(out, "^,", ""))
end

function M.fraction(u: Usage?): number?
	if u and u.tokens and u.window and u.window > 0 then
		return math.clamp(u.tokens / u.window, 0, 1)
	end
	return nil
end

-- Used of the window and what is left; or what is known of it (tokens
-- with no window, a window with nothing counted yet, nothing at all).
function M.lines(u: Usage?): (string, string?)
	local tokens = if u then u.tokens else nil
	local window = if u then u.window else nil
	if tokens and window and window > 0 then
		return locale.s("ds.composer.context.of", { used = M.grouped(tokens), window = M.grouped(window) }),
			locale.s("ds.composer.context.left", { n = M.grouped(math.max(window - tokens, 0)) })
	elseif tokens then
		return locale.s("ds.composer.context.used_only", { n = M.grouped(tokens) }), locale.s("ds.composer.context.no_limit")
	elseif window and window > 0 then
		return locale.s("ds.composer.context.capacity", { n = M.grouped(window) }), locale.s("ds.composer.context.waiting")
	end
	return locale.s("ds.composer.context.unreported"), nil
end

-- Tokens as gpui's: 212K, 1.0M.
function M.short_tokens(n: number): string
	if n >= 999500 then
		return string.format("%.1fM", n / 1000000)
	end
	return `{math.floor((n + 500) / 1000)}K`
end

function M.ink(f: number): string
	return if f >= 0.9 then "$danger" elseif f >= 0.75 then "$warning" else "$text.secondary"
end

-- The ring itself: a faint track under an arc from twelve o'clock, in the
-- ink of how full it is: 12 across, the chips' glyph size, or gpui's footer
-- ring (`footer`: a 16 box, radius 6, a 1.8 stroke).
function M.Ring(props: { fraction: () -> number, ink: ((f: number) -> string)?, footer: boolean? }): Node
	local ink = props.ink or M.ink
	local size = if props.footer then 16 else 12
	local radius = if props.footer then 6 else 5.25
	local stroke = if props.footer then 1.8 else 1.5
	return ui.canvas {
		w = size,
		h = size,
		shrink = 0,
		draw = function(c, w, h)
			local f = props.fraction()
			c:stroke(ui.path():arc(w / 2, h / 2, radius, 0, 360), "$border.strong", stroke)
			if f > 0 then
				c:stroke(ui.path():arc(w / 2, h / 2, radius, -90, 360 * f), ink(f), stroke, { cap = "round" })
			end
		end,
	}
end

-- A plan's usage, as Zeron's account ring reads it: the accent, so it reads
-- apart from the context ring; amber from four fifths, red from 95%.
function M.usage_ink(f: number): string
	return if f >= 0.95 then "$danger" elseif f >= 0.8 then "$warning" else "$accent"
end

local function tone(f: number): progress.Tone
	return if f >= 0.9 then "danger" elseif f >= 0.75 then "warning" else "neutral"
end

-- The last turn seen ending and the prompt cache (what the session stats
-- chip said, folded in here): the cache counts down once a second while the
-- card shows, and not at all once it hides.
local TurnPart = ui.component(function(props: { stats: () -> SessionStats.Stats?, working: () -> boolean }): Node
	local now = ui.use_state(ui.now())
	local shown = ui.use_state(false)
	local function known(): boolean
		local s = props.stats()
		return s ~= nil and s.cache_ttl ~= nil and s.cache_touched ~= nil
	end
	ui.effect(function()
		if shown:get() and known() then
			now:set(ui.now())
			ui.every(1000, function()
				now:set(ui.now())
			end)
		end
	end)
	local function last(): SessionStats.Last?
		local s = props.stats()
		return if s then s.last else nil
	end
	local function cache(): SessionStats.Cache?
		local s = props.stats()
		return if s then SessionStats.cache_of(s, props.working(), now:get()) else nil
	end
	local function row(label: string, id: string, value: () -> string, shows: () -> boolean): Node
		return ui.show(shows, ui.row {
			h = 22,
			px = "$space.2",
			gap = "$space.2",
			items = "center",
			justify = "between",
			text_size = "$font.size.sm",
			color = "$text.tertiary",
			ui.text { locale.t(label) },
			ui.text { value, id = id, tabular = true, color = "$text.primary" },
		})
	end
	return ui.show(function(): boolean
		return last() ~= nil or known()
	end, ui.column {
		id = "composer-context-turn",
		w_full = true,
		pb = "$space.1",
		on_visible = function(visible: boolean)
			shown:set(visible)
		end,
		ui.div { mx = ui.raw(-4, "the card's inset: the rule runs edge to edge"), mb = "$space.1", Menu.Rule {} },
		row("ds.composer.context.last_turn", "composer-context-turn-rate", function(): string
			local l = last()
			return locale.s("ds.composer.context.tok_s", { n = if l then l.tps or "" else "" })
		end, function(): boolean
			local l = last()
			return l ~= nil and l.tps ~= nil
		end),
		row("ds.composer.context.output", "composer-context-turn-output", function(): string
			local l = last()
			return locale.s("ds.composer.context.output_tokens", { n = if l then l.output else "" })
		end, function(): boolean
			return last() ~= nil
		end),
		row("ds.composer.context.cache", "composer-context-cache", function(): string
			local c = cache()
			return if c then SessionStats.cache_countdown(c) else ""
		end, known),
	})
end)

-- A plan's window as the footer names it (account_usage.rs `window_label`), and as the card's row does.
local SHORT = { Session = "5h", Week = "Weekly", Day = "Daily" }
local LONG = { Session = "ds.composer.context.window_session", ["5h"] = "ds.composer.context.window_session", Week = "ds.composer.context.window_week", Weekly = "ds.composer.context.window_week", Day = "ds.composer.context.window_day", Daily = "ds.composer.context.window_day" }

function M.window_short(label: string): string
	return SHORT[label] or label
end

function M.window_long(label: string): string
	local key = LONG[label]
	return if key then locale.s(key) else label
end

-- The plan's window nearest its limit, from PLAN_NEAR; nil below it.
function M.near(plan: Plan?): Window?
	local most: Window? = nil
	for _, w in if plan then plan.windows else {} do
		if w.used >= M.PLAN_NEAR and (most == nil or w.used > most.used) then
			most = w
		end
	end
	return most
end

local function percent(f: number): string
	return locale.s("ds.composer.context.percent", { n = math.floor(f * 100 + 0.5) })
end

-- The card's plan: "<agent> <plan> · plan", then a row and a meter a window.
local function PlanPart(plan: () -> Plan?): Node
	local function windows(): { Window }
		local p = plan()
		return if p then p.windows else {}
	end
	return ui.show(function(): boolean
		return #windows() > 0
	end, ui.column {
		id = "composer-context-plan",
		w_full = true,
		pb = "$space.1",
		ui.div { mx = ui.raw(-4, "the card's inset: the rule runs edge to edge"), my = "$space.1", Menu.Rule {} },
		ui.row {
			h = "$zeron.menu.heading",
			px = "$space.2",
			items = "center",
			ui.text {
				function(): string
					local p = plan()
					if not p then
						return ""
					end
					local name = if p.plan and p.plan ~= "" then `{p.agent.label} {p.plan}` else p.agent.label
					return locale.s("ds.composer.context.plan", { name = name })
				end,
				text_size = "$font.size.xs",
				weight = "$font.weight.medium",
				color = "$text.tertiary",
			},
		},
		ui.each(ui.computed(windows), function(w: Window): Node
			local resets = ProviderCard.when_words(w.resets, ui.now() / 1000)
			return ui.column {
				w_full = true,
				ui.row {
					h = 22,
					px = "$space.2",
					gap = "$space.2",
					items = "center",
					justify = "between",
					text_size = "$font.size.sm",
					color = "$text.tertiary",
					ui.text { M.window_long(w.label) },
					ui.text {
						if resets then locale.s("ds.composer.context.plan_window", { percent = percent(w.used), when = resets }) else percent(w.used),
						id = `composer-context-plan-{M.window_short(w.label)}`,
						tabular = true,
						color = M.usage_ink(w.used),
					},
				},
				ui.div {
					mx = "$space.2",
					mt = "$zeron.space.hair",
					mb = "$space.1",
					h = 4,
					rounded = ui.raw(2, "the 4 px meter's round ends"),
					overflow = "hidden",
					bg = "$hover",
					ui.div { h = "100%", w = `{math.clamp(w.used, 0, 1) * 100}%`, rounded = ui.raw(2, "the 4 px meter's round ends"), bg = M.usage_ink(w.used) },
				},
			}
		end),
	})
end

local MARKS: { { key: string, words: string, ink: string } } = {
	{ key = "floor", words = "ds.composer.context.mark_floor", ink = "$text.tertiary" },
	{ key = "warn", words = "ds.composer.context.mark_warn", ink = "$text.tertiary" },
	{ key = "auto", words = "ds.composer.context.mark_auto", ink = "$warning" },
}
-- A mark's box: wider than its tick, so the pointer finds its words.
local MARK_W = 9

local Meter = ui.component(function(props: { usage: () -> Usage?, fraction: () -> number, marks: () -> Marks? }): Node
	local track_w = ui.use_state(0)
	local layer: { [any]: any } = {
		relative = true,
		w_full = true,
		h = 10,
		justify = "center",
		on_resize = function(size: { w: number, h: number })
			if math.abs(track_w:peek() - size.w) >= 0.5 then
				track_w:set(size.w)
			end
		end,
		progress.ProgressBar {
			value = props.fraction,
			w_full = true,
			label = locale.t("ds.composer.context.heading"),
			tone = function(): progress.Tone
				return tone(props.fraction())
			end,
		},
	}
	for _, mark in MARKS do
		local function at(): number?
			local m = props.marks()
			local u = props.usage()
			local v = if m then (m :: any)[mark.key] else nil
			if type(v) ~= "number" or not u or not u.window or u.window <= 0 then
				return nil
			end
			return v
		end
		local function words(): string
			return locale.s(mark.words, { n = math.floor((at() or 0) / 1000 + 0.5) })
		end
		table.insert(layer, ui.show(function(): boolean
			return at() ~= nil
		end, ui.div {
			class = `context-mark context-mark-{mark.key}`,
			absolute = true,
			top = 0,
			w = MARK_W,
			h = 10,
			items = "center",
			left = function(): number
				local u = props.usage()
				local f = math.clamp((at() or 0) / (if u and u.window then u.window else 1), 0, 1)
				return f * track_w:get() - MARK_W / 2
			end,
			label = words,
			tooltip = Tooltip { words },
			ui.div { w = 1.5, h = 10, rounded = ui.raw(1, "the mark's round ends"), bg = mark.ink },
		}))
	end
	return ui.column(layer)
end)

local function NextPart(c: Compaction, working: () -> boolean): Node
	local function upcoming(): Next?
		local n = c.next()
		return if n then n else nil
	end
	local function icon(): string
		local n = upcoming()
		return if n then n.icon else "off"
	end
	local GLYPHS = { agent = "lucide-box", clock = "lucide-clock", warn = "danger-triangle", off = "lucide-circle-dashed" }
	return ui.show(function(): boolean
		return upcoming() ~= nil
	end, ui.column {
		id = "composer-context-next",
		w_full = true,
		pb = "$space.1",
		ui.div { mx = ui.raw(-4, "the card's inset: the rule runs edge to edge"), mb = "$space.1", Menu.Rule {} },
		ui.row {
			h = "$zeron.menu.heading",
			px = "$space.2",
			items = "center",
			ui.text { locale.t("ds.composer.context.next"), text_size = "$font.size.xs", weight = "$font.weight.medium", color = "$text.tertiary" },
		},
		ui.row {
			px = "$space.2",
			gap = "$space.2",
			items = "start",
			ui.row {
				w = 14,
				h = "$font.line.sm",
				shrink = 0,
				items = "center",
				justify = "center",
				ui.show(function(): boolean
					return icon() == "spin"
				end, status.Spinner { size = 12, ink = "$text.secondary" }),
				ui.show(function(): boolean
					return icon() ~= "spin"
				end, ui.div {
					color = function(): string
						return if icon() == "warn" then "$warning" else "$text.secondary"
					end,
					Icon {
						name = function(): any
							return GLYPHS[icon()] or GLYPHS.off
						end,
						size = 14,
					},
				}),
			},
			ui.column {
				grow = 1,
				min_w = 0,
				gap = "$zeron.space.hair",
				ui.text {
					function(): string
						local n = upcoming()
						return if n then n.title else ""
					end,
					id = "composer-context-next-title",
					wrap = true,
					text_size = "$font.size.sm",
					line_height = "$font.line.sm",
					weight = "$font.weight.medium",
					color = function(): string
						return if icon() == "off" then "$text.secondary" else "$text.primary"
					end,
				},
				ui.show(function(): boolean
					local n = upcoming()
					return n ~= nil and (n.detail or "") ~= ""
				end, ui.text {
					function(): string
						local n = upcoming()
						return if n then n.detail or "" else ""
					end,
					id = "composer-context-next-detail",
					wrap = true,
					text_size = "$font.size.xs",
					color = "$text.secondary",
				}),
			},
		},
		if c.on_compact
			then ui.show(function(): boolean
				return not working() and not (c.busy and c.busy())
			end, ui.row {
				px = "$space.2",
				pt = "$space.2",
				Button {
					locale.t("ds.composer.context.compact_now"),
					id = "composer-context-compact",
					tone = "secondary",
					size = "xs",
					w_full = true,
					on_click = c.on_compact,
				},
			})
			else nil,
	})
end

function M.Card(props: { usage: () -> Usage?, stats: (() -> SessionStats.Stats?)?, working: (() -> boolean)?, plan: (() -> Plan?)?, compaction: Compaction? }): Node
	local usage = props.usage
	local function fraction(): number?
		return M.fraction(usage())
	end
	local function known(): boolean
		return fraction() ~= nil
	end
	local working = props.working or function(): boolean
		return false
	end
	local compaction = props.compaction
	return HoverCard.Card {
		id = "composer-context-card",
		w = 248,
		p = "$space.1",
		gap = 0,
		ui.row {
			h = "$zeron.menu.heading",
			px = "$space.2",
			gap = "$space.2",
			items = "center",
			ui.text {
				locale.t("ds.composer.context.heading"),
				grow = 1,
				text_size = "$font.size.xs",
				weight = "$font.weight.medium",
				color = "$text.tertiary",
			},
			ui.show(known, ui.text {
				function(): string
					return locale.s("ds.composer.context.percent", { n = math.floor((fraction() or 0) * 100 + 0.5) })
				end,
				tabular = true,
				text_size = "$font.size.xs",
				weight = "$font.weight.medium",
				color = function(): string
					return M.ink(fraction() or 0)
				end,
			}),
		},
		ui.column {
			px = "$space.2",
			pt = "$space.1",
			pb = "$space.2",
			gap = "$space.2",
			ui.show(known, Meter {
				usage = usage,
				fraction = function(): number
					return fraction() or 0
				end,
				marks = if compaction then compaction.marks else function(): Marks?
					return nil
				end,
			}),
			ui.column {
				gap = "$zeron.space.hair",
				ui.text {
					function(): string
						return (M.lines(usage()))
					end,
					id = "composer-context-used",
					tabular = true,
					text_size = "$font.size.sm",
					line_height = "$font.line.sm",
					weight = "$font.weight.medium",
					color = "$text.primary",
				},
				ui.show(function(): boolean
					local _, second = M.lines(usage())
					return second ~= nil
				end, ui.text {
					function(): string
						local _, second = M.lines(usage())
						return second or ""
					end,
					id = "composer-context-left",
					tabular = true,
					text_size = "$font.size.sm",
					line_height = "$font.line.sm",
					color = "$text.tertiary",
				}),
			},
		},
		if compaction then NextPart(compaction, working) else nil,
		if props.stats then TurnPart { stats = props.stats, working = working } else nil,
		if props.plan then PlanPart(props.plan) else nil,
	}
end

-- Kept across remounts, so a footer built again shows the words at once rather than opening them again.
local next_widths = ui.state("context_next_w", {} :: { [string]: number })

local function NextWord(upcoming: () -> Next?): Node
	local last = ""
	local function chip(): string?
		local n = upcoming()
		return if n and (n.chip or "") ~= "" then n.chip else nil
	end
	local function words(): string
		local c = chip()
		if c then
			last = locale.s("ds.composer.context.next_chip", { chip = c })
		end
		return last
	end
	local EASE = { duration = 240, ease = motion.sidebar.ease }
	return ui.row {
		id = "composer-context-next-chip",
		shrink = 0,
		relative = true,
		h = "$font.line.xs",
		overflow = "hidden",
		w = motion.tween(ui.computed(function(): number
			return if chip() then next_widths:get()[words()] or 0 else 0
		end), EASE),
		opacity = ui.tween(ui.computed(function(): number
			return if chip() then 1 else 0
		end), if motion.reduced_now() then { duration = motion.FADE } else EASE),
		-- Out of flow: in-flow text wider than a closing clip is a layout warning.
		ui.text {
			words,
			absolute = true,
			left = 0,
			top = 0,
			line_height = "$font.line.xs",
			shrink = 0,
			pr = "$space.1",
			text_size = "$font.size.xs",
			color = "$text.secondary",
			on_resize = function(size: { w: number, h: number })
				local key = words()
				if key ~= "" and math.abs((next_widths:peek()[key] or 0) - size.w) >= 0.5 then
					next_widths:set_key(key, size.w)
				end
			end,
		},
	}
end

-- The footer's one chip (gpui's `ring_chip`, context_usage.rs): the ring
-- and how full the context is once a window is told, then " · " and the
-- prompt cache's clock once a turn told its TTL. Resting on it (or the keys
-- reaching it) opens the card.
M.Chip = ui.component(function(props: {
	usage: () -> Usage?,
	stats: (() -> SessionStats.Stats?)?,
	working: (() -> boolean)?,
	plan: (() -> Plan?)?,
	compaction: Compaction?,
	side: () -> string,
	sized: State<Sized?>?,
}): Node
	local working = props.working or function(): boolean
		return false
	end
	local word = SessionStats.use_word(props.stats or function(): SessionStats.Stats?
		return nil
	end, working)
	local function windowed(): boolean
		local u = props.usage()
		return u ~= nil and u.window ~= nil and u.window > 0
	end
	local function fraction(): number?
		return M.fraction(props.usage())
	end
	local function sized(): Sized?
		return if props.sized then props.sized:get() else nil
	end
	local rolling = false
	local el: any = nil
	local nudged = 0
	ui.effect(function()
		local s = sized()
		if not s then
			ui.after(0, function()
				rolling = false
			end)
			return
		end
		rolling = true
		if s.held and s.n ~= nudged and el and not motion.reduced_now() then
			anime.animate(el, { x = { NUDGE, 0 }, duration = NUDGE_MS, ease = "outQuint" })
		end
		nudged = s.n
	end)
	local function card(): Node
		return M.Card { usage = props.usage, stats = props.stats, working = props.working, plan = props.plan, compaction = props.compaction }
	end
	return ui.show(function(): boolean
		return windowed() or word:get() ~= nil or sized() ~= nil
	end, ui.presence(props.side, function(at: string): Node
		local chip = ui.row {
			id = "composer-context",
			on_mount = function(mounted)
				el = mounted
			end,
			h = 24,
			px = "$zeron.space.tight",
			gap = ui.raw(5, "gpui's ring_chip gap"),
			items = "center",
			shrink = 0,
			rounded = "$radius.lg",
			hover = { bg = "$hover" },
			focusable = true,
			selectable = false,
			text_size = "$font.size.xs",
			label = locale.t("ds.composer.context_used"),
			tooltip = card(),
			tooltip_placement = { side = at, align = "end", offset = 8 },
			tooltip_delay = 150,
			tooltip_interactive = props.compaction ~= nil,
			ui.show(windowed, M.Ring {
				footer = true,
				fraction = function(): number
					return fraction() or 0
				end,
			}),
			ui.show(function(): boolean
				return fraction() ~= nil or sized() ~= nil
			end, ui.row {
				id = "composer-context-percent",
				shrink = 0,
				RollText {
					function(): string
						local s, u = sized(), props.usage()
						if s then
							return if u and u.tokens then `{M.short_tokens(u.tokens)} / {s.label}` else s.label
						end
						return percent(fraction() or 0)
					end,
					text_size = "$font.size.xs",
					tabular = true,
					rolls = function(): boolean
						return rolling or sized() ~= nil
					end,
					color = function(): string
						local u = props.usage()
						if sized() and u and u.tokens and u.window and u.tokens > u.window then
							return "$warning"
						end
						return M.ink(fraction() or 0)
					end,
				},
			}),
			ui.show(function(): boolean
				return windowed() and word:get() ~= nil
			end, ui.text { "·", color = "$text.tertiary", opacity = 0.5 }),
			ui.show(function(): boolean
				return word:get() ~= nil
			end, ui.row {
				gap = "$zeron.space.hair",
				items = "center",
				color = function(): string
					local w = word:get()
					return if w then w.tone else "$text.tertiary"
				end,
				label = function(): string
					local w = word:get()
					return locale.s("ds.session_stats.cache", { left = if w then w.long else "" })
				end,
				ui.div { shrink = 0, Icon { name = "clock-circle", size = 11 } },
				ui.text {
					function(): string
						local w = word:get()
						return if w then w.short else ""
					end,
					id = "composer-context-clock",
					tabular = true,
				},
			}),
		}
		local compaction = props.compaction
		if not compaction then
			return chip
		end
		return ui.row { items = "center", shrink = 0, NextWord(compaction.next), chip }
	end))
end)

-- The plan nearing its limit, left of the context chip: the agent's mark and
-- "5h 82%" in the plan's ink. Below PLAN_NEAR, nothing. It opens the same card.
function M.PlanChip(props: { plan: () -> Plan?, card: Node, side: () -> string }): Node
	return ui.show(function(): boolean
		return M.near(props.plan()) ~= nil
	end, ui.presence(props.side, function(at: string): Node
		return ui.row {
			id = "composer-plan",
			h = 24,
			px = "$zeron.space.tight",
			gap = ui.raw(5, "gpui's ring_chip gap"),
			items = "center",
			shrink = 0,
			rounded = "$radius.lg",
			hover = { bg = "$hover" },
			focusable = true,
			selectable = false,
			text_size = "$font.size.xs",
			color = function(): string
				local w = M.near(props.plan())
				return M.usage_ink(if w then w.used else 0)
			end,
			label = function(): string
				local p = props.plan()
				return locale.s("ds.composer.context.plan", { name = if p then p.agent.label else "" })
			end,
			tooltip = props.card,
			tooltip_placement = { side = at, align = "end", offset = 8 },
			tooltip_delay = 150,
			tooltip_interactive = true,
			ui.presence(function(): string?
				local p = props.plan()
				return if p then p.agent.id else nil
			end, function(): Node
				local a = (props.plan() :: Plan).agent
				return AgentMark { label = a.label, seed = a.id, mark = if a.icon then nil else a.mark, icon = a.icon, size = 14, bare = true }
			end, { shrink = 0 }),
			ui.text {
				function(): string
					local w = M.near(props.plan())
					return if w then `{M.window_short(w.label)} {percent(w.used)}` else ""
				end,
				id = "composer-plan-word",
				tabular = true,
			},
		}
	end))
end

return M
