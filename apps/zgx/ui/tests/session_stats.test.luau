
local function settle(app: any)
	app:frames(20)
end

local function mounted(): any
	local app = mount("session_stats_fixture.luau")
	app:resize(700, 600)
	settle(app)
	return app
end

-- `ui.now()` is the wall clock and a test's frames run faster than it.
local function until_text(app: any, sel: string, done: (string) -> boolean): boolean
	local began = os.clock()
	while os.clock() - began < 4 do
		app:frames(20)
		if done(app:text(sel)) then
			return true
		end
	end
	return false
end

local function stats(extra: { [string]: any }?): any
	local s: { [string]: any } = {
		turns = 86,
		steps = 1903,
		llm = "13.4s",
		tool = "2h52m",
		ttft = "1.2s",
		tps = "137",
		hit = "90",
		headline = "$12.84",
		priced = true,
		unpriced = 0,
		estimated = false,
		total = "3.1K",
		total_n = 3140,
		split = {
			{ label = "Input", n = 100, tokens = "100" },
			{ label = "Cache read", n = 2000, tokens = "2K" },
			{ label = "Cache write", n = 40, tokens = "40" },
			{ label = "Output", n = 1000, tokens = "1K" },
		},
	}
	for k, v in extra or {} do
		s[k] = if v == "nil" then nil else v
	end
	return s
end

