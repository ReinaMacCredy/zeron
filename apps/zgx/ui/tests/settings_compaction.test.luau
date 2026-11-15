-- Settings → Compaction (screens/settings/compaction), on the shell in
-- ui/shell_fixture.luau with the engine's answers of ui/settings_stub.luau:
-- the three reasons a chat compacts, each with who decides it and its own
-- control, the engine's values, each change written to the engine, the
-- agent's switch as the `compact` tool's, a reason's own setting out of
-- reach while it is off, and a refused change put back with its words.

local PAGE = "#settings-page"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> "Compaction"')
	app:frames(30)
	return app
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end
local function checked(app: any, label: string): boolean
	for line in app:tree(PAGE):gmatch("[^\n]+") do
		if line:find(`switch "{label}"`, 1, true) then
			return line:find("checked", 1, true) ~= nil and line:find("unchecked", 1, true) == nil
		end
	end
	error(`no switch {label}`)
end

local function count(app: any, what: string): number
	return select(2, string.gsub(calls(app), what, ""))
end

test("compaction settings: three reasons, who decides each, with the engine's values", function()
	local app = open()
	local tree = app:tree(PAGE)
	for _, w in {
		"A chat compacts for one of three reasons",
		"The cache is about to expire",
		"An idle chat compacts before its 1-hour prompt cache",
		"Zeron",
		"Only chats larger than",
		"Before resuming at a limit reset",
		"When a usage limit resets, a chat past the idle",
		"The work changed",
		"The agent calls compact for a new task; it runs when",
		"Agent",
		"Remind it from",
		"Tokens above a fresh chat.",
		"The window is full",
		"Claude Code compacts on its own near the window and keeps",
		"Claude Code",
		"Window per chat: model picker",
		"Scope",
		"Mission threads",
		"Lead and step threads compact like any chat.",
		"100K",
		"150K",
		"200K",
		"Off",
	} do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	for _, w in { "Classifier", "Compact ahead", "Check from", "Jev" } do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: false`)
	end
	expect(checked(app, "The cache is about to expire")).to_be(true)
	expect(checked(app, "Before resuming at a limit reset")).to_be(true)
	expect(checked(app, "The work changed")).to_be(true)
	expect(checked(app, "Mission threads")).to_be(true)
	expect(calls(app)).to_contain("settings.engine:GetCompactionSettings")
	expect(calls(app)).to_contain("settings.engine:GetToolSettings")
	expect(app:error()).to_be(nil)
end)

test("compaction settings: each change goes to the engine, the agent's switch to the compact tool", function()
	local app = open()
	app:click(`{PAGE} >> role:switch "Before resuming at a limit reset"`)
	app:frames(20)
	expect(calls(app)).to_contain("settings.engine:SetCompactionSettings")
	expect(checked(app, "Before resuming at a limit reset")).to_be(false)
	app:click(`{PAGE} >> role:switch "The cache is about to expire"`)
	app:frames(20)
	expect(checked(app, "The cache is about to expire")).to_be(false)
	app:click(`{PAGE} >> role:switch "The work changed"`)
	app:frames(20)
	expect(calls(app)).to_contain("settings.engine:SetToolSettings")
	expect(checked(app, "The work changed")).to_be(false)
	app:click(`{PAGE} >> role:switch "Mission threads"`)
	app:frames(20)
	expect(checked(app, "Mission threads")).to_be(false)
	-- Back on the page, it reads what the engine now has. (Settings open, a ⌘, would leave it.)
	app:click('#settings-nav >> "General"')
	app:frames(20)
	app:click('#settings-nav >> "Compaction"')
	app:frames(30)
	expect(checked(app, "The cache is about to expire")).to_be(false)
	expect(checked(app, "Before resuming at a limit reset")).to_be(false)
	expect(checked(app, "The work changed")).to_be(false)
	expect(checked(app, "Mission threads")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("compaction settings: a reason's own setting is out of reach while the reason is off", function()
	local app = open()
	local before = count(app, "SetCompactionSettings")
	app:click(`{PAGE} >> "200K"[1]`)
	app:frames(20)
	expect(count(app, "SetCompactionSettings")).to_be(before + 1)
	app:click(`{PAGE} >> "Off"[1]`)
	app:frames(20)
	expect(count(app, "SetCompactionSettings")).to_be(before + 2)
	app:click(`{PAGE} >> role:switch "The cache is about to expire"`)
	app:frames(20)
	app:click(`{PAGE} >> role:switch "The work changed"`)
	app:frames(20)
	local off = count(app, "SetCompactionSettings")
	app:click(`{PAGE} >> "100K"[1]`)
	app:frames(20)
	app:click(`{PAGE} >> "150K"[2]`)
	app:frames(20)
	expect(count(app, "SetCompactionSettings")).to_be(off)
	app:click(`{PAGE} >> role:switch "The cache is about to expire"`)
	app:frames(20)
	app:click(`{PAGE} >> "100K"[1]`)
	app:frames(20)
	expect(count(app, "SetCompactionSettings")).to_be(off + 2)
	expect(app:error()).to_be(nil)
end)

test("compaction settings: a refused change goes back and says why under the rows", function()
	local app = open()
	expect(app:has("#compaction-refused")).to_be(false)
	app:command("settings_stub.refuse_next")
	app:click(`{PAGE} >> role:switch "Mission threads"`)
	app:frames(20)
	expect(checked(app, "Mission threads")).to_be(true)
	expect(app:text("#compaction-refused")).to_be("refused")
	app:click(`{PAGE} >> role:switch "Mission threads"`)
	app:frames(20)
	expect(app:has("#compaction-refused")).to_be(false)
	expect(checked(app, "Mission threads")).to_be(false)
end)
