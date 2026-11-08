-- On ui/chat_fixture.luau.

local function open(): any
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	app:frames(30)
	app:command("threads_listed")
	app:frames(10)
	return app
end

test("@ threads: the open chat can't be dropped into its own draft", function()
	local app = open()
	app:command("mention_s2")
	app:frames(10)
	expect(app:tree("#chat-composer")).to_contain("This is the open thread")
end)