test("session stats: words as stats_view.rs's tests have them", function()
	local app = mounted()
	local words = string.split(app:state("test_words"), "|")
	local expected = {
		"13.4s", "1m00s", "59m59s", "1h00m", "2h52m", "1d00h", "2d02h", "1mo00d", "1y01mo",
		"52m", "52m00s left · 1h", "false",
		"10s", "true",
		"cold", "Expired 12m00s ago",
		"1h", "Refreshing · 1h",
		"nil",
		"1", "10000", "nil", "nil",
		"5m",
		"86 turns 1903 steps · 137 tok/s", "1 turn 1 step", "nil",
	}
	expect(#words).to_be(#expected)
	for i, w in expected do
		expect(words[i]).to_be(w)
	end
end)

test("session stats: no chip on a fresh chat, or on a host that says nothing", function()
	local app = mounted()
	expect(app:has("#session-stats")).to_be(false)
	app:set_state("test_stats", stats({ turns = 0, steps = 0 }))
	settle(app)
	expect(app:has("#session-stats")).to_be(false)
	app:set_state("test_stats", stats())
	settle(app)
	expect(app:text("#session-stats-label[1]")).to_be("86 turns 1903 steps · 137 tok/s")
	expect(app:has("#session-stats-cache")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("session stats: the chip counts down by gpui's rule, and goes cold", function()
	local app = mounted()
	app:set_state("test_stats", stats())
	app:set_state("test_left_ms", 60300)
	settle(app)
	expect(app:text("#session-stats-cache")).to_be("1m")
	expect(until_text(app, "#session-stats-cache", function(t: string): boolean
		return t == "59s" or t == "60s"
	end)).to_be(true)
	app:set_state("test_working", true)
	settle(app)
	expect(app:text("#session-stats-cache")).to_be("1h")
	app:set_state("test_working", false)
	app:set_state("test_left_ms", -1000)
	settle(app)
	expect(app:text("#session-stats-cache")).to_be("cold")
	expect(app:error()).to_be(nil)
end)

test("session stats: the card says the cost, the split and the rows, the TTL ticking", function()
	local app = mounted()
	app:set_state("test_stats", stats())
	app:set_state("test_left_ms", 90000)
	settle(app)
	app:hover("#session-stats")
	app:wait("#session-stats-card", { timeout_ms = 2000 })
	settle(app)
	expect(app:text("#session-stats-headline")).to_be("$12.84")
	expect(app:text("#session-stats-total")).to_be("3.1K tokens")
	expect(app:text("#session-stats-llm")).to_be("13.4s")
	expect(app:text("#session-stats-tool")).to_be("2h52m")
	expect(app:text("#session-stats-ttft")).to_be("1.2s")
	expect(app:text("#session-stats-tps")).to_be("137 tok/s")
	expect(app:text("#session-stats-hit")).to_be("90%")
	local first = app:text("#session-stats-ttl")
	expect(first).to_contain(" left · 1h")
	expect(until_text(app, "#session-stats-ttl", function(t: string): boolean
		return t ~= first
	end)).to_be(true)
	expect(app:has("#session-stats-note")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("session stats: the card's notes and what isn't known", function()
	local app = mounted()
	app:set_state("test_stats", stats({ headline = "≥ $12.84", unpriced = 4 }))
	settle(app)
	app:hover("#session-stats")
	app:wait("#session-stats-card", { timeout_ms = 2000 })
	settle(app)
	expect(app:text("#session-stats-headline")).to_be("≥ $12.84")
	expect(app:text("#session-stats-note")).to_be("Excludes 4 turns on a model with no known price")
	app:set_state("test_stats", stats({ headline = "≈ $12.84", estimated = true }))
	settle(app)
	expect(app:text("#session-stats-note")).to_be("Estimated from public API prices")
	app:set_state("test_stats", stats({ headline = "No known price", priced = false, ttft = "nil", tps = "nil", hit = "nil", unpriced = 0 }))
	settle(app)
	expect(app:text("#session-stats-headline")).to_be("No known price")
	expect(app:text("#session-stats-ttft")).to_be("—")
	expect(app:text("#session-stats-tps")).to_be("—")
	expect(app:has("#session-stats-hit")).to_be(false)
	expect(app:has("#session-stats-ttl")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("session stats: an open card's split, legend and hero follow the numbers", function()
	local app = mounted()
	app:set_state("test_stats", stats({ priced = false, split = {
		{ label = "Input", n = 100, tokens = "100" },
		{ label = "Cache read", n = 0, tokens = "0" },
		{ label = "Output", n = 1000, tokens = "1K" },
	}, total_n = 1100 }))
	settle(app)
	app:hover("#session-stats")
	app:wait("#session-stats-card", { timeout_ms = 2000 })
	settle(app)
	expect(app:text("#session-stats-part-input")).to_be("100")
	expect(app:has("#session-stats-bar-cache-read")).to_be(false)
	local function width(sel: string): number
		local first = string.match(app:tree(sel), "[^\n]+") or ""
		return tonumber(string.match(first, " %-?%d+,%-?%d+ (%d+)x%d+")) or -1
	end
	local input = width("#session-stats-bar-input")
	local function height(sel: string): number
		local first = string.match(app:tree(sel), "[^\n]+") or ""
		return tonumber(string.match(first, " %-?%d+,%-?%d+ %d+x(%d+)")) or -1
	end
	local unpriced = height("#session-stats-headline")
	app:set_state("test_stats", stats({ split = {
		{ label = "Input", n = 900, tokens = "900" },
		{ label = "Cache read", n = 600, tokens = "600" },
		{ label = "Output", n = 500, tokens = "500" },
	}, total_n = 2000 }))
	settle(app)
	expect(app:text("#session-stats-part-input")).to_be("900")
	expect(app:text("#session-stats-part-output")).to_be("500")
	expect(app:has("#session-stats-bar-cache-read")).to_be(true)
	expect(width("#session-stats-bar-input") > input).to_be(true)
	expect(height("#session-stats-headline") > unpriced).to_be(true)
	app:set_state("test_stats", stats({ split = {
		{ label = "Input", n = 100, tokens = "100" },
		{ label = "Cache read", n = 1400, tokens = "1.4K" },
		{ label = "Output", n = 500, tokens = "500" },
	}, total_n = 2000 }))
	settle(app)
	expect(app:text("#session-stats-part-input")).to_be("100")
	expect(app:text("#session-stats-part-cache-read")).to_be("1.4K")
	expect(app:error()).to_be(nil)
end)

test("session stats: another chat's numbers on a card opened in the first", function()
	local app = mounted()
	app:set_state("test_stats", stats())
	settle(app)
	app:hover("#session-stats")
	app:wait("#session-stats-card", { timeout_ms = 2000 })
	settle(app)
	expect(app:text("#session-stats-part-cache-read")).to_be("2K")
	app:hover("#fixture-footer")
	settle(app)
	app:set_state("test_stats", stats({ split = {
		{ label = "Input", n = 7, tokens = "7" },
		{ label = "Cache read", n = 8, tokens = "8" },
		{ label = "Cache write", n = 9, tokens = "9" },
		{ label = "Output", n = 10, tokens = "10" },
	}, total_n = 34 }))
	settle(app)
	app:hover("#session-stats")
	app:wait("#session-stats-card", { timeout_ms = 2000 })
	settle(app)
	expect(app:text("#session-stats-part-cache-read")).to_be("8")
	expect(app:text("#session-stats-part-output")).to_be("10")
	expect(app:error()).to_be(nil)
end)

test("session stats: the card's tick ends with the card", function()
	local app = mounted()
	app:set_state("test_stats", stats())
	app:set_state("test_left_ms", 90000)
	settle(app)
	app:hover("#session-stats")
	app:wait("#session-stats-card", { timeout_ms = 2000 })
	settle(app)
	local open = app:text("#session-stats-ttl")
	expect(until_text(app, "#session-stats-ttl", function(t: string): boolean
		return t ~= open
	end)).to_be(true)
	app:hover("#fixture-root")
	local function run_for(seconds: number)
		local began = os.clock()
		while os.clock() - began < seconds do
			app:frames(20)
		end
	end
	-- (Its fade runs on the clock; a tick during it is still the card's.)
	run_for(0.5)
	expect(string.find(app:tree('role:dialog "Tooltip"'), "hidden", 1, true) ~= nil or not app:has('role:dialog "Tooltip"')).to_be(true)
	local closed = app:text("#session-stats-ttl")
	run_for(2.2)
	expect(app:text("#session-stats-ttl")).to_be(closed)
	expect(app:error()).to_be(nil)
end)
