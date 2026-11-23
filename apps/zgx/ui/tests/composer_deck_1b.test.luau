-- The deck on the chat page (1B, ui/chat_fixture.luau): only the queue and
-- the plan, there only while one of them is, folded to one line over the
-- composer; what runs stays on the pill row above it.

local PILL = 'role:toolbar "What\'s waiting"'

local function box(app: any, what: string): (number, number, number, number)
	local line = app:tree(what):match("[^\n]+") or ""
	local x, y, w, h = line:match(" (%-?[%d%.]+),(%-?[%d%.]+) ([%d%.]+)x([%d%.]+)")
	return tonumber(x) or 0, tonumber(y) or 0, tonumber(w) or 0, tonumber(h) or 0
end

-- Whether two nodes' boxes stand clear of each other.
local function apart(app: any, a: string, b: string): boolean
	local ax, ay, aw, ah = box(app, a)
	local bx, by, bw, bh = box(app, b)
	return ax + aw <= bx or bx + bw <= ax or ay + ah <= by or by + bh <= ay
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

test("a thread's message in the queue brings no line until it has waited there", function()
	local app = chat()
	patch(app, { queue = { { id = "q1", text = "Step one is done", from = "c2" } } })
	expect(app:has(PILL)).to_be(false)
	app:wait("#never-shown", { timeout_ms = 1600 })
	app:frames(10)
	expect(app:tree(PILL)).to_contain("1 queued")
	patch(app, { queue = { { id = "q1", text = "Step one is done", from = "c2" }, TWO[1] } })
	expect(app:tree(PILL)).to_contain("2 queued")
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

test("the composer holds still as the deck comes, opens and folds; the pill row stands clear of it", function()
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
	expect(apart(app, "#pills-shells-pill", PILL)).to_be(true)
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

test("a row's Edit stands at rest, faint as Remove (gpui's .72), not only under the pointer", function()
	local app = chat()
	patch(app, { queue = TWO, steers = true })
	app:click(`{PILL} >> role:button "Open Queue"`)
	app:frames(30)
	app:hover('role:textbox "Message"')
	app:frames(20)
	local edit = 'role:listitem "1. Then update the changelog" >> role:button "Edit"'
	expect(app:tree(edit)).never.to_contain("hidden")
	expect(app:tree('role:listitem "1. Then update the changelog" >> role:button "Remove"')).never.to_contain("hidden")
	expect(app:error()).to_be(nil)
end)

test("the pill row rises over the deck's pill only where its pills would meet it, and the conversation keeps clear of it", function()
	-- One pill, at its left: no need to rise, so the Working row stays where it was.
	local app = chat()
	app:command("task_running")
	app:frames(30)
	local _, row_was = box(app, "#composer-pills")
	local _, working_was = box(app, "#working-clock")
	app:command("queue_one")
	app:frames(40)
	local _, row_y = box(app, "#composer-pills")
	expect(row_y).to_be(row_was)
	expect(apart(app, "#pills-shells-pill", PILL)).to_be(true)
	local _, working_y = box(app, "#working-clock")
	expect(working_y).to_be(working_was)
	expect(apart(app, "#working-clock", "#pills-shells-pill")).to_be(true)
	-- Agents and Shells across a narrow composer reach its middle: it rises,
	-- and the conversation's end rises with it.
	local narrow = mount("chat_fixture.luau")
	narrow:resize(600, 700)
	narrow:command("go_s1")
	narrow:frames(30)
	narrow:command("start_reply")
	narrow:command("task_running")
	narrow:command("subagents_running")
	narrow:frames(30)
	narrow:command("queue_one")
	narrow:frames(40)
	local _, pill_y = box(narrow, PILL)
	local _, ry, _, rh = box(narrow, "#composer-pills")
	expect(ry + rh <= pill_y).to_be(true)
	expect(apart(narrow, "#pills-shells-pill", PILL)).to_be(true)
	local _, wy, _, wh = box(narrow, "#working-clock")
	expect(wy + wh <= ry).to_be(true)
	expect(narrow:error()).to_be(nil)
end)

test("the folded pill and the pill row stand over the notice line, never on it", function()
	local app = chat()
	app:command("task_running")
	app:command("queue_one")
	app:frames(30)
	app:command("connection_offline")
	app:frames(40)
	local notice = '~"Offline — messages will send"'
	expect(app:has(notice)).to_be(true)
	expect(apart(app, PILL, notice)).to_be(true)
	expect(apart(app, "#pills-shells-pill", notice)).to_be(true)
	-- A Steer that fails: the deck's pill is up just while its card is.
	local failed = chat()
	failed:command("queue_one")
	failed:frames(30)
	failed:click(`{PILL} >> role:button "Open Queue"`)
	failed:frames(30)
	failed:command("steer_fails")
	failed:click('role:listitem "1. Queued words" >> role:button "Steer"')
	failed:frames(30)
	failed:click('role:tab "Fold Queue"')
	failed:frames(40)
	local card = '~"Couldn\'t send that message"'
	expect(failed:has(card)).to_be(true)
	expect(failed:has(PILL)).to_be(true)
	expect(apart(failed, PILL, card)).to_be(true)
	expect(apart(failed, "#composer-side-chat", card)).to_be(true)
	expect(failed:error()).to_be(nil)
end)

test("beside the side chat's tab the folded pill stops short of it, and Scroll to end still clears it", function()
	for _, w in { 520, 700 } do
		local app = mount("chat_fixture.luau")
		app:resize(w, 700)
		app:command("go_s1")
		app:frames(30)
		app:command("start_reply")
		app:frames(10)
		patch(app, { queue = { { id = "q1", text = "Words" } }, todos = plan(0, 2) })
		expect(app:tree(PILL)).to_contain("Plan 0/2")
		expect(apart(app, PILL, "#composer-side-chat")).to_be(true)
		app:scroll("#transcript", -900)
		app:frames(60)
		local jump = 'role:button "New reply"'
		expect(app:has(jump)).to_be(true)
		expect(apart(app, PILL, "#composer-side-chat")).to_be(true)
		expect(apart(app, PILL, jump)).to_be(true)
		expect(app:error()).to_be(nil)
	end
end)

-- A long chat, running, with a shell and two agents on the pill row, at a width.
local function busy(w: number): any
	local app = mount("chat_fixture.luau")
	app:resize(w, 700)
	app:command("go_s1")
	app:frames(30)
	app:command("long_reply")
	app:frames(10)
	app:command("start_reply")
	app:command("task_running")
	app:command("subagents_running")
	app:frames(40)
	return app
end

-- Whether a node's bottom stands at or over another's top.
local function over(app: any, a: string, b: string): boolean
	local _, ay, _, ah = box(app, a)
	local _, by = box(app, b)
	return ay + ah <= by
end

-- (A test for each width: a test's mounts share its storage, so one after
-- another that scrolled away opens where that one was left, not at its end.)
for _, w in { 400, 460, 480, 600 } do
	test(`at a narrow composer ({w} wide) the pills, the side chat's tab, the folded pill and Jump to latest never meet, and the conversation's end clears the pill row`, function()
		local AGENTS, SHELLS, TAB, JUMP = "#pills-agents", "#pills-shells-pill", "#composer-side-chat", 'role:button "New reply"'
		local app = busy(w)
		expect(apart(app, AGENTS, SHELLS)).to_be(true)
		expect(apart(app, AGENTS, TAB)).to_be(true)
		expect(apart(app, SHELLS, TAB)).to_be(true)
		expect(over(app, "#working-clock", "#composer-pills")).to_be(true)
		app:command("queue_one")
		app:frames(60)
		for _, pill in { AGENTS, SHELLS } do
			expect(apart(app, pill, PILL)).to_be(true)
			expect(apart(app, pill, TAB)).to_be(true)
		end
		expect(over(app, "#working-clock", "#composer-pills")).to_be(true)
		app:scroll("#transcript", -900)
		app:frames(60)
		expect(app:has(JUMP)).to_be(true)
		for _, other in { AGENTS, SHELLS, PILL, TAB } do
			expect(apart(app, JUMP, other)).to_be(true)
		end
		local left = box(app, "#composer-pills")
		expect(box(app, JUMP) >= left).to_be(true)
		expect(app:error()).to_be(nil)
	end)
end

test("Jump to latest beside a folded pill held short of the side chat's tab stays over the composer, or stands over the pill when the two don't fit", function()
	for _, w in { 400, 450, 520 } do
		local app = mount("chat_fixture.luau")
		app:resize(w, 700)
		app:command("go_s1")
		app:frames(30)
		app:command("long_reply")
		app:frames(10)
		app:command("start_reply")
		app:frames(10)
		patch(app, { queue = { { id = "q1", text = "Words" } }, todos = plan(0, 2) })
		app:scroll("#transcript", -900)
		app:frames(60)
		local jump = 'role:button "New reply"'
		expect(app:has(jump)).to_be(true)
		local left = box(app, "#composer-pills")
		expect(box(app, jump) >= left).to_be(true)
		expect(apart(app, jump, PILL)).to_be(true)
		expect(apart(app, jump, "#composer-side-chat")).to_be(true)
		-- Wider than the room before the tab (whose shortcut reads "Ctrl+;"
		-- off macOS, 27 wider than "⌘;"), the pill keeps the composer's left edge.
		local px, _, pw = box(app, PILL)
		if pw + 8 <= box(app, "#composer-side-chat") - left then
			expect(apart(app, PILL, "#composer-side-chat")).to_be(true)
		else
			expect(px).to_be(left)
		end
		expect(app:error()).to_be(nil)
	end
end)

test("Jump to latest and the folded pill fade out where they stood, not from the middle of their row", function()
	local app = mount("chat_fixture.luau")
	app:resize(1100, 700)
	app:command("go_s1")
	app:frames(30)
	app:command("start_reply")
	app:frames(10)
	patch(app, { queue = { { id = "q1", text = "Words" } } })
	app:scroll("#transcript", -900)
	app:frames(60)
	local jump = 'role:button "New reply"'
	local jx, jy = box(app, jump)
	app:scroll("#transcript", 100000)
	app:frames(3)
	expect(app:has(jump)).to_be(true)
	local lx, ly = box(app, jump)
	expect(math.abs(lx - jx) <= 1).to_be(true)
	expect(ly - jy <= 8).to_be(true)
	app:frames(60)
	local _, _, one = box(app, PILL)
	patch(app, { queue = { { id = "q1", text = "Words" } }, todos = plan(0, 2) })
	app:frames(30)
	local px, _, two = box(app, PILL)
	expect(two > one).to_be(true)
	app:click(`{PILL} >> role:button "Open Queue"`)
	app:frames(2)
	expect(app:has(PILL)).to_be(true)
	local qx = box(app, PILL)
	expect(math.abs(qx - px) <= 1).to_be(true)
	expect(app:error()).to_be(nil)
end)
