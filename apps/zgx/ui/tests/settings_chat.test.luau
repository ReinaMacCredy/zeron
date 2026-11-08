-- Settings → Chat and General as gpui splits them, on ui/shell_fixture.luau with ui/settings_stub.luau.

local PAGE = "#settings-page"
local NAV = "#settings-nav"

local function launch(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("shell.settings")
	app:frames(20)
	return app
end
local function open(page: string): any
	local app = launch()
	app:click(`{NAV} >> role:tab "{page}"`)
	app:frames(30)
	return app
end
local function shows(app: any, text: string): boolean
	return app:tree(PAGE):find(text, 1, true) ~= nil
end
local function checked(app: any, label: string): boolean
	for line in app:tree(PAGE):gmatch("[^\n]+") do
		if line:find(`switch "{label}"`, 1, true) then
			return line:find("checked", 1, true) ~= nil and line:find("unchecked", 1, true) == nil
		end
	end
	error(`no switch {label}`)
end

test("Chat has the composer's rows, the transcript's and thread naming; General has the switcher, not them", function()
	local app = open("Chat")
	for _, w in { "Composer", "Send messages with", "Follow-up behavior", "Stop agent with Escape", "Transcript", "Compact mode", "Thread naming" } do
		expect(`{w}: {shows(app, w)}`).to_be(`{w}: true`)
	end
	expect(table.concat(app:state("settings_stub_calls") or {}, ",")).to_contain("settings.engine:GetTitleSettings")
	app:click(`{NAV} >> role:tab "General"`)
	app:frames(30)
	expect(shows(app, "Send messages with")).to_be(false)
	expect(shows(app, "Thread naming")).to_be(false)
	expect(shows(app, "Session switcher")).to_be(true)
	expect(shows(app, "Ctrl+Tab cycles tabs")).to_be(true)
	app:click(`{NAV} >> role:tab "Shortcuts"`)
	app:frames(30)
	expect(shows(app, "Session switcher")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("Chat's choices land at once and are there at the next launch", function()
	local app = open("Chat")
	app:click(`{PAGE} >> role:button "Follow-up behavior"`)
	app:frames(10)
	app:click('role:menuitem "Steer"')
	app:frames(10)
	expect(app:state("settings_follow_up")).to_be("steer")
	app:click(`{PAGE} >> role:switch "Compact mode"`)
	app:frames(10)
	expect(app:state("settings_compact_transcript")).to_be(true)
	app:click(`{PAGE} >> role:switch "Stop agent with Escape"`)
	app:frames(10)
	expect(app:state("settings_escape_stops")).to_be(true)
	local next = open("Chat")
	expect(next:state("settings_follow_up")).to_be("steer")
	expect(checked(next, "Compact mode")).to_be(true)
	expect(checked(next, "Stop agent with Escape")).to_be(true)
	expect(next:tree(`{PAGE} >> role:button "Follow-up behavior"`)).to_contain("Steer")
end)

test("the search finds a Chat row and opens Chat at it; General's switcher rows are found on General", function()
	local app = launch()
	app:type(`{NAV} >> #settings-search`, "follow-up")
	app:frames(10)
	app:key("Enter")
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/settings/chat")
	expect(app:state("settings_search_lit")).to_be("follow_up")
	app:fill(`{NAV} >> "Search settings"`, "cycles tabs")
	app:frames(10)
	app:key("Enter")
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/settings/general")
	expect(app:state("settings_search_lit")).to_be("key_switcher_cycles")
end)
