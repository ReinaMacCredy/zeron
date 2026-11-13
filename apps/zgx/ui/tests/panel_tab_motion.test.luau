-- The panel's tabs open and close as Chrome's: a new one grows to its width, a closed one
-- stays, shrinking away, while the tabs after it slide over.

local function width(app: any, id: string): number?
	local w = app:tree("#panel-strip"):match(`#panel%-tab%-{id} tab "[^"]*" %-?%d+,%-?%d+ (%d+)x%d+`)
	return if w then tonumber(w) else nil
end

local function two_tabs(app: any)
	app:resize(1200, 800)
	app:frames(30)
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(20)
	app:click('#titlebar >> role:button "Panel"')
	expect(app:wait("#panel-open-files", { timeout_ms = 15000 })).to_be(true)
	app:frames(60)
	app:click("#panel-open-files")
	app:frames(60)
	app:click('#panel-strip >> role:button "New Tab"')
	expect(app:wait("#panel-open-terminal", { timeout_ms = 15000 })).to_be(true)
	app:click("#panel-open-terminal")
end

test("a panel tab opening grows to its width, clickable from its first frame", function()
	local app = mount("shell_fixture.luau")
	two_tabs(app)
	app:frames(2)
	local early = width(app, "tab2") :: number
	app:click('#panel-strip >> role:tab "Terminal 1"')
	app:frames(30)
	local full = width(app, "tab2") :: number
	expect(early < full).to_be(true)
end)

test("a panel tab closing stays, shrinking, and is gone after", function()
	local app = mount("shell_fixture.luau")
	two_tabs(app)
	app:frames(30)
	local full = width(app, "tab1") :: number
	app:click('#panel-strip >> role:tab "Files"')
	app:key("cmd+w")
	app:frames(3)
	local partway = width(app, "tab1")
	expect(partway ~= nil and partway < full).to_be(true)
	app:frames(20)
	expect(width(app, "tab1")).to_be(nil)
	expect(app:count("#panel-strip >> role:tab")).to_be(1)
end)
