-- Settings → General, with gpui's Chat page as its Chat section, on
-- ui/shell_fixture.luau with ui/settings_stub.luau; and the ids of pages
-- folded into another (`settings/chat`, `settings/browser_history`) opening
-- the page that has them now, at their section.

local reachable = require("./settings_panel").reachable

local PAGE = "#settings-page"
local NAV = "#settings-nav"

local function launch(): any
	local app = reachable(mount("shell_fixture.luau"))
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
local function box(app: any, target: string): { y: number, h: number }
	local line = app:tree(target):match("[^\n]+") :: string
	local y, h = line:match(" %-?[%d%.]+,(%-?[%d%.]+) [%d%.]+x([%d%.]+)")
	return { y = tonumber(y) :: number, h = tonumber(h) :: number }
end

test("General has Chat's rows in a Chat section, thread naming once, and the switcher; there is no Chat page", function()
	local app = open("General")
	expect(app:has(`{NAV} >> role:tab "Chat"`)).to_be(false)
	for _, w in { "Chat", "Send messages with", "Follow-up behavior", "Stop agent with Escape", "Compact mode", "Thread naming", "Session switcher", "Ctrl+Tab cycles tabs" } do
		expect(`{w}: {shows(app, w)}`).to_be(`{w}: true`)
	end
	local _, naming = app:tree(PAGE):gsub('text "Thread naming"', "")
	expect(naming).to_be(1)
	expect(table.concat(app:state("settings_stub_calls") or {}, ",")).to_contain("settings.engine:GetTitleSettings")
	app:click(`{NAV} >> role:tab "Shortcuts"`)
	app:frames(30)
	expect(shows(app, "Session switcher")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("the Chat section's choices land at once and are there at the next launch", function()
	local app = open("General")
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
	local next = open("General")
	expect(next:state("settings_follow_up")).to_be("steer")
	expect(checked(next, "Compact mode")).to_be(true)
	expect(checked(next, "Stop agent with Escape")).to_be(true)
	expect(next:tree(`{PAGE} >> role:button "Follow-up behavior"`)).to_contain("Steer")
end)

test("the search finds a Chat row, and thread naming once, on General's Chat section", function()
	local app = launch()
	app:type(`{NAV} >> #settings-search`, "thread naming")
	app:frames(10)
	local _, hits = app:tree("#settings-search-results"):gsub('option "Thread naming"', "")
	expect(hits).to_be(1)
	expect(app:tree("#settings-search-results")).to_contain("General · Chat")
	app:fill(`{NAV} >> "Search settings"`, "follow-up")
	app:frames(10)
	app:key("Enter")
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/settings/general")
	expect(app:state("settings_search_lit")).to_be("follow_up")
	app:fill(`{NAV} >> "Search settings"`, "cycles tabs")
	app:frames(10)
	app:key("Enter")
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/settings/general")
	expect(app:state("settings_search_lit")).to_be("key_switcher_cycles")
end)

test("settings/chat and settings/browser_history open General and Browser at their sections", function()
	for _, case in { { "chat", "/settings/general", "#setting-chat_section" }, { "browser_history", "/settings/browser", "#setting-history_section" } } do
		local app = reachable(mount("shell_fixture.luau"))
		app:resize(1200, 900)
		app:frames(30)
		app:command("settings_stub.sign_in")
		app:set_state("app.link", { seq = 1, settings = case[1] })
		app:frames(60)
		expect(app:state("shell_route_path")).to_be(case[2])
		expect(`{case[1]} scrolled: {(app:state("shell_settings_scroll") :: number) > 0}`).to_be(`{case[1]} scrolled: true`)
		local view, section = box(app, "#settings-scroll"), box(app, case[3])
		expect(`{case[1]} in view: {section.y >= view.y and section.y < view.y + view.h / 2}`).to_be(`{case[1]} in view: true`)
	end
end)
