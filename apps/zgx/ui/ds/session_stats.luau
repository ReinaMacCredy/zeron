-- The prompt cache's time, as the host's `chat.stats` gives it (its TTL and
-- when a turn last touched it), for the context card and the compaction's
-- rows. The countdown stays in Luau: a host action answers no script
-- synchronously.

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

return M
