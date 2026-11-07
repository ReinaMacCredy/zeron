
ui.theme.strict("warn")
local ds = require("@ds")
local SessionStats = require("@ds/session_stats")
ds.setup()

local stats = ui.state("test_stats", nil :: any)
local working = ui.state("test_working", false)
-- How long the cache has left, from the moment it's set (a test's own VM has no clock).
local left = ui.state("test_left_ms", nil :: number?)
local touched = ui.state("test_touched", nil :: number?)
ui.effect(function()
	local l = left:get()
	touched:set(if l then ui.now() - 3600000 + l else nil)
end)

local function words(): string
	local out: { string } = {}
	for _, ms in { 13400, 60000, 59 * 60000 + 59000, 3600000, 172 * 60000 + 14000, 24 * 3600000, 50 * 3600000, 30 * 86400000, 400 * 86400000 } do
		table.insert(out, SessionStats.duration(ms))
	end
	local s: any = { cache_ttl = 3600, cache_touched = 1000000 }
	local warm = SessionStats.cache_of(s, false, 1000000 + 8 * 60000) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_short(warm))
	table.insert(out, SessionStats.cache_long(warm))
	table.insert(out, tostring(SessionStats.running_low(warm)))
	local low = SessionStats.cache_of(s, false, 1000000 + 3590000) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_short(low))
	table.insert(out, tostring(SessionStats.running_low(low)))
	local cold = SessionStats.cache_of(s, false, 1000000 + 3600000 + 720000) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_short(cold))
	table.insert(out, SessionStats.cache_long(cold))
	local live = SessionStats.cache_of(s, true, 1000000 + 9000000) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_short(live))
	table.insert(out, SessionStats.cache_long(live))
	table.insert(out, tostring(SessionStats.cache_of({} :: any, false, 0)))
	table.insert(out, tostring(SessionStats.next_change(warm)))
	table.insert(out, tostring(SessionStats.next_change(low)))
	table.insert(out, tostring(SessionStats.next_change(cold)))
	table.insert(out, tostring(SessionStats.next_change(live)))
	local five = SessionStats.cache_of({ cache_ttl = 300, cache_touched = 0 } :: any, true, 0) :: SessionStats.Cache
	table.insert(out, SessionStats.cache_short(five))
	table.insert(out, SessionStats.chip_label({ turns = 86, steps = 1903, tps = "137" } :: any) or "nil")
	table.insert(out, SessionStats.chip_label({ turns = 1, steps = 1 } :: any) or "nil")
	table.insert(out, SessionStats.chip_label({ turns = 0, steps = 0 } :: any) or "nil")
	return table.concat(out, "|")
end
ui.state("test_words", ""):set(words())

return ui.column {
	id = "fixture-root",
	w_full = true,
	h_full = true,
	bg = "$background",
	justify = "end",
	p = "$space.4",
	ui.row {
		id = "fixture-footer",
		w = ui.state("test_w", 500),
		h = 24,
		items = "center",
		SessionStats.Chip {
			stats = function(): any
				local s = stats:get()
				local t = touched:get()
				if not s or not t then
					return s
				end
				local with = table.clone(s)
				with.cache_ttl, with.cache_touched = 3600, t
				return with
			end,
			working = function(): boolean
				return working:get()
			end,
			side = function(): string
				return "top"
			end,
		},
	},
}
