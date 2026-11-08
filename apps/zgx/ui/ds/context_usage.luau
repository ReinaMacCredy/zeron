-- How full a model's context is, as the composer's ring shows it and its
-- card says it (Zeron's crates/ui/src/context_usage.rs): what the agent
-- reports (tokens in it, the window's size, either missing), how full that
-- is, the ink it reads in (the text's own, amber from three quarters, red
-- from nine tenths), and the card's content.
--
--   context_usage.fraction(u)  0 to 1, or nil while it can't be said
--   context_usage.lines(u)     the card's two lines, as Zeron writes them
--   context_usage.ink(f)       the ring's ink at `f`
--   Body { usage = fn }        "Context window" and the lines, followed as
--                              the usage changes
--   Card { usage, stats?, working? }  the ring's card; given the chat's
--                              stats (`chat.stats`), its last turn's rate and
--                              output and the prompt cache's time left

local locale = require("@self/locale")
local Menu = require("@self/menu")
local HoverCard = require("@self/hover_card")
local progress = require("@self/progress")
local SessionStats = require("@self/session_stats")

export type Usage = { tokens: number?, window: number? }

local M = {}

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

function M.Body(props: { usage: () -> Usage?, more: Node? }): Node
	local usage = props.usage
	return ui.column {
		id = "composer-context-card",
		w = 248,
		p = "$space.1",
		gap = 0,
		Menu.CardHeading { locale.t("ds.composer.context.heading") },
		ui.column {
			px = "$space.2",
			pb = "$zeron.space.tight",
			ui.text {
				function(): string
					return (M.lines(usage()))
				end,
				id = "composer-context-used",
				tabular = true,
				text_size = "$font.size.sm",
				line_height = 19,
				color = "$text.secondary",
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
				line_height = 19,
				color = "$text.secondary",
			}),
		},
		-- (The compaction's part, under the numbers: ds/compaction's CardPart.)
		props.more,
	}
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

function M.Card(props: { usage: () -> Usage?, stats: (() -> SessionStats.Stats?)?, working: (() -> boolean)? }): Node
	local usage = props.usage
	local function fraction(): number?
		return M.fraction(usage())
	end
	local function known(): boolean
		return fraction() ~= nil
	end
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
			ui.show(known, progress.ProgressBar {
				value = fraction,
				size = "thin",
				w_full = true,
				label = locale.t("ds.composer.context.heading"),
				tone = function(): progress.Tone
					return tone(fraction() or 0)
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
		if props.stats
			then TurnPart {
				stats = props.stats,
				working = props.working or function(): boolean
					return false
				end,
			}
			else nil,
	}
end

return M
