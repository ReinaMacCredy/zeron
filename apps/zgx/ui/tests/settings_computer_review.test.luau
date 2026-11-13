-- Settings → Computer use (screens/settings/computer) and Settings → Code
-- review (screens/settings/code_review), on the shell in ui/shell_fixture.luau
-- with the engine's answers of ui/settings_stub.luau: gpui's rows with the
-- engine's values, each change written to the engine at once and read back
-- when the page is opened again, a refused one put back with its words, and
-- Computer use left as the engine has it off a Mac, as gpui's is.

local reachable = require("./settings_panel").reachable

local PAGE = "#settings-page"

local function open(page: string, mac: boolean?): any
	local app = reachable(mount("shell_fixture.luau"))
	app:resize(1200, 900)
	if mac then
		app:set_state("platform", "macos")
	end
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click(`#settings-nav >> "{page}"`)
	app:frames(30)
	return app
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end
local function count(app: any, what: string): number
	return select(2, string.gsub(calls(app), what, ""))
end
local function checked(app: any, label: string): boolean
	for line in app:tree(PAGE):gmatch("[^\n]+") do
		if line:find(`switch "{label}`, 1, true) then
			return line:find("checked", 1, true) ~= nil and line:find("unchecked", 1, true) == nil
		end
	end
	error(`no switch {label}`)
end
local function again(app: any, page: string)
	app:click('#settings-nav >> "General"')
	app:frames(20)
	app:click(`#settings-nav >> "{page}"`)
	app:frames(30)
end
local function has(app: any, words: string): boolean
	return app:tree(PAGE):find(words, 1, true) ~= nil
end

local CONTROL = "Let the agent use the desktop in any chat"
local DEVELOPER = "Developer access for Zeron browser"

