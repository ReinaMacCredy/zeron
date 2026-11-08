ui.theme.strict("warn")
local ds = require("@ds")
local SessionStats = require("@ds/session_stats")
ds.setup()

local function words(): string
	local out: { string } = {}
	for _, ms in { 13400, 60000, 59 * 60000 + 59000, 3600000, 172 * 60000 + 14000, 24 * 3600000, 50 * 3600000, 30 * 86400000, 400 * 86400000 } do
		table.insert(out, SessionStats.duration(ms))
	end
	local s: any = { cache_ttl = 3600, cache_touched = 1000000 }
	local warm = SessionStats.cache_of(s, false, 1000000 + 8 * 60000) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_long(warm))
	table.insert(out, SessionStats.cache_countdown(warm))
	local cold = SessionStats.cache_of(s, false, 1000000 + 3600000 + 720000) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_long(cold))
	table.insert(out, SessionStats.cache_countdown(cold))
	local live = SessionStats.cache_of(s, true, 1000000 + 9000000) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_long(live))
	table.insert(out, SessionStats.cache_countdown(live))
	table.insert(out, tostring(SessionStats.cache_of({} :: any, false, 0)))
	for _, ms in { 192500, 59000, 999, 3600000, -5 } do
		table.insert(out, SessionStats.clock(ms))
	end
	return table.concat(out, "|")
end
ui.state("test_words", ""):set(words())

return ui.column { id = "fixture-root", w_full = true, h_full = true, bg = "$background" }
