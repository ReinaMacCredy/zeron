-- A chat's compactions as gpui's transcript tells them (ds/compaction,
-- crates/ui/src/transcript/compaction_card.rs), on the chat fixture's long
-- chat: the rows the host publishes for an engine's compaction entries
-- (src/chat.rs's tests prove their shape), the wait the session says, the
-- composer's chip and resume card, the ring's card and its next compaction.

local function settle(app: any)
	app:frames(70)
end

local function opened(): any
	local app = mount("chat_fixture.luau")
	app:resize(1100, 800)
	app:command("go_s1")
	settle(app)
	return app
end

local function row(id: string, fields: { [string]: any }): any
	local m: { [string]: any } = { id = id, role = "system", line = "compaction", text = fields.text or "" }
	for k, v in fields do
		m[k] = v
	end
	return m
end

local function push(app: any, rows: { any })
	app:set_state("test_rows", rows)
	app:command("push_rows")
	settle(app)
end

local function patch(app: any, fields: { [string]: any })
	app:set_state("test_patch", fields)
	app:command("patch_open")
	settle(app)
end

local function wait_for(app: any, wait: any)
	patch(app, { compaction_wait = wait or false })
end

local function run_for(app: any, seconds: number)
	local began = os.clock()
	while os.clock() - began < seconds do
		app:frames(20)
	end
end

test("compaction: each landed divider says who compacted in the entry's words, its sizes and how long; tokens, never dollars", function()
	local app = opened()
	push(app, {
		row("c1", { text = "Agent compacted", kept = { pre = 245561, post = 100904, ms = 49000 } }),
		row("c2", { text = "Compacted while idle, before the prompt cache expired", kept = { pre = 242575, post = 71000, ms = 41000 } }),
		row("c3", { text = "Compacted, window full", kept = { pre = 567000, post = 80000, ms = 52000 } }),
		row("c4", { text = "Context compacted", kept = { pre = 120000, post = 40200, ms = 65000 } }),
		row("c5", { kept = {} }),
	})
	local tree = app:tree("#transcript")
	for _, w in {
		"Agent compacted",
		"· 245.6k → 100.9k · 49s",
		"Compacted while idle, before the prompt cache expired",
		"· 242.6k → 71.0k · 41s",
		"Compacted, window full",
		"· 567.0k → 80.0k · 52s",
		"Context compacted",
		"· 120.0k → 40.2k · 1m 5s",
		"Compacted",
	} do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	expect(tree:find("$", 1, true)).to_be(nil)
	expect(app:error()).to_be(nil)
end)

-- A node's box in the tree: x, y, w, h of the first line matching `pat`.
local function box(app: any, scope: string, pat: string): (number?, number?, number?, number?)
	for line in (if scope ~= "" then app:tree(scope) else app:tree()):gmatch("[^\n]+") do
		if line:find(pat) then
			local x, y, w, h = line:match(" (%-?%d+),(%-?%d+) (%d+)x(%d+)")
			if x then
				return tonumber(x), tonumber(y), tonumber(w), tonumber(h)
			end
		end
	end
	return nil, nil, nil, nil
end

local function clocks(app: any): number
	app:command("compaction_clocks")
	return app:state("test_clocks")
end

test("compaction: the wait stands in the trailer's place, queued then compacting, a clock only while it runs", function()
	local app = opened()
	push(app, { { id = "u99", role = "user", text = "Next: the report", waiting = "sending", since = 0 } })
	expect(app:wait('~"Sending…"', { timeout_ms = 2000 })).to_be(true)
	wait_for(app, { phase = "queued" })
	local tree = app:tree("#compaction-wait")
	expect(tree:find("Compaction queued", 1, true) ~= nil).to_be(true)
	expect(tree:find("· starts when the turn settles", 1, true) ~= nil).to_be(true)
	expect(app:has('~"Sending…"')).to_be(false)
	expect(clocks(app)).to_be(0)
	wait_for(app, { phase = "compacting" })
	tree = app:tree("#compaction-wait")
	expect(tree:find("Compacting", 1, true) ~= nil).to_be(true)
	expect(tree:find("· 0:00", 1, true) ~= nil).to_be(true)
	expect(app:has("#working-clock")).to_be(false)
	expect(clocks(app)).to_be(1)
	run_for(app, 2.2)
	expect(app:tree("#compaction-wait"):find("· 0:02", 1, true) ~= nil).to_be(true)
	wait_for(app, nil)
	app:frames(20)
	expect(app:has("#compaction-wait")).to_be(false)
	expect(app:has('~"Sending…"')).to_be(true)
	expect(clocks(app)).to_be(0)
	expect(app:error()).to_be(nil)
end)

test("compaction: the queued divider stays whole: its words between both rules, nothing cut", function()
	local app = opened()
	wait_for(app, { phase = "queued" })
	local x, _, w = box(app, "#compaction-wait", "#compaction%-wait")
	local lx, _, lw = box(app, "#compaction-wait", " status ")
	expect(x ~= nil and lx ~= nil).to_be(true)
	local left, right = (lx :: number) - (x :: number), ((x :: number) + (w :: number)) - ((lx :: number) + (lw :: number))
	expect(left >= 24 and right >= 24).to_be(true)
	expect(math.abs(left - right) <= 2).to_be(true)
	app:resize(560, 800)
	settle(app)
	x, _, w = box(app, "#compaction-wait", "#compaction%-wait")
	lx, _, lw = box(app, "#compaction-wait", " status ")
	left, right = (lx :: number) - (x :: number), ((x :: number) + (w :: number)) - ((lx :: number) + (lw :: number))
	expect(`narrow {left},{right}`).to_be(`narrow {left},{left}`)
	expect(left >= 24).to_be(true)
end)

test("compaction: the panel: what the agent was asked to keep and its summary, opened and closed by a click or Enter, kept by a click elsewhere", function()
	local app = opened()
	push(app, {
		row("c1", { text = "Agent compacted", kept = { pre = 245561, post = 100904, ms = 49000, focus = "The cart.ts API and the coupon work", summary = "Checkout is done; coupons are next." } }),
		row("c2", { text = "Compacted while idle, before the prompt cache expired", kept = { pre = 242575, post = 71000, summary = "The plan, step 3 next." } }),
		row("c3", { text = "Context compacted", kept = { pre = 90000, post = 30000 } }),
	})
	expect(app:has('~"Asked to keep"')).to_be(false)
	expect(app:has('~"Checkout is done; coupons are next."')).to_be(false)
	app:click("#c1-head")
	settle(app)
	local tree = app:tree("#transcript")
	for _, w in { "Asked to keep", "The cart.ts API and the coupon work", "Summary", "Checkout is done; coupons are next." } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	for _, w in { "CLASSIFIER", "Carried over", "WHAT THE AGENT KEPT" } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: false`)
	end
	local _, _, _, open_h = box(app, "#transcript", "#c1%-panel")
	expect((open_h or 0) > 30).to_be(true)
	app:scroll("#transcript", -4000)
	app:frames(30)
	app:scroll("#transcript", 8000)
	app:frames(1)
	local _, _, _, again_h = box(app, "#transcript", "#c1%-panel")
	expect(again_h).to_be(open_h)
	settle(app)
	app:click('#chat-composer >> role:textbox "Message"')
	settle(app)
	expect(app:has('~"Checkout is done; coupons are next."')).to_be(true)
	app:click("#c1-head")
	settle(app)
	expect(app:has('~"Checkout is done; coupons are next."')).to_be(false)
	local _, _, _, closed_h = box(app, "#transcript", "#c1%-panel")
	expect(closed_h or 0).to_be(0)
	app:key("Enter")
	settle(app)
	expect(app:has('~"Checkout is done; coupons are next."')).to_be(true)
	app:key("Enter")
	settle(app)
	expect(app:has('~"Checkout is done; coupons are next."')).to_be(false)
	app:click("#c2-head")
	settle(app)
	tree = app:tree("#transcript")
	expect(tree:find("The plan, step 3 next.", 1, true) ~= nil).to_be(true)
	expect(tree:find("Asked to keep", 1, true)).to_be(nil)
	app:click("#c3-head")
	settle(app)
	local _, _, _, c3_h = box(app, "#transcript", "#c3%-panel")
	expect(c3_h or 0).to_be(0)
	expect(app:error()).to_be(nil)
end)

local function put(app: any, rows: { any })
	app:set_state("test_rows", rows)
	app:command("put_rows")
	settle(app)
end

test("compaction: what the chat continued with joins its row: · Continued, the note in the panel, no bubble; a limit reset's resume too", function()
	local app = opened()
	local NOTE = "Build the checkout page; reuse the cart's price formatter."
	local landed = row("c1", { text = "Agent compacted", kept = { pre = 406500, post = 19100, ms = 53000, summary = "Cart merged." } })
	local note = { id = "u-note", role = "user", text = NOTE, from_chat = "s1", waiting = "sending", since = 0 }
	push(app, { landed, note })
	expect(app:has("#c1-head-continued")).to_be(false)
	expect(app:tree("#transcript"):find(NOTE, 1, true) ~= nil).to_be(true)
	local told = table.clone(landed)
	told.continued = { note = NOTE }
	local folded = table.clone(note)
	folded.continued_on = "c1"
	put(app, { told, folded })
	expect(app:wait('~"Sending…"', { timeout_ms = 2000 })).to_be(true)
	expect(app:tree("#c1-head-continued")).to_contain("Continued")
	local _, _, w = box(app, "#transcript", "#c1%-head%-continued")
	expect((w or 0) > 40).to_be(true)
	local tree = app:tree("#transcript")
	expect(tree:find(NOTE, 1, true)).to_be(nil)
	expect(tree:find("Auto-continue", 1, true)).to_be(nil)
	app:click("#c1-head")
	settle(app)
	tree = app:tree("#c1-panel")
	for _, w in { "Summary", "Cart merged.", "Continued with", NOTE } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end

	app:scroll("#transcript", -1800)
	settle(app)
	local _, reading = box(app, "#transcript", "#u20 article")
	expect(reading ~= nil).to_be(true)
	push(app, {
		row("c9", { text = "Compacted at limit reset", limit_reset = true, kept = { pre = 300000, post = 20000, focus = "The engine's words" }, continued = { limit = true, at = "2:00 PM" } }),
		{ id = "r9", role = "system", line = "resumed", text = "", continued_on = "c9" },
	})
	local _, still_reading = box(app, "#transcript", "#u20 article")
	expect(`{still_reading}`).to_be(`{reading}`)
	app:scroll("#transcript", 8000)
	settle(app)
	tree = app:tree("#transcript")
	expect(tree:find("Compacted at limit reset", 1, true)).to_be(nil)
	expect(app:tree("#c9-head-continued")).to_contain("Continued after limit reset")
	expect(tree:find("Resumed", 1, true)).to_be(nil)
	app:click("#c9-head")
	settle(app)
	tree = app:tree("#c9-panel")
	for _, w in { "Limit reset", "2:00 PM" } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	expect(tree:find("Asked to keep", 1, true)).to_be(nil)
	expect(app:error()).to_be(nil)
end)

test("compaction: a compaction never moves the reading place or the composer", function()
	local app = opened()
	app:scroll("#transcript", -1800)
	settle(app)
	local _, cy = box(app, "#transcript", "#u20 article")
	local _, comp_before = box(app, "", "#chat%-composer ")
	expect(cy ~= nil).to_be(true)
	wait_for(app, { phase = "queued" })
	wait_for(app, { phase = "compacting" })
	push(app, { row("c7", { text = "Agent compacted", kept = { pre = 212000, post = 69000, ms = 53000, summary = "Refunds are done." } }) })
	wait_for(app, nil)
	local _, y_after = box(app, "#transcript", "#u20 article")
	local _, comp_after = box(app, "", "#chat%-composer ")
	expect(`{y_after}`).to_be(`{cy}`)
	expect(`{comp_after}`).to_be(`{comp_before}`)
	expect(app:error()).to_be(nil)
end)

test("compaction: a message that waits out a compaction shows as its row, not only in the folded pill", function()
	local app = opened()
	patch(app, { running = true, queue = { { id = "q1", text = "Queued in the turn" } } })
	expect(app:has('role:button "Open Queue"')).to_be(true)
	patch(app, { compacting = true, compaction_wait = { phase = "compacting" } })
	expect(app:has('role:button "Open Queue"')).to_be(false)
	expect(app:tree("#chat-composer-dock"):find("Sends after compaction", 1, true) ~= nil).to_be(true)
	patch(app, { compacting = false, compaction_wait = false, queue = {} })
	expect(app:has('role:listitem "1. Queued in the turn"')).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("compaction: the composer's chip says Compacting while it compacts, none while queued", function()
	local app = opened()
	wait_for(app, { phase = "queued" })
	expect(app:has("#composer-compacting")).to_be(false)
	wait_for(app, { phase = "compacting" })
	app:frames(60)
	expect(app:tree("#composer-compacting"):find("Compacting", 1, true) ~= nil).to_be(true)
	wait_for(app, nil)
	app:frames(20)
	expect(app:has("#composer-compacting")).to_be(false)
end)

test("compaction: the resume card on a large idle Claude chat: Keep full history puts it away for this quiet spell only", function()
	local app = opened()
	expect(app:has("#composer-resume")).to_be(false)
	local function offer(at: string)
		patch(app, { resume = { tokens = "184K", idle = "2h 25m", at = at } })
	end
	offer("t1")
	local tree = app:tree("#composer-resume")
	expect(tree:find("Resume with less context", 1, true) ~= nil).to_be(true)
	expect(tree:find("184K tokens from 2h 25m ago · cache expired", 1, true) ~= nil).to_be(true)
	expect(app:has('#composer-resume >> role:button "Compact"')).to_be(true)
	app:click('#composer-resume >> role:button "Keep full history"')
	settle(app)
	expect(app:has("#composer-resume")).to_be(false)
	offer("t1")
	expect(app:has("#composer-resume")).to_be(false)
	offer("t2")
	expect(app:has("#composer-resume")).to_be(true)
	patch(app, { resume = false })
	expect(app:has("#composer-resume")).to_be(false)
end)

test("compaction: Find finds a compaction by its label's words", function()
	local app = opened()
	push(app, {
		row("c1", { text = "Agent compacted", kept = { pre = 245561, post = 100904, ms = 49000 } }),
		row("c2", { text = "Context compacted", kept = { pre = 120000, post = 40200, ms = 65000 } }),
	})
	app:command("find_open")
	settle(app)
	app:type('role:textbox "Find"', "Agent compacted")
	settle(app)
	expect(app:tree("#chat-find"):match('text "(%d+ of %d+)"')).to_be("1 of 1")
	app:fill('role:textbox "Find"', "120.0k → 40.2k")
	settle(app)
	expect(app:tree("#chat-find"):match('text "(%d+ of %d+)"')).to_be("1 of 1")
end)

test("compaction: a /context card: its bar's legend with the buffer and what is free, and its actions", function()
	local app = opened()
	push(app, {
		row("cc", {
			line = "claude_command",
			text = "Claude Code /context",
			context = {
				model = "claude-opus-5-5",
				pill = "61.2K / 200K · 30.6%",
				scale = 2,
				parts = {
					{ name = "System prompt", tokens = "3.1K", percent = "1.6%", w = 0.03, ink = "system" },
					{ name = "Messages", tokens = "40K", percent = "20.0%", w = 0.4, ink = "messages" },
					{ name = "Autocompact buffer", tokens = "33K", percent = "16.5%", w = 0.33, ink = "buffer" },
				},
				free = { name = "Free", tokens = "106K", percent = "53.0%", w = 0, ink = "free" },
				tip = "Messages take 40K.",
			},
		}),
	})
	local tree = app:tree("#transcript")
	for _, w in { "Context", "Claude Code · claude-opus-5-5", "61.2K / 200K · 30.6%", "System prompt", "Autocompact buffer", "Free", "106K", "53.0%", "Messages take 40K.", "Compact now", "Context settings" } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	expect(app:error()).to_be(nil)
end)

local WINDOW = { context_tokens = 84000, context_window = 200000 }

local function card(app: any)
	patch(app, WINDOW)
	app:hover("#composer-context")
	app:wait("#composer-context-card", { timeout_ms = 2000 })
	settle(app)
end

local function told(app: any, fields: { [string]: any })
	local all = table.clone(WINDOW)
	for k, v in fields do
		all[k] = v
	end
	patch(app, all)
	app:hover('role:textbox "Message"')
	settle(app)
	app:hover("#composer-context")
	app:wait("#composer-context-card", { timeout_ms = 2000 })
	settle(app)
end

local IDLE = { icon = "clock", title = "Idle · 3:42 PM", detail = "A message from you first cancels it", chip = "compacts 3:42 PM" }

test("compaction: the context card marks the bar where the agent is reminded, warned and compacted, only those the host names", function()
	local app = opened()
	patch(app, { context_marks = { floor = 120000, warn = 142000, auto = 167000 } })
	card(app)
	local tree = app:tree("#composer-context-card")
	for _, w in { "New-task reminder from 120K", "Agent warned at 142K", "Compacts on its own at 167K" } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	local bx, _, bw = box(app, "#composer-context-card", 'status "Context window')
	expect(bx ~= nil).to_be(true)
	for _, m in { { '"New%-task reminder from 120K"', 0.6 }, { '"Agent warned at 142K"', 0.71 }, { '"Compacts on its own at 167K"', 0.835 } } do
		local x, _, w = box(app, "#composer-context-card", m[1])
		local at = ((x :: number) + (w :: number) / 2 - (bx :: number)) / (bw :: number)
		expect(`{m[1]} {math.abs(at - m[2]) <= 0.01}`).to_be(`{m[1]} true`)
	end
	told(app, { context_marks = { auto = 167000 } })
	tree = app:tree("#composer-context-card")
	expect(tree:find("Compacts on its own at 167K", 1, true) ~= nil).to_be(true)
	expect(tree:find("New-task reminder", 1, true)).to_be(nil)
	expect(tree:find("Agent warned", 1, true)).to_be(nil)
	told(app, { context_marks = false })
	expect(app:tree("#composer-context-card"):find("Compacts on its own", 1, true)).to_be(nil)
	expect(app:error()).to_be(nil)
end)

test("compaction: the context card says what compacts the chat next, and offers Compact now only while it is idle and nothing compacts", function()
	local app = opened()
	patch(app, { next_compaction = IDLE })
	card(app)
	expect(app:text("#composer-context-next-title")).to_be("Idle · 3:42 PM")
	expect(app:text("#composer-context-next-detail")).to_be("A message from you first cancels it")
	expect(app:has('#composer-context-next >> "Next compaction"')).to_be(true)
	expect(app:has('#composer-context-next >> role:button "Compact now"')).to_be(true)
	told(app, { next_compaction = { icon = "warn", title = "Claude Code · at 167K", detail = "The turn carries on after it" }, running = true })
	expect(app:text("#composer-context-next-title")).to_be("Claude Code · at 167K")
	expect(app:has('#composer-context-next >> role:button "Compact now"')).to_be(false)
	told(app, { next_compaction = { icon = "spin", title = "Compacting now" }, running = false, compacting = true, compaction_wait = { phase = "compacting" } })
	expect(app:text("#composer-context-next-title")).to_be("Compacting now")
	expect(app:has("#composer-context-next-detail")).to_be(false)
	expect(app:has('#composer-context-next >> role:button "Compact now"')).to_be(false)
	told(app, { next_compaction = { icon = "off", title = "Not scheduled", detail = "Context is under 100K" }, compacting = false, compaction_wait = false })
	expect(app:has('#composer-context-next >> role:button "Compact now"')).to_be(true)
	told(app, { next_compaction = false })
	expect(app:has("#composer-context-next")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("compaction: the footer names the next compaction left of the ring, opening to its width and closing as it goes", function()
	local app = opened()
	patch(app, WINDOW)
	local _, _, closed_w = box(app, "", "#composer%-context%-next%-chip")
	expect(closed_w or 0).to_be(0)
	patch(app, { next_compaction = IDLE, context_tokens = 84000, context_window = 200000 })
	expect(app:text("#composer-context-next-chip")).to_be("compacts 3:42 PM ·")
	local x, _, w = box(app, "", "#composer%-context%-next%-chip")
	local rx = box(app, "", "#composer%-context ")
	expect((w or 0) > 40).to_be(true)
	expect((x :: number) + (w :: number) <= (rx :: number) + 1).to_be(true)
	patch(app, { next_compaction = { icon = "off", title = "Not scheduled", detail = "The agent is working" }, context_tokens = 84000, context_window = 200000 })
	_, _, w = box(app, "", "#composer%-context%-next%-chip")
	expect(w or 0).to_be(0)
	expect(app:error()).to_be(nil)
end)
