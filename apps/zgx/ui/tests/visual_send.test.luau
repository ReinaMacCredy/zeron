-- What a visual sends (a quiz's ask, a send button, a field's Enter) is the
-- person's message to the chat through `chat.send`, labeled for the agent by
-- its last line and for the person by "From <title>" over the bubble. It is
-- never a pending question's answer: queued behind a turn that waits on one,
-- refused with the composer's notice when the question has no turn to wait in.

local LABELED = 'send:s2:Explain the quiz questions I got wrong: 1, 3.\n\n(Sent from "Quick check", not typed.)'

local function open(app: any)
	app:resize(900, 700)
	app:command("go_s2")
	app:frames(30)
end

test("a visual's message goes to the chat with its label line", function()
	local app = mount("chat_fixture.luau")
	open(app)
	app:command("visual_sends")
	app:frames(10)
	expect(app:state("calls")).to_contain(LABELED)
	expect(app:state("calls")).to_contain("visual_send:true")
	expect(app:error()).to_be(nil)
end)

test("with a question waiting, a visual's message is queued behind it, not its answer", function()
	local app = mount("chat_fixture.luau")
	open(app)
	app:command("start_reply")
	app:command("ask_question")
	app:frames(10)
	app:command("visual_sends")
	app:frames(10)
	local calls = app:state("calls")
	expect(calls).to_contain(LABELED)
	expect(calls).to_contain("visual_send:true")
	expect(string.find(calls, "answer:", 1, true)).to_be(nil)
	expect(app:has('~"Which branch should the fix go on?"')).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("a question left open with no turn to wait in refuses a visual's message and says why", function()
	local app = mount("chat_fixture.luau")
	open(app)
	app:command("ask_question")
	app:frames(10)
	app:command("visual_sends")
	app:frames(10)
	local calls = app:state("calls")
	expect(calls).to_contain("visual_send:false")
	expect(string.find(calls, "Sent from", 1, true)).to_be(nil)
	expect(string.find(calls, "answer:", 1, true)).to_be(nil)
	expect(app:has("~\"Answer the agent's question first\"")).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("a message a visual sent shows From <title> over its bubble", function()
	local app = mount("chat_fixture.luau")
	open(app)
	app:command("visual_sent_lands")
	app:frames(20)
	expect(app:has('#transcript >> .user-from >> "From Quick check"')).to_be(true)
	expect(app:has('#transcript >> ~"Explain the quiz questions I got wrong"')).to_be(true)
	expect(app:error()).to_be(nil)
end)
