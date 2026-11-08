-- Settings → Usage: the host's figures over a window, in dollars or tokens,
-- broken down by model or period. On the shell in ui/shell_fixture.luau, the
-- host's calls answered by ui/usage_stub.luau.

local PAGE = "#settings-page"

local function shows(app: any, text: string): boolean
	return app:tree(PAGE):find(text, 1, true) ~= nil
end

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1200)
	app:frames(30)
	app:command("settings_stub.sign_in")
	app:command("shell.settings")
	app:frames(20)
	return app
end

local function to_usage(app: any)
	app:click(`#settings-nav >> role:tab "Usage"`)
	app:frames(30)
end

test("Usage reads the window, then follows the metric and the breakdown", function()
	local app = open()
	to_usage(app)
	expect(app:state("shell_route_path")).to_be("/settings/usage")
	expect((app:state("usage_stub_calls") :: any)[1]).to_be(30)
	expect(shows(app, "$12.34")).to_be(true)
	expect(shows(app, "3 sessions · API estimate")).to_be(true)
	expect(shows(app, "Excludes 2 responses")).to_be(true)
	expect(shows(app, "Claude Code · 97%")).to_be(true)
	-- gpui's summary: each agent's sessions, figure and share; the chart named, its axis on round steps.
	expect(app:tree("#usage-providers")).to_contain("97% of cost · 1.2M tokens")
	expect(app:tree("#usage-providers")).to_contain("$0.34")
	expect(app:tree("#usage-chart-title")).to_contain("Daily cost")
	expect(app:tree("#usage-axis")).to_contain('"$15"')
	expect(app:tree("#usage-axis")).to_contain('"$5"')

	app:click(`{PAGE} >> "Tokens"`)
	app:frames(10)
	expect(shows(app, "1.5M")).to_be(true)
	expect(shows(app, "Claude Code · 80%")).to_be(true)
	expect(shows(app, "API estimate")).to_be(false)
	expect(app:tree("#usage-providers")).to_contain("80% of tokens · $12.00")
	expect(app:tree("#usage-chart-title")).to_contain("Daily processed tokens")
	expect(app:tree("#usage-axis")).to_contain('"500K"')

	app:click(`{PAGE} >> "Day"`)
	app:frames(10)
	expect(shows(app, "Claude Code · 80%")).to_be(false)
end)

test("Usage says so when a window has no activity, or the read fails", function()
	local app = open()
	app:command("usage_stub.fail_next")
	to_usage(app)
	expect(shows(app, "Couldn't read usage")).to_be(true)
	app:click(`{PAGE} >> "Refresh usage"`)
	app:frames(10)
	expect(shows(app, "$12.34")).to_be(true)

	app:set_state("settings_usage_days", 90)
	app:frames(20)
	expect(shows(app, "No activity in this window.")).to_be(true)
end)
