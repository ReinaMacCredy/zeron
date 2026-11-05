-- An engine older than the one on disk: started again at once when it's
-- this app's and idle; offered ("Restart to update") when it's this app's
-- and busy; only noted when someone else started it: the app rail's Update
-- entry. Too old for this interface: the sidebar foot's chip. On
-- ui/engine_update_fixture.luau.

local function eventually(app: any, ok: () -> boolean, ms: number?): boolean
	local tries = math.ceil((ms or 15000) / 50)
	for _ = 1, tries do
		if ok() then
			return true
		end
		app:wait('"never shown, only waited past"', { timeout_ms = 50 })
	end
	return ok()
end

local ENTRY, CHIP = "#update-rail-button", "#engine-update"

local function none_shown(app: any)
	expect(app:has(ENTRY)).to_be(false)
	expect(app:has(CHIP)).to_be(false)
end

local function open(): any
	local app = mount("engine_update_fixture.luau")
	app:frames(10)
	expect(app:error()).to_be_nil()
	return app
end

test("current: nothing shown, nothing restarted", function()
	local app = open()
	app:set_state("engine.update", { stale = false, ours = true })
	app:frames(10)
	none_shown(app)
	expect(app:state("test_engine_restarts")).to_be(0)
end)

test("stale, this app's and idle: started again at once, nothing shown", function()
	local app = open()
	app:set_state("engine.update", { stale = true, ours = true })
	expect(eventually(app, function(): boolean
		return app:state("test_engine_restarts") == 1
	end)).to_be(true)
	app:frames(10)
	none_shown(app)
end)

test("stale, this app's and busy: offered, restarted only when asked", function()
	local app = open()
	app:set_state("test_engine_chats", { { id = "c1", title = "Working", device = "mac", status = "working", updated = 1, created = 1 } })
	app:frames(5)
	app:set_state("engine.update", { stale = true, ours = true })
	expect(app:wait('#app-rail >> role:button "Restart to update"', { timeout_ms = 15000 })).to_be(true)
	expect(app:has(CHIP)).to_be(false)
	expect(app:state("test_engine_restarts")).to_be(0)
	app:click('#app-rail >> role:button "Restart to update"')
	expect(eventually(app, function(): boolean
		return app:state("test_engine_restarts") == 1
	end)).to_be(true)
	app:frames(10)
	none_shown(app)
end)

test("stale, someone else's: noted, never restarted", function()
	local app = open()
	app:set_state("engine.update", { stale = true, ours = false })
	expect(app:wait(`#app-rail >> role:button "Zeron's engine is out of date"`, { timeout_ms = 15000 })).to_be(true)
	expect(app:has(CHIP)).to_be(false)
	app:click(ENTRY)
	app:frames(10)
	expect(app:state("test_engine_restarts")).to_be(0)
end)

-- The beta's check: an engine that doesn't serve this interface is the Zeron
-- app's to update: said, and never restarted from here.
test("an engine too old for this interface: said, never restarted", function()
	local app = open()
	app:set_state("engine.interface", { supported = false })
	expect(app:wait('role:button "This Zeron needs a newer engine"', { timeout_ms = 15000 })).to_be(true)
	expect(app:has(ENTRY)).to_be(false)
	app:frames(10)
	expect(app:state("test_engine_restarts")).to_be(0)
	-- A newer engine serves it: nothing shown.
	app:set_state("engine.interface", { supported = true })
	app:frames(10)
	none_shown(app)
end)
