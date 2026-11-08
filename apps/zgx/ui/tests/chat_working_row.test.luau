
local function settle(app: any)
	app:frames(30)
end

local function working(app: any): any
	local app2 = app
	app2:command("go_s1")
	settle(app2)
	app2:command("thinking_turn")
	settle(app2)
	return app2
end

local function opened(): any
	local app = mount("chat_fixture.luau")
	app:resize(1100, 700)
	return working(app)
end

test("working row: the live rate shows beside the clock and goes when it stops", function()
	local app = opened()
	expect(app:has("#working-rate")).to_be(false)
	expect(app:has("#working-clock")).to_be(true)
	app:command("rate_41")
	settle(app)
	expect(app:text("#working-rate")).to_be("· 41 tok/s")
	app:command("rate_gone")
	settle(app)
	expect(app:has("#working-rate")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("working row: consulting its advisor", function()
	local app = opened()
	app:command("says_consulting")
	app:wait('~"Consulting advisor…"', { timeout_ms = 2000 })
	expect(app:has('~"Consulting advisor…"')).to_be(true)
	app:command("says_nothing")
	app:wait('~"Consulting advisor…"', { gone = true, timeout_ms = 2000 })
	expect(app:has('~"Consulting advisor…"')).to_be(false)
end)

test("working row: a remote chat whose sync stalled says Queued, as gpui's", function()
	local app = mount("chat_fixture.luau")
	app:command("stalled_words")
	expect(app:state("test_stalled")).to_be("queued|nil|nil")
	expect(app:error()).to_be(nil)
end)

test("footer: no pull request's place on a project with no git", function()
	local app = mount("chat_fixture.luau")
	app:resize(1100, 700)
	app:command("go_s1")
	settle(app)
	app:command("stats_say")
	settle(app)
	expect(app:has(".composer-footer-status")).to_be(true)
	app:command("project_no_git")
	settle(app)
	expect(app:has(".composer-footer-status")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("working row: what a send waits on, no rate and no clock meanwhile", function()
	local app = mount("chat_fixture.luau")
	app:resize(1100, 700)
	app:command("go_s1")
	settle(app)
	app:command("rate_41")
	app:type('role:textbox "Message"', "Say something")
	app:key("Enter")
	expect(app:wait('~"Sending…"', { timeout_ms = 3000 })).to_be(true)
	expect(app:has("#working-rate")).to_be(false)
	expect(app:has("#working-clock")).to_be(false)
	app:command("says_starting")
	expect(app:wait('~"Starting a Cloud machine…"', { timeout_ms = 3000 })).to_be(true)
	app:command("says_waking")
	expect(app:wait('~"Waking the Cloud machine…"', { timeout_ms = 3000 })).to_be(true)
	app:command("connection_offline")
	expect(app:wait('~"Queued — will send automatically"', { timeout_ms = 3000 })).to_be(true)
	expect(app:has('~"will send automatically…"')).to_be(false)
	app:command("connection_back")
	expect(app:wait('~"Waking the Cloud machine…"', { timeout_ms = 3000 })).to_be(true)
	expect(app:error()).to_be(nil)
end)

-- `ui.now()` is the wall clock and a test's frames run faster than it.
local function until_text(app: any, sel: string, done: (string) -> boolean): boolean
	local began = os.clock()
	while os.clock() - began < 4 do
		app:frames(20)
		if app:has(sel) and done(app:text(sel)) then
			return true
		end
	end
	return false
end

local function run_for(app: any, seconds: number)
	local began = os.clock()
	while os.clock() - began < seconds do
		app:frames(20)
	end
end

test("working row: the turn's output so far beside the rate, on the clock's tick, gone once it settles", function()
	local app = opened()
	expect(app:has("#working-tokens")).to_be(false)
	app:command("rate_41")
	app:command("tokens_1_1k")
	expect(until_text(app, "#working-tokens", function(t: string): boolean
		return t == "· 1.1K tokens"
	end)).to_be(true)
	expect(app:text("#working-rate")).to_be("· 41 tok/s")
	app:command("turn_settles")
	app:wait("#working-tokens", { gone = true, timeout_ms = 2000 })
	expect(app:has("#working-rate")).to_be(false)
	expect(app:has("#working-clock")).to_be(false)
	expect(app:error()).to_be(nil)
end)

local function card(app: any)
	app:command("context_84k")
	settle(app)
	app:hover("#composer-context")
	app:wait("#composer-context-card", { timeout_ms = 2000 })
	settle(app)
end

test("context card: the last turn's rate, output and the cache's time left, once a turn has ended", function()
	local app = mount("chat_fixture.luau")
	app:resize(1100, 700)
	app:command("go_s1")
	settle(app)
	app:command("stats_none_ended")
	card(app)
	expect(app:text("#composer-context-used")).to_be("84,000 / 200,000 tokens")
	expect(app:has("#composer-context-turn")).to_be(false)
	app:command("stats_turn_ended")
	settle(app)
	expect(app:text("#composer-context-turn-rate")).to_be("52 tok/s")
	expect(app:text("#composer-context-turn-output")).to_be("1.2K tokens")
	expect(app:has('#composer-context-turn >> "Last turn"')).to_be(true)
	expect(app:text("#composer-context-cache")).to_contain(":")
	expect(app:text("#composer-context-cache")).to_contain(" left")
	-- No stats chip in the footer any more: its numbers are the card's.
	expect(app:has("#session-stats")).to_be(false)
	expect(app:has('~"tok/s ·"')).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("context card: the cache counts down once a second while the card shows, and stops with it", function()
	local app = mount("chat_fixture.luau")
	app:resize(1100, 700)
	app:command("go_s1")
	settle(app)
	app:command("stats_turn_ended")
	card(app)
	local open = app:text("#composer-context-cache")
	expect(until_text(app, "#composer-context-cache", function(t: string): boolean
		return t ~= open
	end)).to_be(true)
	app:hover('role:textbox "Message"')
	-- (Its fade runs on the clock; a tick during it is still the card's.)
	run_for(app, 0.5)
	expect(string.find(app:tree('role:dialog "Tooltip"'), "hidden", 1, true) ~= nil or not app:has('role:dialog "Tooltip"')).to_be(true)
	local closed = app:text("#composer-context-cache")
	run_for(app, 2.2)
	expect(app:text("#composer-context-cache")).to_be(closed)
	expect(app:error()).to_be(nil)
end)
