-- The command palette as a gpui user reaches for it every day: opened over
-- what had the keys, typed into at once, arrows, Enter, Escape, and the keys
-- back where they were; rows that stay hits keep their place while typing.
-- On the shell in ui/shell_fixture.luau. (Its shortcut, ⌘K, is a menu key:
-- these run its command; the key itself is on the Mac list, U27-M15.)

local SIDE = "#shell-sidebar"
local FIELD = 'role:textbox "Search"'
local LIST = "#palette-list"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	return app
end

local function focused(app: any, what: string): boolean
	return string.find(app:tree(what), " focused", 1, true) ~= nil
end

-- The options listed, top to bottom.
local function options(app: any): { string }
	local out = {}
	for line in app:tree(LIST):gmatch("[^\n]+") do
		local name = line:match(' option "([^"]+)"')
		if name then
			table.insert(out, name)
		end
	end
	return out
end

-- Where an option stands (its top), by its label.
local function top_of(app: any, label: string): number?
	for line in app:tree(LIST):gmatch("[^\n]+") do
		if line:find(` option "{label}"`, 1, true) then
			return tonumber(line:match(" %-?%d+,(%-?%d+) "))
		end
	end
	return nil
end

test("the palette opens over the composer with the keys, Escape closes it at once, and the composer has them back", function()
	local app = open()
	app:click(`{SIDE} >> role:button "Write the docs"`)
	app:frames(20)
	app:click('role:textbox "Message"')
	app:frames(5)
	expect(focused(app, 'role:textbox "Message"')).to_be(true)
	app:command("shell.palette")
	app:frames(20)
	expect(app:state("shell_palette_open")).to_be(true)
	-- Typed into at once: the field has the keys.
	expect(focused(app, FIELD)).to_be(true)
	app:type(FIELD, "tidy")
	app:frames(20)
	expect(options(app)[1]).to_be("Tidy the tests")
	expect(app:has('#command-palette >> "Close"')).to_be(true)
	-- Escape once: the palette closes at once, typed text and all (Zeron's).
	app:key("Escape")
	app:frames(20)
	expect(app:state("shell_palette_open")).to_be(false)
	expect(focused(app, 'role:textbox "Message"')).to_be(true)
end)

test("Escape gives the keys back to whatever had them, not to the composer", function()
	local app = open()
	local row = `{SIDE} >> role:button "Write the docs"`
	app:click(row)
	app:frames(20)
	expect(focused(app, row)).to_be(true)
	app:command("shell.palette")
	app:frames(20)
	expect(focused(app, FIELD)).to_be(true)
	app:type(FIELD, "ship")
	app:frames(10)
	app:key("Escape")
	app:frames(20)
	expect(app:state("shell_palette_open")).to_be(false)
	expect(focused(app, row)).to_be(true)
	expect(focused(app, 'role:textbox "Message"')).to_be(false)
end)

test("arrows move the highlight round the ends and Enter does the row it's on", function()
	local app = open()
	app:command("shell.palette")
	app:frames(20)
	app:type(FIELD, "the")
	app:frames(20)
	expect(options(app)[1]).to_be("Fix the build")
	expect(options(app)[2]).to_be("Write the docs")
	app:key("ArrowDown")
	app:key("ArrowDown")
	app:key("ArrowUp")
	app:frames(5)
	app:key("Enter")
	app:frames(20)
	expect(app:state("shell_palette_open")).to_be(false)
	expect(app:state("shell_route_path")).to_be("/session/a2")
	-- Sessions alone (⌘2): up from the first is the last.
	app:command("shell.palette")
	app:frames(20)
	app:type(FIELD, "the")
	app:key("cmd+2")
	app:frames(20)
	local list = options(app)
	expect(list[#list]).to_be("Answer the review")
	app:key("ArrowUp")
	app:frames(5)
	app:key("Enter")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/session/b2")
end)

test("typing on, a row that stays a hit keeps its place and the card its top", function()
	local app = open()
	app:command("shell.palette")
	app:frames(20)
	local function card_top(): number?
		return tonumber((app:tree("#command-palette"):match(" %-?%d+,(%-?%d+) %d+x%d+")))
	end
	local top = card_top()
	expect(top).never.to_be_nil()
	app:type(FIELD, "tid")
	app:frames(20)
	expect(options(app)[1]).to_be("Tidy the tests")
	local y = top_of(app, "Tidy the tests")
	for _, ch in { "y", " ", "t" } do
		app:type(FIELD, ch)
		-- Every frame of the change: the row there, where it was.
		for _ = 1, 6 do
			app:frames(1)
			expect(top_of(app, "Tidy the tests")).to_be(y)
		end
		expect(card_top()).to_be(top)
	end
end)

test("the chat open is left out until it's searched for; side chats come by search alone", function()
	local app = open()
	app:command("side_chats")
	app:click(`{SIDE} >> role:button "Write the docs"`)
	app:frames(20)
	app:command("shell.palette")
	app:frames(20)
	app:key("cmd+2")
	app:frames(20)
	local names = table.concat(options(app), "|")
	expect(names).never.to_contain("Write the docs")
	expect(names).to_contain("Fix the build")
	expect(names).never.to_contain("Look into the flake")
	app:type(FIELD, "docs")
	app:frames(20)
	expect(app:has(`{LIST} >> role:option "Write the docs"`)).to_be(true)
	app:fill(FIELD, "flake")
	app:frames(20)
	expect(options(app)[1]).to_be("Look into the flake")
	app:key("Enter")
	app:frames(30)
	expect(app:state("shell_palette_open")).to_be(false)
	-- (A side chat opens in its parent, as the sidebar's lanes card does.)
	expect(app:state("shell_route_path")).to_be("/session/a1")
end)

test("a setting found by its name opens Settings at its row, and each page added is a place to go", function()
	local app = open()
	app:command("shell.palette")
	app:frames(20)
	-- (None listed with nothing typed: Settings' scope holds its pages and switches alone.)
	local function in_scope(): number
		return tonumber((app:tree('#command-palette >> role:button "Settings"'):match('text "(%d+)"'))) or -1
	end
	local idle = in_scope()
	expect(idle > 0 and idle < 60).to_be(true)
	app:type(FIELD, "follow-up")
	app:frames(20)
	expect(app:has(`{LIST} >> role:option "Follow-up behavior"`)).to_be(true)
	expect(app:tree(`{LIST} >> role:option "Follow-up behavior"`)).to_contain("Chat")
	app:click(`{LIST} >> role:option "Follow-up behavior"`)
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/settings/chat")
	expect(app:state("settings_search_lit")).to_be("follow_up")
	for _, page in { { "Routing", "routing" }, { "Tools", "tools" }, { "Experimental", "experimental" }, { "Chat", "chat" } } do
		app:command("shell.palette")
		app:frames(20)
		app:type(FIELD, page[1])
		app:frames(20)
		app:click(`{LIST} >> role:option "Settings: {page[1]}"`)
		app:frames(20)
		expect(app:state("shell_route_path")).to_be(`/settings/{page[2]}`)
	end
end)

test("a row's keys follow Settings → Shortcuts, and its branch and agent the sidebar's Show", function()
	local app = open()
	app:set_state("shell_keymap", { ["shell.new_chat"] = "cmd+shift+y" })
	app:command("shell.palette")
	app:frames(20)
	-- (⌘K's own cap at the field's end.)
	expect(app:tree("#command-palette")).to_contain('text "K"')
	app:type(FIELD, "new chat")
	app:frames(20)
	local row = app:tree(`{LIST} >> role:option "New chat"`)
	expect(row).to_contain('text "Y"')
	expect(row).never.to_contain('text "N"')
	app:key("Escape")
	app:frames(20)
	app:command("shell.palette")
	app:frames(20)
	app:type(FIELD, "fix the")
	app:frames(20)
	expect(app:tree(`{LIST} >> role:option "Fix the build"`)).to_contain("fix/build")
	app:key("Escape")
	app:frames(20)
	app:set_state("shell_sidebar_show_branch", false)
	app:frames(5)
	app:command("shell.palette")
	app:frames(20)
	app:type(FIELD, "fix the")
	app:frames(20)
	expect(app:tree(`{LIST} >> role:option "Fix the build"`)).never.to_contain("fix/build")
end)

-- (The drop itself, on a composer, a split or the sidebar, needs frames while the pointer moves, which
-- a test's drag doesn't give: on the Mac list, U27-M16.)
test("a lifted row let go where nothing takes it brings the palette back as it was", function()
	local app = open()
	app:command("shell.palette")
	app:frames(20)
	app:type(FIELD, "tidy")
	app:frames(20)
	app:drag(`{LIST} >> role:option "Tidy the tests"`, "#palette-above")
	app:frames(30)
	expect(app:state("shell_palette_open")).to_be(true)
	expect(app:text(FIELD)).to_be("tidy")
	expect(focused(app, FIELD)).to_be(true)
	expect(options(app)[1]).to_be("Tidy the tests")
end)

test("with the palette open, the menu's ⌘1–⌘4 pick its scopes rather than jump to a session", function()
	local app = open()
	app:command("shell.palette")
	app:frames(20)
	local before = app:state("shell_route_path")
	app:command("shell.session_2")
	app:frames(20)
	expect(app:state("shell_palette_open")).to_be(true)
	expect(app:state("shell_route_path")).to_be(before)
	for _, name in options(app) do
		expect(name).never.to_be("New chat")
	end
	expect(options(app)[1]).to_be("Fix the build")
	app:command("shell.session_3")
	app:frames(20)
	expect(options(app)[1]).to_be("New chat")
	-- Closed, the jump is the jump.
	app:key("Escape")
	app:frames(20)
	app:command("shell.session_2")
	app:frames(20)
	expect(app:state("shell_route_path")).never.to_be(before)
end)

test("⌘, opens Settings and a second leaves it, back where the chats were", function()
	local app = open()
	app:click(`{SIDE} >> role:button "Write the docs"`)
	app:frames(20)
	app:command("shell.settings")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/settings/general")
	app:command("shell.settings")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/session/a2")
end)
