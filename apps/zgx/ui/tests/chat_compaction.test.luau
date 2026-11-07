-- A chat's compactions as gpui's transcript tells them (ds/compaction,
-- crates/ui/src/transcript/compaction_card.rs), on the chat fixture's long
-- chat: the rows the host publishes for an engine's classifier and
-- compaction entries (src/chat.rs's tests prove their shape), the wait the
-- session says, the composer's chip and resume card, the ring's card.

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

local function decision(verdict: string, extra: { [string]: any }?): any
	local d: { [string]: any } = {
		verdict = verdict,
		by = "stage1",
		ms = 1670,
		context = 212000,
		input = "person: refund the order\nagent: done",
		reply = "CARRY",
		tokens_in = 2300,
		tokens_out = 4,
		agent = "Claude Code",
		model = "claude-sonnet-5-5",
		mark = "claude",
	}
	for k, v in extra or {} do
		d[k] = v
	end
	return d
end

local KEPT = { pre = 212000, post = 69000, ms = 53000, summary = "Refunds are done; the tests pass.", carried = "person: next?\nagent: the report", turns = 1 }

local function row(id: string, line: string, fields: { [string]: any }): any
	local m: { [string]: any } = { id = id, role = "system", line = line, text = fields.text or "" }
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

local function wait_for(app: any, wait: any)
	app:set_state("test_patch", { compaction_wait = wait or false })
	app:command("patch_open")
	settle(app)
end

local function words(app: any, sel: string): string
	return app:tree(sel)
end

test("compaction: each outcome's divider says it in gpui's words, tokens and never dollars", function()
	local app = opened()
	push(app, {
		row("k1", "classifier", { decision = decision("carry"), kept = KEPT }),
		row("k2", "classifier", { decision = decision("continue") }),
		row("k3", "classifier", { decision = decision("failed") }),
		row("k4", "classifier", { decision = decision("new") }),
		row("k5", "classifier", { decision = decision("carry", { by = "background_copy" }), kept = { pre = 212000, post = 69000, carried = "person: a\nagent: b", turns = 1 } }),
		row("k6", "classifier", { decision = decision("carry", { by = "streak" }), kept = { pre = 180000, post = 40000, ms = 30000 } }),
		row("c1", "compaction", { text = "Context compacted", kept = { pre = 120000, post = 40200, ms = 65000 } }),
		row("c2", "compaction", { text = "Compacted while idle, before the prompt cache expired", kept = {} }),
	})
	local tree = words(app, "#transcript")
	for _, w in {
		"Compacted",
		"Carry over",
		"· 212.0k → 69.0k · 53s",
		"Not compacted",
		"· 212.0k · context kept",
		"· 212.0k · the classifier gave no answer",
		"Same task",
		"No answer",
		"Classifier",
		"New task",
		"· 212.0k · 1.7s",
		"Compacted in the background",
		"Swapped in",
		"· 212.0k → 69.0k · 1 turn as text",
		"· 180.0k → 40.0k · 30s",
		"· 120.0k → 40.2k · 1m 5s",
		"Compacted while idle, before the prompt cache expired",
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

test("compaction: the wait stands in the trailer's place through each phase, a clock only while one runs", function()
	local app = opened()
	push(app, { { id = "u99", role = "user", text = "Next: the report", waiting = "sending", since = 0 } })
	expect(app:wait('~"Sending…"', { timeout_ms = 2000 })).to_be(true)
	wait_for(app, { phase = "checking", agent = "Claude Code", model = "claude-sonnet-5-5", mark = "claude" })
	local tree = app:tree("#compaction-wait")
	expect(tree:find("Checking whether to compact", 1, true) ~= nil).to_be(true)
	expect(tree:find("claude-sonnet-5-5", 1, true) ~= nil).to_be(true)
	expect(tree:find("· 0s", 1, true) ~= nil).to_be(true)
	expect(app:has('~"Sending…"')).to_be(false)
	expect(app:has("#working-clock")).to_be(false)
	expect(clocks(app)).to_be(1)
	wait_for(app, { phase = "checking", agent_note = true, agent = "Claude Code" })
	expect(app:tree("#compaction-wait"):find("Checking the agent's note", 1, true) ~= nil).to_be(true)
	wait_for(app, { phase = "compacting", tag = "new", tokens = 212000, usual = 53 })
	tree = app:tree("#compaction-wait")
	for _, w in { "Compacting", "New task", "· 212.0k · 0s, usually ~53s" } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	wait_for(app, { phase = "compacting" })
	expect(app:tree("#compaction-wait"):find("· 0s", 1, true) ~= nil).to_be(true)
	wait_for(app, { phase = "queued" })
	tree = app:tree("#compaction-wait")
	expect(tree:find("Compaction queued", 1, true) ~= nil).to_be(true)
	expect(tree:find("· starts when the turn settles", 1, true) ~= nil).to_be(true)
	expect(clocks(app)).to_be(0)
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

test("compaction: a decision that compacts shows it on its own row, then Compacted with its sizes", function()
	local app = opened()
	push(app, {
		row("k9", "classifier", { decision = decision("carry") }),
		{ id = "u9", role = "user", text = "Then the report" },
	})
	wait_for(app, { phase = "compacting", tag = "carry", tokens = 212000, row = "k9", usual = 53 })
	local tree = app:tree("#transcript")
	expect(tree:find("Compacting", 1, true) ~= nil).to_be(true)
	expect(tree:find("· 212.0k · 0s, usually ~53s", 1, true) ~= nil).to_be(true)
	expect(app:has("#compaction-wait")).to_be(false)
	expect(clocks(app)).to_be(1)
	app:set_state("test_row_patch", { id = "k9", kept = KEPT })
	app:command("patch_row")
	wait_for(app, nil)
	tree = app:tree("#transcript")
	expect(tree:find("· 212.0k → 69.0k · 53s", 1, true) ~= nil).to_be(true)
	expect(tree:find("usually", 1, true)).to_be(nil)
	expect(clocks(app)).to_be(0)
	expect(app:error()).to_be(nil)
end)

test("compaction: the open panel: a click or Enter opens and closes it, a click elsewhere keeps it", function()
	local app = opened()
	push(app, {
		row("k1", "classifier", {
			decision = decision("carry", { said = "continue", by = "stage2", stage2 = "CARRY", reply = "CONTINUE", reason = "Same task, the last turns matter" }),
			kept = KEPT,
		}),
	})
	expect(app:has('~"WHAT THE AGENT KEPT"')).to_be(false)
	app:click("#k1-head")
	settle(app)
	local tree = app:tree("#transcript")
	for _, w in {
		"WHAT THE AGENT KEPT",
		"Summary the agent wrote",
		"Refunds are done; the tests pass.",
		"Carried over word for word",
		"CLASSIFIER",
		"Carry over (said Same task)",
		"Same task, the last turns matter",
		"Claude Code · claude-sonnet-5-5 · Second look · 1.7s",
		"212.0k tokens",
		"2.3k in · 4 out",
		"What the classifier saw",
	} do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	app:click('role:button "What the classifier saw"')
	settle(app)
	tree = app:tree("#transcript")
	expect(tree:find("First look", 1, true) ~= nil and tree:find("Second look", 1, true) ~= nil).to_be(true)
	expect(tree:find("person: refund the order", 1, true) ~= nil).to_be(true)
	app:click('#chat-composer >> role:textbox "Message"')
	settle(app)
	expect(app:has('~"WHAT THE AGENT KEPT"')).to_be(true)
	app:click("#k1-head")
	settle(app)
	expect(app:has('~"WHAT THE AGENT KEPT"')).to_be(false)
	app:key("Enter")
	settle(app)
	expect(app:has('~"WHAT THE AGENT KEPT"')).to_be(true)
	app:key("Enter")
	settle(app)
	expect(app:has('~"WHAT THE AGENT KEPT"')).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("compaction: a plain divider opens what the agent kept; one without a summary doesn't open", function()
	local app = opened()
	push(app, {
		row("c1", "compaction", { text = "Agent compacted", kept = { pre = 120000, post = 40200, ms = 65000, summary = "The plan, step 3 next." } }),
		row("c2", "compaction", { text = "Context compacted", kept = { pre = 90000, post = 30000 } }),
	})
	app:click("#c1-head")
	settle(app)
	local tree = app:tree("#transcript")
	expect(tree:find("The plan, step 3 next.", 1, true) ~= nil).to_be(true)
	expect(tree:find("Nothing carried word for word: the next turn starts from the", 1, true) ~= nil).to_be(true)
	expect(tree:find("CLASSIFIER", 1, true)).to_be(nil)
	app:click("#c2-head")
	settle(app)
	expect(app:has(".compaction-panel")).to_be(true)
	local _, count = string.gsub(app:tree("#transcript"), "WHAT THE AGENT KEPT", "")
	expect(count).to_be(1)
end)

test("compaction: a compaction never moves the reading place or the composer", function()
	local app = opened()
	app:scroll("#transcript", -1800)
	settle(app)
	local _, y_before = box(app, "#transcript", "#u20 article")
	local _, cy = box(app, "#transcript", "#u20 article")
	local _, comp_before = box(app, "", "#chat%-composer ")
	expect(y_before ~= nil).to_be(true)
	wait_for(app, { phase = "queued" })
	wait_for(app, { phase = "checking", agent = "Claude Code" })
	push(app, { row("k7", "classifier", { decision = decision("carry") }) })
	wait_for(app, { phase = "compacting", tag = "carry", tokens = 212000, row = "k7" })
	app:set_state("test_row_patch", { id = "k7", kept = KEPT })
	app:command("patch_row")
	wait_for(app, nil)
	local _, y_after = box(app, "#transcript", "#u20 article")
	local _, comp_after = box(app, "", "#chat%-composer ")
	expect(`{y_after}`).to_be(`{cy}`)
	expect(`{comp_after}`).to_be(`{comp_before}`)
	expect(app:error()).to_be(nil)
end)

test("compaction: the composer's chip says the same words while it checks or compacts, none while queued", function()
	local app = opened()
	wait_for(app, { phase = "queued" })
	expect(app:has("#composer-compacting")).to_be(false)
	wait_for(app, { phase = "checking", agent = "Claude Code" })
	expect(app:tree("#composer-compacting"):find("Checking whether to compact", 1, true) ~= nil).to_be(true)
	wait_for(app, { phase = "compacting", tag = "carry", tokens = 1 })
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
		app:set_state("test_patch", { resume = { tokens = "184K", idle = "2h 25m", at = at } })
		app:command("patch_open")
		settle(app)
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
	app:set_state("test_patch", { resume = false })
	app:command("patch_open")
	settle(app)
	expect(app:has("#composer-resume")).to_be(false)
end)

test("compaction: the ring's card: the plan's section and dot, why Compact context can't, the idle line", function()
	local app = opened()
	app:set_state("test_patch", {
		context_tokens = 212000,
		context_window = 1000000,
		compact_plan = { heading = "Agent compaction", note = "The agent asked to compact. It runs as soon as the turn ends.", rows = { { label = "Compacts", value = "when this turn ends" } }, busy = false },
		compact_blocked = "running",
		resume = { tokens = "212K", idle = "2h 25m", at = "t1" },
	})
	app:command("patch_open")
	settle(app)
	expect(app:has("#composer-context-dot")).to_be(true)
	app:click("#composer-context")
	settle(app)
	local tree = app:tree()
	for _, w in { "Agent compaction", "The agent asked to compact. It runs as soon as the turn", "Compacts", "when this turn ends", "Compact context", "Wait for the current turn to finish", "Idle for 2h 25m; the prompt cache has expired." } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	app:set_state("test_patch", { compact_plan = false, compact_blocked = false, resume = false })
	app:command("patch_open")
	settle(app)
	expect(app:has("#composer-context-dot")).to_be(false)
	expect(app:has("#compact-context-reason")).to_be(false)
end)

test("compaction: Find finds a compaction by its label's words", function()
	local app = opened()
	push(app, {
		row("k2", "classifier", { decision = decision("continue") }),
		row("c1", "compaction", { text = "Context compacted", kept = { pre = 120000, post = 40200, ms = 65000 } }),
	})
	app:command("find_open")
	settle(app)
	app:type('role:textbox "Find"', "context kept")
	settle(app)
	expect(app:tree("#chat-find"):match('text "(%d+ of %d+)"')).to_be("1 of 1")
	app:fill('role:textbox "Find"', "120.0k → 40.2k")
	settle(app)
	expect(app:tree("#chat-find"):match('text "(%d+ of %d+)"')).to_be("1 of 1")
end)

test("compaction: a /context card: its bar's legend with the buffer and what is free, and its actions", function()
	local app = opened()
	push(app, {
		row("cc", "claude_command", {
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


test("compaction: a copy compacting in the background shows its clock in the footer, then why it waits", function()
	local app = opened()
	expect(app:has("#composer-copy-chip")).to_be(false)
	app:set_state("test_patch", { compact_plan = { heading = "Background compaction", rows = {}, busy = false, copy = { since = 0, ready = false } } })
	app:command("patch_open")
	settle(app)
	expect(app:tree("#composer-copy-chip"):match('text "(Compacting a copy · %d+:%d%d)"') ~= nil).to_be(true)
	app:set_state("test_patch", { running = false, in_flight = false, compact_plan = { heading = "Background compaction", rows = {}, busy = false, copy = { since = 0, ready = true } } })
	app:command("patch_open")
	settle(app)
	expect(app:tree("#composer-copy-chip"):find("Copy ready · waits for background work", 1, true) ~= nil).to_be(true)
	app:set_state("test_patch", { compact_plan = false })
	app:command("patch_open")
	settle(app)
	expect(app:has("#composer-copy-chip")).to_be(false)
end)
