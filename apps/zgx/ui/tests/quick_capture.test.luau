-- Quick capture on ui/tasks_fixture.luau.

local function open(): any
	local app = mount("tasks_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	return app
end
local function calls(app: any): string
	return app:state("tasks_calls") :: string
end
-- (A modal that closed stays in the tree, hidden.)
local function shown(app: any, target: string): boolean
	return app:has(target) and not string.find((app:tree(target):match("[^\n]+")) or "", " hidden")
end
local function focused(app: any, target: string): boolean
	return (string.find((app:tree(target):match("[^\n]+")) or "", " focused")) ~= nil
end

local function on_tasks(app: any)
	app:click("#rail-tasks")
	app:frames(30)
	app:click("#tasks-subtitle")
	app:frames(5)
end

-- (Its command, which its key ⌘⌥N runs: no shortcut reaches a command in
-- these tests on Linux, the baseline's shortcut reds; U26-M3 presses it.)
test("its command opens it; Enter saves to To do and it closes", function()
	local app = open()
	app:command("shell.quick_capture")
	app:frames(20)
	expect(shown(app, "#quick-capture")).to_be(true)
	expect(focused(app, "#quick-capture-input")).to_be(true)
	app:type("#quick-capture-input", "Call the bank")
	app:key("Enter")
	app:frames(20)
	expect(calls(app)).to_contain("create:Call the bank")
	expect(shown(app, "#quick-capture")).to_be(false)
	expect(app:has('"Added to To do: Call the bank"')).to_be(true)
end)

test("saved from the Tasks page, focus comes back to the page", function()
	local app = open()
	on_tasks(app)
	app:command("shell.quick_capture")
	app:frames(20)
	expect(focused(app, "#quick-capture-input")).to_be(true)
	app:type("#quick-capture-input", "Call the bank")
	app:key("Enter")
	app:frames(20)
	expect(focused(app, "#tasks-page")).to_be(true)
end)

test("Escape closes it with nothing saved, and focus goes back where it was", function()
	local app = open()
	on_tasks(app)
	app:command("shell.quick_capture")
	app:frames(20)
	app:type("#quick-capture-input", "Never mind")
	app:key("Escape")
	app:frames(20)
	expect(shown(app, "#quick-capture")).to_be(false)
	expect(calls(app)).never.to_contain("Never mind")
	expect(focused(app, "#tasks-page")).to_be(true)
end)

test("Shift+Enter starts the notes: the first line is the title, the rest the notes", function()
	local app = open()
	app:command("shell.quick_capture")
	app:frames(20)
	app:type("#quick-capture-input", "Plan the offsite")
	app:key("shift+Enter")
	app:type("#quick-capture-input", "book the room")
	app:key("Enter")
	app:frames(20)
	expect(calls(app)).to_contain("create:Plan the offsite|book the room")
end)

test("a capture is New on Tasks and counted on the rail until Tasks has shown it; Undo takes it back", function()
	local app = open()
	expect(app:has("#rail-tasks-badge")).to_be(false)
	app:command("shell.quick_capture")
	app:frames(20)
	app:type("#quick-capture-input", "Water the plants")
	app:key("Enter")
	app:frames(20)
	expect(app:tree("#rail-tasks-badge")).to_contain("1")
	app:click("#rail-tasks")
	app:frames(30)
	expect(app:has("#rail-tasks-badge")).to_be(false)
	expect(app:tree("#task-row-n101")).to_contain("New")
	app:click("#rail-inbox")
	app:frames(20)
	app:click("#rail-tasks")
	app:frames(30)
	expect(app:tree("#task-row-n101")).never.to_contain("New")
end)

test("the toast's Undo takes the capture back", function()
	local app = open()
	app:command("shell.quick_capture")
	app:frames(20)
	app:type("#quick-capture-input", "Oops")
	app:key("Enter")
	app:frames(20)
	app:click('role:button "Undo"')
	app:frames(20)
	expect(calls(app)).to_contain("delete:n101")
end)
