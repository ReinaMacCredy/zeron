local function open(): any
	local app = mount("lanes_fixture.luau")
	app:resize(1400, 900)
	app:frames(30)
	return app
end

local function calls(app: any): string
	return app:state("calls") or ""
end

local function tabs(app: any): { any }
	local s = ((app:state("panel_sessions") :: any) or {})["chat:chat-1"]
	return if s then s.tabs else {}
end

local function seeded(): any
	local app = open()
	app:command("threads_seed")
	app:frames(10)
	app:click('#session-lanes >> role:button "Open side chat Looking at the tests"')
	app:frames(20)
	app:command("threads_open")
	app:frames(30)
	return app
end

test("threads: the tab opens first, counts the chat's rows, and groups them from Ready for review to Done", function()
	local app = seeded()
	local list = tabs(app)
	expect(list[1].kind).to_be("threads")
	expect(list[2].kind).to_be("side_chat")
	expect(app:text(`#panel-tab-{list[1].id}-count`)).to_be("9")
	expect(app:text("#threads-count")).to_be("9 threads")
	local function y(sel: string): number
		return tonumber(string.match(app:tree(sel), ",(-?[%d.]+) ")) :: number
	end
	local order = {}
	for _, g in { "review", "waiting", "working", "planned", "done" } do
		table.insert(order, y(`#threads-group-{g}`))
	end
	for i = 2, #order do
		expect(order[i] > order[i - 1]).to_be(true)
	end
	expect(app:tree("#threads-group-waiting")).to_contain("Waiting on you · 2")
	expect(y("#threads-row-owned-dg-fail") > order[2] and y("#threads-row-owned-dg-fail") < order[3]).to_be(true)
	expect(app:has(".threads-row-failed")).to_be(true)
	expect(app:tree("#threads-row-step-s6")).to_contain("Skipped")
	expect(app:tree("#threads-row-subagent-chat-1--sub--1")).to_contain("Sub-agent · read-only")
	expect(app:tree("#threads-row-peer-peer-1")).to_contain("Peer · read-only")
	app:click("#threads-group-done")
	app:frames(20)
	expect(app:has("#threads-row-owned-dg-done")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("threads: a row opens its thread in the tab, waiting says so, and back lets it go", function()
	local app = seeded()
	app:click("#threads-row-owned-dg-ask")
	app:frames(30)
	expect(app:has("#threads-thread")).to_be(true)
	expect(calls(app)).to_contain("watch:dg-ask")
	expect(app:tree("#threads-waiting-banner")).to_contain("Waiting on you: Where should the rate limiter")
	expect(tabs(app)[1].thread).to_be("dg-ask")
	app:click("#threads-back")
	app:frames(30)
	expect(app:has("#threads-list")).to_be(true)
	expect(calls(app)).to_contain("watch:none")
	expect(tabs(app)[1].thread).to_be(nil)
	expect(app:error()).to_be(nil)
end)

test("threads: the tab follows its chat's peers while it's there, and nothing once it closes", function()
	local app = seeded()
	expect(calls(app)).to_contain("follow:chat-1")
	app:click(`#panel-tab-{tabs(app)[1].id}`)
	app:key("cmd+w")
	app:frames(30)
	expect(tabs(app)[1].kind).to_be("side_chat")
	local last: string? = nil
	for chats in string.gmatch(calls(app), "follow:([^,]*)") do
		last = chats
	end
	expect(last).to_be("")
	expect(app:error()).to_be(nil)
end)

test("threads: a step's menu marks it and runs it again; a read-only row has no Stop or Archive", function()
	local app = seeded()
	app:hover("#threads-row-peer-peer-1")
	app:frames(5)
	app:click('#threads-row-peer-peer-1 >> role:button "More for A peer thread"')
	app:frames(20)
	expect(app:has('role:menuitem "Open thread"')).to_be(true)
	expect(app:has('role:menuitem "Stop"')).to_be(false)
	expect(app:has('role:menuitem "Archive"')).to_be(false)
	app:key("Escape")
	app:frames(20)
	app:hover("#threads-row-step-s1")
	app:frames(5)
	app:click('#threads-row-step-s1 >> role:button "More for Add CSV export to orders"')
	app:frames(20)
	app:click('role:menuitem "Mark done"')
	app:frames(10)
	expect(calls(app)).to_contain("threads.mark:s1:done")
	app:hover("#threads-row-owned-dg-ask")
	app:frames(5)
	app:click('#threads-row-owned-dg-ask >> role:button "More for Add rate limiting to checkout"')
	app:frames(20)
	app:click('role:menuitem "Run again"')
	app:frames(10)
	expect(calls(app)).to_contain("threads.retry:s2:")
	app:key("Escape")
	app:frames(20)
	app:hover("#threads-row-owned-dg-fail")
	app:frames(5)
	app:click('#threads-row-owned-dg-fail >> role:button "More for Fix flaky cart total test"')
	app:frames(20)
	app:click('role:menuitem "Mark done"')
	app:frames(10)
	expect(calls(app)).to_contain("settle:dg-fail:true")
	expect(app:error()).to_be(nil)
end)

test("threads: a chat with no rows and no plan says how threads come", function()
	local app = open()
	app:command("threads_open")
	app:frames(30)
	expect(app:has("#threads-empty")).to_be(true)
	expect(app:text("#threads-count")).to_be("0 threads")
	expect(app:error()).to_be(nil)
end)
