-- The countdown stays in Luau: a host action answers no script synchronously.

local Icon = require("@self/icon")
local FadeText = require("@self/fade_text")
local HoverCard = require("@self/hover_card")
local locale = require("@self/locale")
local Menu = require("@self/menu")

export type Stats = {
	turns: number,
	steps: number,
	llm: string,
	tool: string,
	ttft: string?,
	tps: string?,
	hit: string?,
	cache_ttl: number?,
	cache_touched: number?,
	headline: string?,
	priced: boolean,
	unpriced: number,
	estimated: boolean,
	total: string,
	total_n: number,
	split: { { label: string, n: number, tokens: string } },
}

export type Cache =
	{ kind: "refreshing", ttl: number }
	| { kind: "warm", left: number, ttl: number }
	| { kind: "cold", ago: number }

local M = {}

function M.cache_of(s: Stats, working: boolean, now_ms: number): Cache?
	local ttl, touched = s.cache_ttl, s.cache_touched
	if not ttl or not touched then
		return nil
	end
	local left = touched + ttl * 1000 - now_ms
	if working then
		return { kind = "refreshing", ttl = ttl }
	elseif left > 0 then
		return { kind = "warm", left = left, ttl = ttl }
	end
	return { kind = "cold", ago = -left }
end

local function tier(ttl: number): string
	return if ttl % 3600 == 0 then `{ttl // 3600}h` else `{ttl // 60}m`
end

local UNITS = { { 365 * 86400, "y" }, { 30 * 86400, "mo" }, { 86400, "d" }, { 3600, "h" }, { 60, "m" }, { 1, "s" } }