test("computer use settings: gpui's rows with the engine's values and its desktop host's status", function()
	local app = open("Computer use", true)
	for _, w in {
		"Let an agent see and drive this Mac's apps.",
		"Computer control",
		"Enable Computer by default in any chat.",
		"Cursor colors",
		"Stock",
		"Custom",
		"Preview",
		"Compact",
		"Large",
		"Placement",
		"Floating card",
		"Rail + thumbnail",
		"Click sounds",
		"Foreground and background clicks",
		"Developer access",
		"Always-allowed apps",
		"Ghostty",
		"com.mitchellh.ghostty",
		"com.example.tool",
		"Getting started",
		"Ask for a task",
		"Approve the task",
		"Follow and stop",
		"Advanced",
	} do
		expect(`{w}: {has(app, w)}`).to_be(`{w}: true`)
	end
	expect(app:text("#computer-status")).to_be("Computer control available · Host sh.zeron.app · Driver revision 39")
	expect(checked(app, CONTROL)).to_be(false)
	expect(checked(app, DEVELOPER)).to_be(false)
	expect(checked(app, "Show the computer preview automatically")).to_be(true)
	expect(calls(app)).to_contain("settings.engine:GetComputerSettings")
	expect(calls(app)).to_contain("settings.computer_status")
	expect(app:has("#computer-cursor-fill")).to_be(false)
	app:scroll(PAGE, 2000)
	app:frames(10)
	app:click("#computer-advanced-toggle")
	app:frames(20)
	expect(has(app, "Revision 39, matching this build.")).to_be(true)
	expect(has(app, "Registered host: sh.zeron.app")).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("computer use settings: on a Mac each change goes to the engine at once and is there when the page opens again", function()
	local app = open("Computer use", true)
	app:click(`{PAGE} >> role:switch "{CONTROL}"`)
	app:frames(20)
	expect(count(app, "SetComputerSettings")).to_be(1)
	expect(checked(app, CONTROL)).to_be(true)
	app:click(`{PAGE} >> role:switch "{DEVELOPER}"`)
	app:frames(20)
	app:click(`{PAGE} >> "Custom"`)
	app:frames(20)
	expect(app:has("#computer-cursor-fill")).to_be(true)
	app:type("#computer-cursor-fill", "#ABCDEF")
	app:key("enter")
	app:frames(20)
	app:click(`{PAGE} >> "Large"`)
	app:frames(20)
	app:click(`{PAGE} >> "Floating card"`)
	app:frames(20)
	app:click(`{PAGE} >> role:button "Remove Ghostty from always-allowed apps"`)
	app:frames(20)
	expect(has(app, "com.mitchellh.ghostty")).to_be(false)
	expect(count(app, "SetComputerSettings")).to_be(7)
	-- Back on the page, it reads what the engine now has.
	again(app, "Computer use")
	expect(count(app, "GetComputerSettings")).to_be(2)
	expect(checked(app, CONTROL)).to_be(true)
	expect(checked(app, DEVELOPER)).to_be(true)
	expect(app:has("#computer-cursor-fill")).to_be(true)
	expect(app:tree("#computer-cursor-fill")).to_contain("#abcdef")
	expect(has(app, "com.mitchellh.ghostty")).to_be(false)
	expect(has(app, "com.example.tool")).to_be(true)
	-- Reset puts gpui's defaults back.
	app:click("#computer-cursor-reset")
	app:frames(20)
	expect(app:has("#computer-cursor-fill")).to_be(false)
	again(app, "Computer use")
	expect(app:has("#computer-cursor-fill")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("computer use settings: a refused change goes back and says why; off a Mac nothing changes", function()
	local app = open("Computer use", true)
	app:command("settings_stub.refuse_next")
	app:click(`{PAGE} >> role:switch "{CONTROL}"`)
	app:frames(20)
	expect(checked(app, CONTROL)).to_be(false)
	expect(app:text("#computer-refused")).to_be("refused")
	local elsewhere = open("Computer use")
	expect(has(elsewhere, "Computer control is set on the Mac that runs this engine.")).to_be(true)
	elsewhere:click(`{PAGE} >> role:switch "{CONTROL}"`)
	elsewhere:frames(20)
	expect(checked(elsewhere, CONTROL)).to_be(false)
	expect(count(elsewhere, "SetComputerSettings")).to_be(0)
end)

test("computer use settings: click sounds and the guide's Got it are this app's own, kept", function()
	local app = open("Computer use")
	app:click(`{PAGE} >> "Off"`)
	app:frames(20)
	expect(app:state("click_sounds")).to_be("off")
	expect(count(app, "SetComputerSettings")).to_be(0)
	app:scroll(PAGE, 2000)
	app:frames(10)
	app:click("#computer-guide-done")
	app:frames(20)
	expect(app:state("guide_acknowledged")).to_be(true)
	expect(has(app, "Ask for a task")).to_be(false)
	again(app, "Computer use")
	expect(has(app, "Ask for a task")).to_be(false)
	app:scroll(PAGE, 2000)
	app:frames(10)
	app:click("#computer-guide-toggle")
	app:frames(20)
	expect(has(app, "Ask for a task")).to_be(true)
	expect(app:has("#computer-guide-done")).to_be(false)
end)

test("code review settings: the reviewer, its model and thinking go to the engine; the instructions save", function()
	local app = open("Code review")
	for _, w in {
		"Who reviews changes when you press Review in the composer.",
		"Reviewer",
		"Auto picks a profile named reviewer",
		"A pick in the Review button's menu applies to that chat only",
		"Review instructions",
	} do
		expect(`{w}: {has(app, w)}`).to_be(`{w}: true`)
	end
	expect(calls(app)).to_contain("settings.engine:GetReviewSettings")
	expect(calls(app)).to_contain("review.reviewers")
	expect(has(app, "Thinking")).to_be(false)
	app:click(`{PAGE} >> role:button "Reviewer"`)
	app:frames(20)
	app:click('role:menuitem "Claude Code"')
	app:frames(30)
	expect(count(app, "SetReviewSettings")).to_be(1)
	expect(calls(app)).to_contain("review.models")
	expect(has(app, "Shown when the reviewer is an agent.")).to_be(true)
	expect(has(app, "How hard the reviewer thinks.")).to_be(true)
	app:click(`{PAGE} >> "High"`)
	app:frames(20)
	expect(count(app, "SetReviewSettings")).to_be(2)
	app:click(`{PAGE} >> role:button "Model"`)
	app:frames(20)
	app:click('role:menuitem "Opus"')
	app:frames(20)
	expect(count(app, "SetReviewSettings")).to_be(3)
	app:type("#code-review-instructions", "Look for races.")
	app:frames(5)
	app:click("#code-review-instructions-save")
	app:frames(20)
	expect(count(app, "SetReviewSettings")).to_be(4)
	again(app, "Code review")
	expect(count(app, "GetReviewSettings")).to_be(2)
	expect(app:tree(`{PAGE} >> role:button "Reviewer"`)).to_contain("Claude Code")
	expect(app:tree(`{PAGE} >> role:button "Model"`)).to_contain("Opus")
	expect(app:tree("#code-review-instructions")).to_contain("Look for races.")
	expect(app:error()).to_be(nil)
end)

test("code review settings: a refused pick goes back and says why", function()
	local app = open("Code review")
	app:command("settings_stub.refuse_next")
	app:click(`{PAGE} >> role:button "Reviewer"`)
	app:frames(20)
	app:click('role:menuitem "Codex"')
	app:frames(30)
	expect(app:text("#code-review-refused")).to_be("refused")
	expect(app:tree(`{PAGE} >> role:button "Reviewer"`)).to_contain("Auto")
	expect(has(app, "Shown when the reviewer is an agent.")).to_be(false)
end)

