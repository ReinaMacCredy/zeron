-- Appshots (screens/chat/appshots), with the host's capture stubbed: a
-- capture the host publishes (`appshots.captured`) lands on the open
-- session's composer as a chip saying its app, opens in the lightbox, and
-- goes with the next send as a picture and its context; never sent on its
-- own; a capture that would pass 96 MB of staged Appshots says so; a
-- failure says why; and one taken in is never taken twice. On the chat page
-- of ui/chat_fixture.luau; and where it lands, by Destination.

local function settle(app: any)
	app:frames(30)
end

local function open(app: any, id: string)
	app:command(`go_{id}`)
	settle(app)
end

local SHOT = "assets/composer/shell.png"
local n = 0
local function capture(app: any, shot: any)
	n += 1
	local base = {
		id = `a{n}`,
		app = "Safari",
		bundle = "com.apple.Safari",
		title = "Release notes",
		-- (A real picture: headless, nothing asks whether a file is there.)
		path = SHOT,
		bytes = 1024,
	}
	for k, v in shot do
		base[k] = v
	end
	app:set_state("appshots.captured", { n = n, shot = base })
	settle(app)
end

test("a capture lands on the open session's composer as its app's chip, and isn't sent", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	open(app, "s2")
	capture(app, {})
	expect(app:error()).to_be_nil()
	expect(app:has('~"Release notes"')).to_be(true)
	expect(app:state("calls") or "").never.to_contain("send:")
end)

test("sent, the capture goes as a picture and its context, the words first", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	open(app, "s2")
	capture(app, { text = "Version 2 ships Tuesday." })
	app:type("role:textbox", "What changed?")
	app:key("Enter")
	settle(app)
	expect(app:wait('~"What changed?"', { timeout_ms = 1000 })).to_be(true)
	settle(app)
	local calls = app:state("calls")
	expect(calls).to_contain("send:s2:What changed?")
	expect(calls).to_contain(`att:{SHOT}|upload`)
	expect(calls).to_contain(`appshot:Safari|Release notes|{SHOT}|text`)
end)

test("a capture past 96 MB of staged Appshots says so and stages nothing", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	open(app, "s2")
	capture(app, { title = "Big one", bytes = 90 * 1024 * 1024 })
	capture(app, { title = "One too many", bytes = 10 * 1024 * 1024 })
	expect(app:has('~"Big one"')).to_be(true)
	expect(app:has('~"One too many"')).to_be(false)
	expect(app:has('~"96 MB staged Appshot limit"')).to_be(true)
end)

test("a capture that failed says why", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	open(app, "s2")
	app:set_state("appshots.failed", { n = 1, message = "Allow window capture in System Settings." })
	settle(app)
	expect(app:has('"Couldn\'t capture an Appshot"')).to_be(true)
end)

test("a capture taken in is never taken twice (a reload keeps what was taken)", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	open(app, "s2")
	capture(app, { title = "Once only" })
	local once = app:count('#chat-composer >> ~"Once only"')
	expect(once > 0).to_be(true)
	-- The same capture published again (as a reload re-reads the host's).
	app:set_state("appshots.captured", { n = n, shot = { id = "again", app = "Safari", title = "Once only", path = SHOT } })
	settle(app)
	expect(app:count('#chat-composer >> ~"Once only"')).to_be(once)
end)

test("a capture's chip opens its picture in the lightbox", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	open(app, "s2")
	capture(app, { title = "Look here" })
	app:click('#chat-composer >> ~"Look here"')
	app:frames(60)
	expect(app:error()).to_be_nil()
	expect(app:state("ds/lightbox.luau::open")).never.to_be_nil()
	app:key("Escape")
	app:frames(60)
	expect(app:state("ds/lightbox.luau::open")).to_be_nil()
end)

-- Where a capture lands, by Destination (the shipping app's rules).
test("Automatic: within a minute of a snap to a session, the next one goes back there from a new chat", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	open(app, "s2")
	capture(app, { title = "First snap" })
	open(app, "home")
	capture(app, { title = "Second snap" })
	expect(app:has('#chat-composer >> ~"First snap"')).to_be(true)
	expect(app:has('#chat-composer >> ~"Second snap"')).to_be(true)
end)

test("New session: a capture goes to a new chat's composer, the session's draft kept", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:set_state("appshots_destination", "new-session")
	open(app, "s2")
	capture(app, { title = "To a new chat" })
	expect(app:has('#chat-composer >> ~"To a new chat"')).to_be(true)
	open(app, "s2")
	expect(app:has('#chat-composer >> ~"To a new chat"')).to_be(false)
end)