function M.duration(ms: number): string
	if ms < 60000 then
		return string.format("%.1fs", ms / 1000)
	end
	local secs = ms // 1000
	for i, unit in UNITS do
		if secs >= unit[1] then
			local next_unit = UNITS[i + 1]
			return string.format("%d%s%02d%s", secs // unit[1], unit[2], (secs % unit[1]) // next_unit[1], next_unit[2])
		end
	end
	return ""
end

function M.cache_short(c: Cache): string
	if c.kind == "refreshing" then
		return tier(c.ttl)
	elseif c.kind == "cold" then
		return locale.s("ds.session_stats.cold")
	elseif c.left >= 60000 then
		return `{c.left // 60000}m`
	end
	return `{c.left // 1000}s`
end

function M.cache_long(c: Cache): string
	if c.kind == "refreshing" then
		return locale.s("ds.session_stats.refreshing", { tier = tier(c.ttl) })
	elseif c.kind == "cold" then
		return locale.s("ds.session_stats.expired", { ago = M.duration(c.ago) })
	end
	return locale.s("ds.session_stats.left", { left = M.duration(c.left), tier = tier(c.ttl) })
end

function M.running_low(c: Cache): boolean
	return c.kind == "warm" and c.left * 5 < c.ttl * 1000
end

-- gpui's chip looks every ten seconds, so under a minute the word steps by ten.
function M.next_change(c: Cache): number?
	if c.kind ~= "warm" then
		return nil
	elseif c.left >= 60000 then
		return c.left % 60000 + 1
	end
	return math.min(10000, c.left + 1)
end

function M.chip_label(s: Stats): string?
	if s.turns == 0 and s.steps == 0 then
		return nil
	end
	local label = locale.s("ds.session_stats.chip", {
		turns = locale.s("ds.session_stats.turns", { n = s.turns, count = tostring(s.turns) }),
		steps = locale.s("ds.session_stats.steps", { n = s.steps, count = tostring(s.steps) }),
	})
	if s.tps then
		label ..= " " .. locale.s("ds.session_stats.tail", { n = s.tps })
	end
	return label
end

local SPLIT: { [string]: { key: string, ink: string } } = {
	Input = { key = "ds.session_stats.legend.input", ink = "$accent" },
	["Cache read"] = { key = "ds.session_stats.legend.cache_read", ink = "$success" },
	["Cache write"] = { key = "ds.session_stats.legend.cache_write", ink = "$warning" },
	Output = { key = "ds.session_stats.legend.output", ink = "$text.secondary" },
}

local function slug(label: string): string
	return string.lower((string.gsub(label, " ", "-")))
end

local function legend_cell(part: { label: string, n: number, tokens: string }): Node
	local kind = SPLIT[part.label] or SPLIT.Output
	return ui.row {
		h = 20,
		grow = 1,
		basis = 0,
		gap = "$zeron.space.tight",
		items = "center",
		text_size = 11.5,
		color = "$text.secondary",
		ui.div { w = 7, h = 7, rounded = 2, shrink = 0, bg = kind.ink },
		ui.text { locale.t(kind.key), grow = 1 },
		ui.text { part.tokens, id = `session-stats-part-{slug(part.label)}`, tabular = true, color = "$text.primary" },
	}
end

local function cost_section(stats: () -> Stats?): Node
	local function now(): Stats
		return stats() :: Stats
	end
	return ui.presence(function(): string?
		local s = stats()
		if not s or not s.headline then
			return nil
		end
		local parts = { tostring(s.priced), tostring(s.total_n) }
		for _, p in s.split do
			table.insert(parts, `{p.label}={p.n}={p.tokens}`)
		end
		return table.concat(parts, "|")
	end, function(): Node
		local s = now()
		local rows: { Child } = {
			Menu.CardHeading { locale.t("ds.session_stats.cost_heading") },
			ui.row {
				px = "$space.2",
				pb = "$zeron.space.tight",
				items = "center",
				justify = "between",
				ui.text {
					function(): string
						local n = now()
						return if n.priced then n.headline or "" else locale.s("ds.session_stats.no_price")
					end,
					tabular = true,
					id = "session-stats-headline",
					text_size = if s.priced then 20 else 15,
					weight = "$font.weight.medium",
					color = if s.priced then "$text.primary" else "$text.tertiary",
				},
				ui.text {
					function(): string
						return locale.s("ds.session_stats.tokens", { n = now().total })
					end,
					id = "session-stats-total",
					tabular = true,
					text_size = "$font.size.xs",
					color = "$text.secondary",
				},
			},
		}
		if s.total_n > 0 then
			local bar: { Child } = {}
			for _, p in s.split do
				if p.n > 0 then
					table.insert(bar, ui.div {
						id = `session-stats-bar-{slug(p.label)}`,
						h = "100%",
						min_w = 2,
						grow = p.n / s.total_n,
						basis = 0,
						bg = (SPLIT[p.label] or SPLIT.Output).ink,
					})
				end
			end
			table.insert(rows, ui.row {
				id = "session-stats-bar",
				mx = "$space.2",
				h = 6,
				rounded = 3,
				overflow = "hidden",
				bg = "$hover",
				table.unpack(bar),
			})
			table.insert(rows, ui.div { h = "$space.1" })
			local i = 1
			while i <= #s.split do
				table.insert(rows, ui.row {
					px = "$space.2",
					gap = "$space.3",
					legend_cell(s.split[i]),
					if s.split[i + 1] then legend_cell(s.split[i + 1]) else ui.div { grow = 1, basis = 0 },
				})
				i += 2
			end
		end
		table.insert(rows, ui.show(function(): boolean
			local n = now()
			return n.unpriced > 0 or n.estimated
		end, ui.text {
			function(): string
				local n = now()
				if n.unpriced > 0 then
					return locale.s("ds.session_stats.excludes", { n = n.unpriced, count = tostring(n.unpriced) })
				end
				return locale.s("ds.session_stats.estimated")
			end,
			id = "session-stats-note",
			px = "$space.2",
			pt = "$space.1",
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		}))
		table.insert(rows, ui.div { mx = ui.raw(-4, "the card's inset: the rule runs edge to edge"), my = "$space.1", Menu.Rule {} })
		return ui.column { w_full = true, table.unpack(rows) }
	end, { w_full = true })
end

local function stat_row(label: any, id: string, value: () -> string): Node
	return ui.row {
		px = "$space.2",
		h = 24,
		items = "center",
		justify = "between",
		gap = "$space.3",
		text_size = "$font.size.sm",
		color = "$text.secondary",
		ui.text { label },
		ui.text { value, id = id, tabular = true, color = "$text.primary" },
	}
end

-- A hover card stays mounted once opened, and a key closes it under a resting pointer: its tick ends with what shows.
local CardBody = ui.component(function(props: { stats: () -> Stats?, working: () -> boolean }): Node
	local now = ui.use_state(ui.now())
	local shown = ui.use_state(false)
	ui.effect(function()
		if shown:get() then
			now:set(ui.now())
			ui.every(1000, function()
				now:set(ui.now())
			end)
		end
	end)
	local stats = props.stats
	local function cache(): Cache?
		local s = stats()
		return if s then M.cache_of(s, props.working(), now:get()) else nil
	end
	local function field(key: string): () -> string
		return function(): string
			local s: any = stats()
			return (s and s[key]) or locale.s("ds.session_stats.missing")
		end
	end
	return ui.column {
		w_full = true,
		on_visible = function(visible: boolean)
			shown:set(visible)
		end,
		cost_section(stats),
		Menu.CardHeading { locale.t("ds.session_stats.heading") },
		stat_row(locale.t("ds.session_stats.llm"), "session-stats-llm", field("llm")),
		stat_row(locale.t("ds.session_stats.tool"), "session-stats-tool", field("tool")),
		stat_row(locale.t("ds.session_stats.ttft"), "session-stats-ttft", field("ttft")),
		stat_row(locale.t("ds.session_stats.tps"), "session-stats-tps", function(): string
			local s = stats()
			return if s and s.tps then locale.s("ds.session_stats.tok_s", { n = s.tps }) else locale.s("ds.session_stats.missing")
		end),
		ui.show(function(): boolean
			return cache() ~= nil
		end, stat_row(locale.t("ds.session_stats.ttl"), "session-stats-ttl", function(): string
			local c = cache()
			return if c then M.cache_long(c) else ""
		end)),
		ui.show(function(): boolean
			local s = stats()
			return s ~= nil and s.hit ~= nil
		end, stat_row(locale.t("ds.session_stats.hit"), "session-stats-hit", function(): string
			local s = stats()
			return locale.s("ds.session_stats.hit_value", { n = if s then s.hit else "" })
		end)),
		ui.div { h = "$space.1" },
	}
end)

M.Chip = ui.component(function(props: { stats: () -> Stats?, working: () -> boolean, side: () -> string }): Node
	local stats, working = props.stats, props.working
	local word = ui.use_state(nil :: { short: string, tone: string }?)
	local beat = ui.use_state(0)
	ui.effect(function()
		beat:get()
		local s = stats()
		local c = if s then M.cache_of(s, working(), ui.now()) else nil
		local next_word = if c
			then { short = M.cache_short(c), tone = if M.running_low(c) then "$warning" elseif c.kind == "cold" then "$zeron.composer.faint" else "$text.tertiary" }
			else nil
		local old = word:peek()
		if (old == nil) ~= (next_word == nil) or (old and next_word and (old.short ~= next_word.short or old.tone ~= next_word.tone)) then
			word:set(next_word)
		end
		local wait = if c then M.next_change(c) else nil
		if wait then
			ui.after(wait, function()
				beat:set(beat:peek() + 1)
			end)
		end
	end)
	local function label(): string
		local s = stats()
		return (if s then M.chip_label(s) else nil) or ""
	end
	return ui.show(function(): boolean
		return label() ~= ""
	end, ui.presence(props.side, function(at: string): Node
		return ui.row {
			id = "session-stats",
			h = 20,
			px = "$space.2",
			gap = "$zeron.space.tight",
			min_w = 0,
			shrink = 1,
			overflow = "hidden",
			items = "center",
			rounded = "$radius.md",
			selectable = false,
			text_size = "$font.size.sm",
			weight = "$font.weight.medium",
			color = "$text.tertiary",
			hover = { bg = "$hover" },
			label = locale.t("ds.session_stats.heading"),
			tooltip = HoverCard.Card {
				id = "session-stats-card",
				w = 300,
				p = "$space.1",
				gap = 0,
				CardBody { stats = stats, working = working },
			},
			tooltip_placement = { side = at, align = "start", offset = 8 },
			ui.div { shrink = 0, Icon { name = "speedometer", size = 12 } },
			FadeText { label, id = "session-stats-label", mb = "$zeron.space.hair", min_w = 0, shrink = 1, fade = 16 },
			ui.show(function(): boolean
				return word:get() ~= nil
			end, ui.row {
				shrink = 0,
				gap = "$space.1",
				items = "center",
				color = function(): string
					local w = word:get()
					return if w then w.tone else "$text.tertiary"
				end,
				ui.text { "·" },
				ui.div { shrink = 0, Icon { name = "clock-circle", size = 11 } },
				ui.text {
					function(): string
						local w = word:get()
						return if w then w.short else ""
					end,
					id = "session-stats-cache",
					tabular = true,
				},
			}),
		}
	end, { min_w = 0, shrink = 1 }))
end)

return M
