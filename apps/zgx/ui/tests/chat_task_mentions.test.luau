-- `@` task rows and the `task ` scope in a chat composer, on ui/chat_fixture.luau.

local TASKS = {
	state = "ready",
	tasks = {
		{ id = "t1", title = "Renew the signing certificate", notes = "", order = "a1", created = 1, images = {}, depends = {} },
		{ id = "t2", title = "Draft [the] copy", notes = "", order = "a2", created = 2, images = {}, depends = {} },
		{ id = "t3", title = "Renew the domain", notes = "", order = "a3", created = 3, images = {}, depends = {}, completed = 10 },
		{ id = "t4", title = "Ask [Fix the build](zeron-chat:a1) again", notes = "", order = "a4", created = 4, images = {}, depends = {} },
	},
}

local function open(): any
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	app:frames(30)
	-- (The tasks' module loads with the first `@`.)
	app:type('role:textbox "Message"', "@")
	app:frames(10)
	app:set_state("tasks.list", TASKS)
	app:fill('role:textbox "Message"', "")
	app:frames(5)
	return app
end
local BOX = 'role:textbox "Message"'

test("@ tasks: open tasks listed under Tasks, done ones left out with no query", function()
	local app = open()
	app:type(BOX, "@")
	app:frames(20)
	expect(app:has("#mention-task-t1")).to_be(true)
	expect(app:has("#mention-task-t2")).to_be(true)
	expect(app:has("#mention-task-t3")).to_be(false)
	expect(app:tree("#mention-panel")).to_contain("T\\u{200a}A\\u{200a}S\\u{200a}K\\u{200a}S")
	expect(app:tree("#mention-task-t1")).to_contain("No project")
end)

test("@task scope: only tasks, past the space, done ones too, and Enter puts the task's link in the draft", function()
	local app = open()
	app:type(BOX, "see @task renew")
	app:frames(20)
	expect(app:has("#mention-task-t1")).to_be(true)
	expect(app:has("#mention-task-t3")).to_be(true)
	expect(app:has("#mention-task-t2")).to_be(false)
	expect(app:has('#mention-panel >> "Threads"')).to_be(false)
	app:key("Enter")
	app:frames(10)
	expect(app:tree(BOX)).to_contain("see [Renew the signing certificate](zeron-task:t1) ")
end)

test("@ tasks: a title's brackets are escaped in its link", function()
	local app = open()
	app:type(BOX, "@task draft")
	app:frames(20)
	app:key("Enter")
	app:frames(10)
	expect(app:tree(BOX)).to_contain("[Draft \\\\[the\\\\] copy](zeron-task:t2)")
end)

test("@ tasks: a title holding a mention lists and links by its words alone", function()
	local app = open()
	app:type(BOX, "@task fix the")
	app:frames(20)
	expect(app:tree("#mention-task-t4")).to_contain("Ask Fix the build again")
	expect(app:tree("#mention-task-t4")).never.to_contain("zeron-chat")
	app:key("Enter")
	app:frames(10)
	expect(app:tree(BOX)).to_contain("[Ask Fix the build again](zeron-task:t4)")
end)
