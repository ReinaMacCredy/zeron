-- The prompt cache's time, as the host's `chat.stats` gives it (its TTL and
-- when a turn last touched it), for the context card and the compaction's
-- rows, and the footer's context chip (`use_word`). The countdown
-- stays in Luau: a host action answers no script synchronously.

local locale = require("@self/locale")

export type Last = { tps: string?, output: string }

export type Stats = {
	turns: number,
	cache_ttl: number?,
	cache_touched: number?,
	-- The last turn seen ending: its average rate and its output.
	last: Last?,
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

-- Time left as a countdown reads it: "3:12", "1:00:00".
function M.clock(ms: number): string
	local secs = math.max(0, ms // 1000)
	if secs >= 3600 then
		return string.format("%d:%02d:%02d", secs // 3600, (secs % 3600) // 60, secs % 60)
	end
	return string.format("%d:%02d", secs // 60, secs % 60)
end

function M.cache_long(c: Cache): string
	if c.kind == "refreshing" then
		return locale.s("ds.session_stats.refreshing", { tier = tier(c.ttl) })
	elseif c.kind == "cold" then
		return locale.s("ds.session_stats.expired", { ago = M.duration(c.ago) })
	end
	return locale.s("ds.session_stats.left", { left = M.duration(c.left), tier = tier(c.ttl) })
end

-- The context card's word for it: "3:12 left", "Refreshing", "Expired".
function M.cache_countdown(c: Cache): string
	if c.kind == "refreshing" then
		return locale.s("ds.composer.context.cache_refreshing")
	elseif c.kind == "cold" then
		return locale.s("ds.composer.context.cache_expired")
	end
	return locale.s("ds.composer.context.cache_left", { left = M.clock(c.left) })
end

-- The footer's word for it, gpui's `Cache::short`: "1h" refreshing, "4m", "30s", "cold".
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

-- The prompt cache's word as gpui's stats chip ends (session_stats.rs
-- `chip`): short, long (its tooltip and label), and its ink, the warning's
-- under a fifth of the TTL, quiet once cold; nil until a turn has told the
-- cache's TTL. Called in a component's body: it wakes only when the word
-- changes, one `ui.after` at a time.
export type Word = { short: string, long: string, tone: string }

function M.use_word(stats: () -> Stats?, working: () -> boolean): State<Word?>
	local word = ui.use_state(nil :: Word?)
	local beat = ui.use_state(0)
	ui.effect(function()
		beat:get()
		local s = stats()
		local c = if s then M.cache_of(s, working(), ui.now()) else nil
		local next_word = if c
			then {
				short = M.cache_short(c),
				long = M.cache_long(c),
				tone = if M.running_low(c) then "$warning" elseif c.kind == "cold" then "$zeron.composer.faint" else "$text.tertiary",
			}
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
	return word
end

return M
