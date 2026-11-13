-- A launch before the engine has answered: nobody is called signed out, in
-- the rail or on the Account page, nor told they have no sessions,
-- however long it takes; the engine saying so is what shows it. On the shell in ui/shell_fixture.luau, Settings'
-- host calls answered by ui/settings_stub.luau.

local RAIL = "#app-rail"
local SIGNED_OUT_PAGE = "Sign in to show up as yourself across Zeron."

local function open(command: string): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1000)
	app:frames(30)
	app:command(command)
	app:frames(20)
	return app
end

local function shows(app: any, scope: string, text: string): boolean
	return app:tree(scope):find(text, 1, true) ~= nil
end

test("restoring: the rail shows no account, and no workspace of its own", function()
	local app = open("settings_stub.restoring")
	expect(app:has("#rail-account")).to_be(false)
	expect(shows(app, RAIL, "Local")).to_be(false)
end)

test("signed out, said by the engine: the rail's account says Local", function()
	local app = open("settings_stub.signed_out")
	expect(shows(app, RAIL, '#rail-account button "Local"')).to_be(true)
end)

test("restoring: the Account page isn't the signed-out one", function()
	local app = open("settings_stub.restoring")
	app:command("shell.settings")
	app:frames(20)
	app:click(`#settings-nav >> role:tab "Account"`)
	app:frames(30)
	expect(shows(app, "#settings-page", SIGNED_OUT_PAGE)).to_be(false)
end)

test("signed out, said by the engine: the Account page offers signing in", function()
	local app = open("settings_stub.signed_out")
	app:command("shell.settings")
	app:frames(20)
	app:click(`#settings-nav >> role:tab "Account"`)
	app:frames(30)
	expect(shows(app, "#settings-page", SIGNED_OUT_PAGE)).to_be(true)
end)

test("loading: the sidebar doesn't say there are no sessions", function()
	local app = open("fixture.sessions_loading")
	expect(app:has("#sidebar-empty")).to_be(false)
end)

test("answered with none: the sidebar says so", function()
	local app = open("fixture.no_sessions")
	expect(app:has("#sidebar-empty")).to_be(true)
end)
