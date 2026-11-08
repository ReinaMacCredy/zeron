-- Settings → Tools (screens/settings/tools) on ui/shell_fixture.luau with ui/settings_stub.luau's engine and groups.

local PAGE = "#settings-page"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> "Tools"')
	app:frames(30)
	return app
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end
local function count(app: any, what: string): number
	return select(2, string.gsub(calls(app), what, ""))
end
local function checked(app: any, scope: string, label: string): boolean
	for line in app:tree(scope):gmatch("[^\n]+") do
		if line:find(`switch "{label}"`, 1, true) then
			return line:find("checked", 1, true) ~= nil and line:find("unchecked", 1, true) == nil
		end
	end
	error(`no switch {label}`)
end
local function has(app: any, scope: string, words: string): boolean
	return app:tree(scope):find(words, 1, true) ~= nil
end
local function again(app: any)
	app:click('#settings-nav >> "General"')
	app:frames(20)
	app:click('#settings-nav >> "Tools"')
	app:frames(30)
end

test("Tools: its groups with their counts, the status, and the Computer group's own switch", function()
	local app = open()
	expect(app:error()).to_be(nil)
	expect(calls(app)).to_contain("settings.engine:GetToolSettings")
	expect(app:text("#tools-headline")).to_be("5 of 6 tools on")
	expect(has(app, "#tools-status", "PARTIAL")).to_be(true)
	expect(has(app, "#tools-group-threads", "3 TOOLS")).to_be(true)
	expect(has(app, "#tools-group-computer", "Same switch as Settings > Computer use")).to_be(true)
	expect(checked(app, "#tools-group-threads", "Threads")).to_be(true)
	expect(checked(app, "#tools-group-computer", "Computer")).to_be(false)
	-- (Its switch is Computer use's: turning it here changes nothing.)
	app:click(`#tools-group-computer >> role:switch "Computer"`)
	app:frames(20)
	expect(count(app, "SetToolSettings")).to_be(0)
	app:click("#tools-link-computer")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/settings/computer")
end)

test("Tools: a group or a single tool turned off goes to the engine at once, its dependents with it, and stays off", function()
	local app = open()
	app:click("#tools-open-threads")
	app:frames(20)
	expect(checked(app, "#tools-group-threads", "read_thread")).to_be(true)
	app:click(`#tools-group-threads >> role:switch "read_thread"`)
	app:frames(20)
	expect(count(app, "SetToolSettings")).to_be(1)
	expect(app:text("#tools-headline")).to_be("3 of 6 tools on")
	expect(has(app, "#tools-group-threads", "1/3 TOOLS")).to_be(true)
	expect(has(app, "#tools-group-threads", "Off while read_thread is off")).to_be(true)
	app:click(`#tools-group-workspace >> role:switch "Workspace"`)
	app:frames(20)
	expect(count(app, "SetToolSettings")).to_be(2)
	expect(checked(app, "#tools-group-workspace", "Workspace")).to_be(false)
	again(app)
	expect(count(app, "GetToolSettings")).to_be(2)
	expect(checked(app, "#tools-group-workspace", "Workspace")).to_be(false)
	expect(app:text("#tools-headline")).to_be("1 of 6 tools on")
	app:click("#tools-open-workspace")
	app:frames(20)
	expect(has(app, "#tools-group-workspace", "The whole group is off.")).to_be(true)
end)

test("Tools: Turn all off and Turn all on, the Computer group left to its page", function()
	local app = open()
	app:click("#tools-all-off")
	app:frames(20)
	expect(app:text("#tools-headline")).to_be("No Zeron tools")
	expect(app:tree("#tools-all-off")).to_contain("disabled")
	app:click("#tools-all-on")
	app:frames(20)
	expect(app:text("#tools-headline")).to_be("5 of 6 tools on")
	expect(count(app, "SetToolSettings")).to_be(2)
	expect(app:error()).to_be(nil)
end)
