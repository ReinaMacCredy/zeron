-- The Home panel under a new chat's composer (gpui's Needs you / Running / Recent cards and widgets).

local function home(statuses: { [string]: string }): any
	local app = mount("chat_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("go_home")
	app:frames(30)
	local now = table.clone(app:state("sessions") :: any)
	local chats = {}
	for _, c in now.chats do
		local copy = table.clone(c)
		copy.status = statuses[c.id]
		copy.updated = os.time() - 120
		table.insert(chats, copy)
	end
	now.chats = chats
	app:set_state("sessions", now)
	app:frames(30)
	return app
end

test("the Home panel stands under a new chat's composer, and the Home widgets switch takes it away", function()
	local app = home({ s1 = "input", s2 = "working" })
	expect(app:has("#home-panel")).to_be(true)
	expect(app:tree("#card-needs")).to_contain("Long chat")
	expect(app:tree("#card-running")).to_contain("Short chat")
	expect(app:count("#home-customize")).to_be(1)
	app:click("#home-collapse")
	app:frames(40)
	expect(app:has("#home-grid")).to_be(true)
	expect(app:count("#home-customize")).to_be(1)
	app:set_state("builtin_home_widgets", false)
	app:frames(20)
	expect(app:has("#home-panel")).to_be(false)
	app:set_state("builtin_home_widgets", true)
	app:frames(20)
	expect(app:has("#home-panel")).to_be(true)
end)

test("a Needs you row opens its chat", function()
	local app = home({ s1 = "failed" })
	expect(app:tree("#card-needs")).to_contain("Failed")
	app:click("#card-need-s1")
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/session/s1")
end)
