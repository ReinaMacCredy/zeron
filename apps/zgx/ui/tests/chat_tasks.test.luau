-- A chat's background processes, as Zeron's tray has them
-- (background_tray.rs): Stop asks the chat's host to stop one, and says so
-- if it couldn't.

test("Stop on a background process asks its host, says Stopping, and says when it couldn't", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	app:frames(30)
	app:command("task_running")
	app:frames(30)
	expect(app:text("#pills-shells-count")).to_be("1 Running")
	app:click("#pills-shells-pill")
	app:frames(20)
	app:click('role:button "Stop npm run dev"')
	app:frames(10)
	expect(app:state("calls")).to_contain("stop_task:s2:bg1")
	expect(app:has('~"Stopping"')).to_be(true)
	expect(app:has('~"Couldn\'t stop that process"')).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("a shell's Stop that fails gives its Stop back and says so", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	app:frames(30)
	app:command("task_running")
	app:frames(30)
	app:click("#pills-shells-pill")
	app:frames(20)
	app:command("stop_fails")
	app:click('role:button "Stop npm run dev"')
	app:frames(10)
	expect(app:has('~"Stopping"')).to_be(false)
	expect(app:has('~"Couldn\'t stop that process"')).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("a background process outlives the turn: its pill stays as the turn ends", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	app:frames(30)
	app:command("start_reply")
	app:command("task_running")
	app:frames(30)
	expect(app:has("#pills-shells-pill")).to_be(true)
	app:command("end_reply")
	-- Through the turn's end, every frame: the pill is there.
	for _ = 1, 30 do
		app:frames(1)
		expect(app:has("#pills-shells-pill")).to_be(true)
	end
	-- And still stoppable.
	app:click("#pills-shells-pill")
	app:frames(20)
	app:click('role:button "Stop npm run dev"')
	app:frames(10)
	expect(app:state("calls")).to_contain("stop_task:s2:bg1")
	expect(app:error()).to_be(nil)
end)

-- The plan (todo_panel.rs), on ui/queue_fixture.luau.

local function plan(states: { string }): { any }
	local out = {}
	for i, s in states do
		table.insert(out, { id = `t{i}`, text = `Step {i}`, state = s })
	end
	return out
end

local function with_plan(states: { string }, running: boolean?): any
	local app = mount("queue_fixture.luau")
	app:resize(900, 700)
	app:set_state("test_session", { id = "s1", running = running == true, steers = true, stopped = false, queue = {}, todos = plan(states) })
	app:frames(30)
	return app
end

test("a plan with work left opens; a finished one stays compact, even in a later turn", function()
	local app = with_plan({ "done", "doing", "pending" }, true)
	expect(app:has('role:button "Collapse todo list"')).to_be(true)
	expect(app:has('~"Step 2"')).to_be(true)
	local done = with_plan({ "done", "done" }, true)
	expect(done:has('role:button "Expand todo list"')).to_be(true)
	expect(done:has('~"All done"')).to_be(true)
end)

test("a plan tidies itself on settling, once: opened by hand after that, it stays", function()
	local app = with_plan({ "done", "doing" }, true)
	app:click('role:button "Collapse todo list"')
	app:frames(5)
	app:click('role:button "Expand todo list"')
	app:frames(5)
	app:set_state("test_session", { id = "s1", running = false, steers = true, stopped = false, queue = {}, todos = plan({ "done", "done" }) })
	app:frames(5)
	expect(app:has('role:button "Expand todo list"')).to_be(true)
	app:click('role:button "Expand todo list"')
	app:frames(5)
	expect(app:has('role:button "Collapse todo list"')).to_be(true)
end)

test("a plan dismissed on an idle turn stays gone until the list changes", function()
	local app = with_plan({ "done", "doing", "pending" }, true)
	expect(app:has('role:button "Dismiss todo list"')).to_be(false)
	local idle = { id = "s1", running = false, steers = true, stopped = false, queue = {}, todos = plan({ "done", "doing", "pending" }) }
	app:set_state("test_session", idle)
	app:frames(5)
	app:click('role:button "Dismiss todo list"')
	app:frames(5)
	expect(app:has('role:button "Collapse todo list"')).to_be(false)
	expect(app:has('role:button "Expand todo list"')).to_be(false)
	app:set_state("test_session", { id = "s1", running = false, steers = true, stopped = false, queue = {}, todos = plan({ "done", "doing", "pending" }) })
	app:frames(5)
	expect(app:has('role:button "Expand todo list"')).to_be(false)
	app:set_state("test_session", { id = "s1", running = false, steers = true, stopped = false, queue = {}, todos = plan({ "done", "done", "doing" }) })
	app:frames(5)
	expect(app:has('role:button "Collapse todo list"')).to_be(true)
end)

test("a long plan folds round the step in progress, its folds showing and hiding", function()
	local app = with_plan({ "done", "done", "done", "doing", "pending", "pending", "pending", "pending" }, true)
	expect(app:has('role:button "Show 2 earlier items"')).to_be(true)
	expect(app:has('role:button "Show 3 later items"')).to_be(true)
	expect(app:has('~"Step 1"')).to_be(false)
	app:click('role:button "Show 2 earlier items"')
	app:frames(10)
	expect(app:has('~"Step 1"')).to_be(true)
	app:click('role:button "Hide 2 earlier items"')
	app:frames(10)
	expect(app:has('~"Step 1"')).to_be(false)
	expect(app:has('role:button "Show 2 earlier items"')).to_be(true)
	expect(app:has('~"Step 8"')).to_be(false)
end)
