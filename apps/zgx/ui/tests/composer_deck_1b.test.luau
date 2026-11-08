-- The deck on the chat page (1B, ui/chat_fixture.luau): only the queue and
-- the plan, there only while one of them is, folded to one line over the
-- composer; what runs stays on the pill row above it.

local PILL = 'role:toolbar "What\'s waiting"'

local function box(app: any, what: string): (number, number, number, number)
	local line = app:tree(what):match("[^\n]+") or ""
	local x, y, w, h = line:match(" (%-?[%d%.]+),(%-?[%d%.]+) ([%d%.]+)x([%d%.]+)")
	return tonumber(x) or 0, tonumber(y) or 0, tonumber(w) or 0, tonumber(h) or 0
end

local function top(app: any): number
	local _, y = box(app, 'role:textbox "Message"')
	return y
end

local function plan(done: number, total: number): { any }
	local out = {}
	for i = 1, total do
		table.insert(out, { id = `t{i}`, text = `Step {i}`, state = if i <= done then "done" elseif i == done + 1 then "doing" else "pending" })
	end
	return out
end

local TWO = { { id = "q1", text = "Then update the changelog" }, { id = "q2", text = "Run the tests again" } }

local function chat(): any
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s1")
	app:frames(30)
	app:command("start_reply")
	app:frames(10)
	return app
end

local function patch(app: any, fields: { [string]: any })
	app:set_state("test_session_patch", fields)
	app:command("patch_session")
	app:frames(30)
end

test("with no queue and no plan nothing of the deck stands; one queued message brings its line", function()
	local app = chat()
	expect(app:has(PILL)).to_be(false)
	expect(app:has('role:button "Open Queue"')).to_be(false)
	expect(app:has('role:tab "Open Queue"')).to_be(false)
	app:command("queue_one")
	app:frames(30)
	expect(app:has(PILL)).to_be(true)
	expect(app:tree(PILL)).to_contain("1 queued")
	expect(app:has(`{PILL} >> role:button "Show what's waiting"`)).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("its tabs are Queue and Plan alone; agents and shells stay on the pill row", function()
	local app = chat()
	app:command("subagents_running")
	patch(app, {
		queue = TWO,
		todos = plan(3, 5),
		tasks = { { id = "bg1", kind = "process", name = "npm run dev", detail = ":5173", since = os.time() } },
	})
	expect(app:tree(PILL)).to_contain("2 queued")
	expect(app:tree(PILL)).to_contain("Plan 3/5")
	expect(app:has('role:button "Open Running"')).to_be(false)
	expect(app:has('role:button "Open Subagents"')).to_be(false)
	app:click(`{PILL} >> role:button "Open Queue"`)
	app:frames(30)
	expect(app:count("#chat-composer >> role:tab")).to_be(2)
	expect(app:has('role:tab "Fold Queue"')).to_be(true)
	expect(app:has('role:tab "Open Plan"')).to_be(true)
	expect(app:has('role:tab "Open Running"')).to_be(false)
	expect(app:has('role:tab "Open Subagents"')).to_be(false)
	expect(app:has('role:button "Stop npm run dev"')).to_be(false)
	-- What runs is the pill row's.
	expect(app:has("#pills-shells-pill")).to_be(true)
	expect(app:has("#pills-agents")).to_be(true)
	app:click('role:tab "Open Plan"')
	app:frames(30)
	expect(app:has('~"Step 4"')).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("Steer all, ✕, Edit and Steer from the deck's rows", function()
	local app = chat()
	patch(app, { queue = TWO, steers = true })
	app:click(`{PILL} >> role:button "Open Queue"`)
	app:frames(30)
	app:click('role:button "Steer all"')
	app:frames(10)
	expect(app:state("calls")).to_contain("steer_all:s1:q1,q2")
	-- (The queue emptied, its tab folded: open again.)
	patch(app, { queue = TWO })
	app:click(`{PILL} >> role:button "Open Queue"`)
	app:frames(30)
	local second = 'role:listitem "2. Run the tests again"'
	app:hover(second)
	app:frames(10)
	app:click(`{second} >> role:button "Remove"`)
	app:frames(10)
	expect(app:state("calls")).to_contain("queue:s1:remove:q2")
	local first = 'role:listitem "1. Then update the changelog"'
	app:hover(first)
	app:frames(10)
	app:click(`{first} >> role:button "Edit"`)
	app:frames(10)
	expect(app:state("calls")).to_contain("queue_edit:s1:q1")
	expect(app:tree(first)).to_contain("Editing in composer")
	app:key("Escape")
	app:frames(10)
	patch(app, { queue = TWO })
	app:click(`{first} >> role:button "Steer"`)
	app:frames(10)
	expect(app:state("calls")).to_contain("queue:s1:send:q1")
	expect(app:error()).to_be(nil)
end)

test("a queued message that fails keeps its row, and its Steer tries again", function()
	local app = chat()
	app:command("queue_one")
	app:frames(30)
	app:click(`{PILL} >> role:button "Open Queue"`)
	app:frames(30)
	app:command("steer_fails")
	app:click('role:listitem "1. Queued words" >> role:button "Steer"')
	app:frames(30)
	expect(app:has('~"Couldn\'t send that message"')).to_be(true)
	expect(app:has('role:listitem "1. Queued words"')).to_be(true)
	app:click('role:listitem "1. Queued words" >> role:button "Steer"')
	app:frames(30)
	local _, sends = string.gsub(app:state("calls"), "queue:s1:send:q1", "")
	expect(sends).to_be(2)
	expect(app:error()).to_be(nil)
end)

test("the composer holds still as the deck comes, opens and folds; the pill row stands clear above it", function()
	local app = chat()
	app:command("task_running")
	app:frames(30)
	local was = top(app)
	app:command("queue_one")
	for _ = 1, 20 do
		app:frames(1)
		expect(math.abs(top(app) - was) <= 1).to_be(true)
	end
	app:frames(30)
	local _, pill_y = box(app, PILL)
	local _, row_y, _, row_h = box(app, "#composer-pills")
	expect(row_y + row_h <= pill_y + 1).to_be(true)
	app:click(`{PILL} >> role:button "Open Queue"`)
	for _ = 1, 30 do
		app:frames(1)
		expect(math.abs(top(app) - was) <= 1).to_be(true)
	end
	app:click('role:tab "Fold Queue"')
	for _ = 1, 30 do
		app:frames(1)
		expect(math.abs(top(app) - was) <= 1).to_be(true)
	end
	expect(app:error()).to_be(nil)
end)

test("a row held on another device reads so in the deck, its action off", function()
	local app = chat()
	patch(app, { queue = { { id = "q1", text = "Words", locked = "Being edited on another device" } } })
	app:click(`{PILL} >> role:button "Open Queue"`)
	app:frames(30)
	local r = 'role:listitem "1. Words"'
	expect(app:tree(r)).to_contain("Being edited on another device")
	expect(app:tree(`{r} >> role:button "Send now"`)).to_contain("disabled")
	app:click(`{r} >> role:button "Send now"`)
	app:frames(5)
	expect(app:state("calls")).never.to_contain("queue:s1:send")
end)

test("with the deck there, the caption still says the agent can't be steered", function()
	local app = chat()
	app:set_state("settings_follow_up", "steer")
	patch(app, { steers = false, queue = { { id = "q1", text = "Words" } } })
	app:type('role:textbox "Message"', "One more thing")
	app:frames(30)
	expect(app:has(PILL)).to_be(true)
	expect(app:has('~"This agent can\'t be steered mid-turn"')).to_be(true)
end)
