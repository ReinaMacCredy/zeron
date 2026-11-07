-- Settings → Agent profiles (screens/settings/agent_profiles), on the shell in
-- ui/shell_fixture.luau with the profiles host of ui/profiles_stub.luau: the
-- list in its order with its badges, lines and notes, reorder, the editor
-- (its fields, its tools, Remove asked once more, a name needed), New
-- profile, the host's System prompt, and the picker's More… landing here.

local PAGE = "#settings-page"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> "Agent profiles"')
	app:frames(30)
	return app
end
local function calls(app: any): string
	return tostring(app:state("calls") or "")
end
local function shows(app: any, target: string, scope: string?): boolean
	for line in app:tree(scope):gmatch("[^\n]+") do
		if line:find(target, 1, true) and not line:find(" hidden", 1, true) then
			return true
		end
	end
	return false
end

test("the profiles list in their order with their lines, by agent and notes; the host's prompt under them", function()
	local app = open()
	expect(app:has(`{PAGE} >> "Planner"`)).to_be(true)
	expect(app:has(`{PAGE} >> "Scout"`)).to_be(true)
	expect(shows(app, '"by agent"')).to_be(true)
	expect(app:has(`{PAGE} >> ~"Claude · Opus · Auto-accept edits · High"`)).to_be(true)
	expect(app:has(`{PAGE} >> ~"Plans before it edits"`)).to_be(true)
	expect(app:text('role:textbox "System prompt"')).to_be("Answer briefly.")
	app:fill('role:textbox "System prompt"', "Answer in English.")
	app:frames(10)
	app:click("#profile-system-prompt-save")
	app:frames(10)
	expect(calls(app)).to_contain("system_prompt:Answer in English.")
end)

test("the arrows move a profile, and the first can't go up", function()
	local app = open()
	app:click("#profile-down-planner")
	app:frames(20)
	expect(calls(app)).to_contain("reorder_profiles:reviewer,planner,scout,quick,relay")
	app:click("#profile-up-reviewer")
	app:frames(20)
	expect(calls(app)).never.to_contain("reorder_profiles:planner,reviewer,scout,quick,relay,")
end)

test("Edit opens the editor on the profile; a name is needed; Save writes it and closes", function()
	local app = open()
	app:click("#profile-edit-planner")
	app:frames(30)
	expect(app:has('role:dialog "Edit agent profile"')).to_be(true)
	expect(app:text('role:textbox "Name"')).to_be("Planner")
	expect(app:text('role:textbox "When to use"')).to_be("Plans before it edits")
	app:fill('role:textbox "Name"', "")
	app:click("#profile-save")
	app:frames(10)
	expect(app:text("#profile-editor-error")).to_be("Give the profile a name.")
	app:fill('role:textbox "Name"', "Planner prime")
	app:fill('role:textbox "Instructions"', "Plan, then wait.")
	app:click("#profile-save")
	app:frames(30)
	expect(calls(app)).to_contain("save_profile:planner:Planner prime")
	expect(app:has('role:dialog "Edit agent profile"')).to_be(false)
	expect(app:has(`{PAGE} >> "Planner prime"`)).to_be(true)
end)

test("Remove asks once more, then the profile goes", function()
	local app = open()
	app:click("#profile-edit-scout")
	app:frames(30)
	app:click("#profile-remove")
	app:frames(10)
	expect(app:has('~"Confirm remove"')).to_be(true)
	expect(calls(app)).never.to_contain("remove_profile")
	app:click("#profile-remove")
	app:frames(30)
	expect(calls(app)).to_contain("remove_profile:scout")
	expect(shows(app, '"Scout"')).to_be(false)
end)

test("the tools open over the rest; a group turned off holds its rules; Escape folds them back first", function()
	local app = open()
	app:click("#profile-edit-reviewer")
	app:frames(30)
	app:click("#profile-tools-folded")
	app:frames(30)
	expect(calls(app)).to_contain("profile_tools:claude-code")
	expect(shows(app, '"Zeron tools"', 'role:dialog')).to_be(true)
	expect(shows(app, '"Claude built-in"', 'role:dialog')).to_be(true)
	expect(shows(app, '"No MCP servers in your Claude config."', 'role:dialog')).to_be(true)
	expect(shows(app, "#profile-folded", 'role:dialog')).to_be(true)
	-- (The keys are in the tools as they open: Escape there folds them back.)
	app:key("Escape")
	app:frames(20)
	expect(shows(app, "#profile-folded", 'role:dialog')).to_be(false)
	app:click("#profile-tools-folded")
	app:frames(30)
	app:click('role:switch "Threads"')
	app:frames(10)
	app:key("Escape")
	app:frames(20)
	expect(shows(app, "#profile-folded", 'role:dialog')).to_be(false)
	expect(app:has('role:dialog "Edit agent profile"')).to_be(true)
	expect(shows(app, '"Tools"', 'role:dialog')).to_be(true)
	expect(app:has('#profile-tools-folded >> ~"3 off"')).to_be(true)
end)

test("New profile opens a blank editor; More… from the picker lands here with its draft", function()
	local app = open()
	app:click("#profile-new")
	app:frames(30)
	expect(app:has('role:dialog "New agent profile"')).to_be(true)
	expect(app:text('role:textbox "Name"')).to_be("")
	app:key("Escape")
	app:frames(30)
	app:set_state("profiles_pending_edit", { draft = { name = "Handed over", harness = "claude-code", model = "opus", options = {}, notes = "From the picker", instructions = "", tools_off = {} } })
	app:frames(30)
	expect(app:has('role:dialog "New agent profile"')).to_be(true)
	expect(app:text('role:textbox "Name"')).to_be("Handed over")
	expect(app:text('role:textbox "When to use"')).to_be("From the picker")
end)
