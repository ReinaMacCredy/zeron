local function box(app: any, sel: string): any
	local line = string.match(app:tree(sel), "[^\n]+") or ""
	local x, y, w, h = string.match(line, "(-?[%d.]+),(-?[%d.]+) ([%d.]+)x([%d.]+)")
	return { x = tonumber(x), y = tonumber(y), w = tonumber(w), h = tonumber(h) }
end

local function runs(app: any, input: any): any
	app:set_state("fan_input", input)
	app:frames(5)
	return app:state("fan_output")
end

local function mounted(): any
	local app = mount("agent_fan_fixture.luau")
	app:resize(500, 300)
	app:frames(5)
	return app
end

local function spawn(id: string, mark: string?, at: number, status: string?): any
	return { ref = id, call = id, title = id, status = status or "running", agent = mark, at = at, recent = true }
end

test("one tile per provider: asking first, then newest, at most three show", function()
	local app = mounted()
	local out = runs(app, {
		subs = { spawn("a", "openai", 1), spawn("b", "claude", 5), spawn("c", "openai", 3), spawn("d", "grok", 2), spawn("e", "cursor", 4) },
		children = { { id = "kid", title = "Kid", status = "input", mark = "grok", created = 0, delegation = "running" } },
	})
	expect(out.tiles).to_be("grok!,claude,cursor,openai")
	expect(out.shown).to_be(3)
	app:wait("#never-shown", { timeout_ms = 300 })
	app:frames(5)
	expect(box(app, "#fan").w).to_be(48)
	expect(app:error()).to_be(nil)
end)

test("a lone tile stands square; the fan spaces by its size", function()
	local app = mounted()
	runs(app, { subs = { spawn("a", "openai", 1) } })
	app:wait("#never-shown", { timeout_ms = 300 })
	app:frames(5)
	expect(box(app, "#fan").w).to_be(20)
	expect(box(app, "#fan16").w).to_be(16)
	runs(app, { subs = { spawn("a", "openai", 1), spawn("b", "grok", 2) } })
	app:wait("#never-shown", { timeout_ms = 300 })
	app:frames(5)
	expect(box(app, "#fan").w).to_be(34)
	expect(math.abs(box(app, "#fan16").w - 27.2) < 1).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("what finished leaves the tiles and stays a row; an old settled turn is no row", function()
	local app = mounted()
	local old = spawn("old", "openai", 1, "done")
	old.recent = false
	local out = runs(app, { subs = { old, spawn("now", "grok", 2, "done"), spawn("run", "claude", 3) } })
	expect(out.tiles).to_be("claude")
	expect(out.rows).to_be("now:completed:0,run:working:0")
	expect(app:error()).to_be(nil)
end)

test("a run's own spawns are nested under it; a settled turn stays while one of them works", function()
	local app = mounted()
	local top = spawn("top", "openai", 1, "done")
	top.recent = false
	local out = runs(app, {
		subs = { top },
		usage = { top = { tokens = 29216, spawns = { spawn("inner", "grok", 2) } } },
	})
	expect(out.rows).to_be("top:completed:0,inner:working:1")
	expect(app:error()).to_be(nil)
end)

test("a child's kind and state follow its chat; one from before this turn is gone once settled", function()
	local app = mounted()
	local out = runs(app, {
		turn_at = 100,
		children = {
			{ id = "side", title = "Side", kind = "side_chat", status = "working", created = 1 },
			{ id = "fork", title = "Fork", kind = "fork", status = "failed", created = 150, last_message = 160 },
			{ id = "gone", title = "Gone", kind = "agent", created = 50, last_message = 60 },
			{ id = "del", title = "Del", delegation = "cancelled", created = 120, delegated_at = 120 },
		},
	})
	expect(out.rows).to_be("del:stopped:0,side:working:0,fork:failed:0")
	expect(app:error()).to_be(nil)
end)

test("the words: elapsed and tokens as gpui's", function()
	local app = mounted()
	local out = runs(app, {})
	expect(out.words).to_be("45s|1m 05s|1h 02m|950|29.2k|1.25M")
	expect(app:error()).to_be(nil)
end)

test("a spawn naming no model wears the chat's mark, keyed as the closed chat's fan is", function()
	local app = mounted()
	local no_mark = { ref = "a", call = "a", title = "a", status = "running", at = 1, recent = true }
	local out = runs(app, { subs = { no_mark }, agent = "openai", harness = "mock" })
	expect(out.tiles).to_be("openai")
	out = runs(app, { subs = { no_mark }, harness = "mock" })
	expect(out.tiles).to_be("mock")
	expect(app:error()).to_be(nil)
end)
