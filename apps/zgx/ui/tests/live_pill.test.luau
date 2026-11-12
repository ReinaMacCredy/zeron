local MARKS = { "claude", "openai", "cursor", "grok", "pi", "devin" }
local made = 0
local function chat(id: string, status: string?, extra: any?): any
	made += 1
	local c = { id = id, title = id, device = "d", status = status, updated = 100, created = 100, harness = "mock", mark = MARKS[(made - 1) % #MARKS + 1] }
	for k, v in extra or {} do
		c[k] = v
	end
	return c
end

local ASK = {
	key = "asks:r1",
	chat = "asks",
	request = "r1",
	kind = "ask",
	ask = { kind = "question", question = { text = "3.1 or 3.0?", options = { { value = "3.1", label = "3.1" }, { value = "3.0", label = "3.0" } }, multi = false, page = 1, pages = 1 } },
	steps = {},
}
local PLAN = { key = "plans:r2", chat = "plans", request = "r2", kind = "plan", ask = { kind = "approval", approval = { title = "Approve?" } }, steps = { "One", "Two" } }

local function open(chats: { any }, waiting: { any }?): any
	local app = mount("live_pill_fixture.luau")
	app:resize(900, 700)
	app:set_state("chat.waiting", waiting or {})
	app:set_state("sessions", { state = "ready", devices = {}, projects = {}, chats = chats, archived = {}, places = { pinned = {}, sections = {} }, places_ready = true })
	app:frames(10)
	return app
end

test("nothing working or waiting shows no pill; working shows its count", function()
	local app = open({ chat("idle", nil), chat("seen", "done") })
	expect(app:has("#live-pill")).to_be(false)
	app:set_state("sessions", { state = "ready", devices = {}, projects = {}, chats = { chat("w1", "working"), chat("w2", "waiting") }, archived = {}, places = { pinned = {}, sections = {} }, places_ready = true })
	app:frames(10)
	expect(app:has('#live-pill-count >> "2"')).to_be(true)
	expect(app:has('role:button "2 working"')).to_be(true)
end)

test("what needs you counts first and wears its marks in front, three at most", function()
	local app = open({ chat("w1", "working"), chat("asks", "input"), chat("plans", "input"), chat("broke", "failed"), chat("w2", "working") }, { ASK, PLAN })
	expect(app:has('#live-pill-count >> "3"')).to_be(true)
	expect(app:has('role:button "3 need you · 2 working"')).to_be(true)
	expect(app:has("#live-mark-3")).to_be(true)
	expect(app:has("#live-mark-4")).to_be(false)
end)

test("the tray answers in place, approves a plan, and dismisses or retries a failed run", function()
	local app = open({ chat("asks", "input"), chat("plans", "input"), chat("broke", "failed"), chat("w1", "working") }, { ASK, PLAN })
	app:click("#live-pill")
	app:frames(10)
	expect(app:has('"Needs you"')).to_be(true)
	expect(app:has('"3.1 or 3.0?"')).to_be(true)
	expect(app:has("#live-options-asks")).to_be(false)
	app:click("#live-answer-asks")
	app:frames(10)
	app:click("#live-option-asks-2")
	app:frames(5)
	app:click("#live-approve-plans")
	app:click("#live-dismiss-broke")
	app:click("#live-retry-broke")
	app:frames(5)
	expect(app:state("calls")).to_contain("answer:asks:r1:answer:3.0")
	expect(app:state("calls")).to_contain("answer:plans:r2:once:")
	expect(app:state("calls")).to_contain("seen:broke")
	expect(app:state("calls")).to_contain("send:broke:Resume without asking to continue.")
end)

test("a thread that finishes comes to the front with a check for a moment", function()
	local app = open({ chat("a", "working"), chat("b", "working") })
	app:set_state("sessions", { state = "ready", devices = {}, projects = {}, chats = { chat("a", "working"), chat("b", "done") }, archived = {}, places = { pinned = {}, sections = {} }, places_ready = true })
	app:frames(5)
	local flash = app:state("shell_live_flash")
	expect(flash ~= nil and flash.id == "b").to_be(true)
	app:set_state("sessions", { state = "ready", devices = {}, projects = {}, chats = { chat("a", "input"), chat("b", "done") }, archived = {}, places = { pinned = {}, sections = {} }, places_ready = true })
	app:frames(5)
	expect(app:state("shell_live_flash").id).to_be("b")
end)
