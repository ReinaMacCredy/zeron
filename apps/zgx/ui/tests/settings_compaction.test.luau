-- Settings → Compaction (screens/settings/compaction), on the shell in
-- ui/shell_fixture.luau with the engine's answers of ui/settings_stub.luau:
-- gpui's rows with the engine's values, each change written to the engine,
-- the agents' switch as the `compact` tool's, the Remind/Check row's name
-- following the mode, and a refused change put back with its words.

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

test("compaction settings: gpui's rows with the engine's values", function()
	local app = open()
	local tree = app:tree(PAGE)
	for _, w in {
		"When idle",
		"Compact idle chats before the cache expires",
		"Idle threshold",
		"Smaller chats are left alone.",
		"During a chat",
		"Let agents compact themselves",
		"In tasks",
		"Check from",
		"100K",
		"150K",
		"200K",
		"Off",
	} do
		expect(`{w}: {tree:find(w, 1, true) ~= nil}`).to_be(`{w}: true`)
	end
	expect(checked(app, "Compact idle chats before the cache expires")).to_be(true)
	expect(checked(app, "Let agents compact themselves")).to_be(true)
	expect(calls(app)).to_contain("settings.engine:GetCompactionSettings")
	expect(calls(app)).to_contain("settings.engine:GetToolSettings")
	expect(app:error()).to_be(nil)
end)

test("compaction settings: each change goes to the engine, the agents' switch to the compact tool", function()
	local app = open()
	app:click(`{PAGE} >> role:switch "Compact idle chats before the cache expires"`)
	app:frames(20)
	expect(calls(app)).to_contain("settings.engine:SetCompactionSettings")
	expect(checked(app, "Compact idle chats before the cache expires")).to_be(false)
	app:click(`{PAGE} >> role:switch "Let agents compact themselves"`)
	app:frames(20)
	expect(calls(app)).to_contain("settings.engine:SetToolSettings")
	expect(checked(app, "Let agents compact themselves")).to_be(false)
	app:click(`{PAGE} >> role:switch "In tasks"`)
	app:frames(20)
	expect(checked(app, "In tasks")).to_be(false)
	local before = select(2, string.gsub(calls(app), "SetCompactionSettings", ""))
	app:click(`{PAGE} >> "200K"[1]`)
	app:frames(20)
	expect(select(2, string.gsub(calls(app), "SetCompactionSettings", ""))).to_be(before + 1)
	-- Back on the page, it reads what the engine now has. (Settings open, a ⌘, would leave it.)
	app:click('#settings-nav >> "General"')
	app:frames(20)
	app:click('#settings-nav >> "Compaction"')
	app:frames(30)
	expect(checked(app, "Compact idle chats before the cache expires")).to_be(false)
	expect(checked(app, "Let agents compact themselves")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("compaction settings: a refused change goes back and says why under the rows", function()
	local app = open()
	expect(app:has("#compaction-refused")).to_be(false)
	app:command("settings_stub.refuse_next")
	app:click(`{PAGE} >> role:switch "In tasks"`)
	app:frames(20)
	expect(checked(app, "In tasks")).to_be(true)
	expect(app:text("#compaction-refused")).to_be("refused")
	app:click(`{PAGE} >> role:switch "In tasks"`)
	app:frames(20)
	expect(app:has("#compaction-refused")).to_be(false)
	expect(checked(app, "In tasks")).to_be(false)
end)
