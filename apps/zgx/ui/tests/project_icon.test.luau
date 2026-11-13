-- A project's icon is `.project-icon` wherever it's drawn (ds/tile's
-- `project`), a stable element a plugin may replace to draw each project's
-- own: the sidebar's project groups, the palette's projects, the panel's
-- project. (What a plugin draws there, and what it reads of the project,
-- is src/plugins/loading_tests.rs's `a_plugin_draws_each_project_s_icon`.)
-- (The panel's project row isn't tested here: our Sidecar holds a session's
-- place while it shows it, with no project tile.)
-- On the shell in ui/shell_fixture.luau: two projects, Atlas and Borealis.

local function settle(app: any)
	app:frames(30)
end

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(40)
	return app
end

test("the sidebar's project groups wear .project-icon, one each", function()
	local app = open()
	expect(app:count("#shell-sidebar >> .project-icon")).to_be(2)
end)

test("the palette's projects wear .project-icon", function()
	local app = open()
	app:command("shell.palette")
	settle(app)
	app:type('role:textbox ~"Search"', "Atl")
	settle(app)
	expect(app:count("#palette-list >> .project-icon") > 0).to_be(true)
end)
