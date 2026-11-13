-- The shell from the keys alone: Tab reaches what's there to use, in the
-- order it reads (title bar, sidebar from its head, the page, the panel),
-- and nothing hidden. On the shell in ui/shell_fixture.luau.

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	return app
end

-- The focused node's line, as the tree says. (The window's dump stops
-- past a budget: the title bar, drawn last, is read on its own too.)
local function focused(app: any): string
	-- (The whole tree stops at 200 nodes, and the title bar and the panel's
	-- edge, layers drawn last, can fall past that.)
	local trees = { app:tree(), app:tree("#titlebar") }
	if app:has('role:slider "Resize files"') then
		table.insert(trees, app:tree('role:slider "Resize files"'))
	end
	for _, tree in trees do
		for line in tree:gmatch("[^\n]+") do
			if line:find(" focused", 1, true) then
				return line
			end
		end
	end
	return ""
end

-- Tab `n` times from where focus is: each stop's line, in order.
local function walk(app: any, n: number): { string }
	local stops = {}
	for _ = 1, n do
		app:key("Tab")
		app:frames(4)
		table.insert(stops, focused(app))
	end
	return stops
end

local function find(stops: { string }, what: string): number?
	for i, line in stops do
		if line:find(what, 1, true) then
			return i
		end
	end
	return nil
end

test("the panel's edge is a Tab stop only while the panel shows", function()
	local app = open()
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	local closed = walk(app, 60)
	expect(find(closed, "Resize files")).to_be_nil()
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	local opened = walk(app, 80)
	expect(find(opened, "Resize files")).never.to_be_nil()
end)

test("Tab reads each part of the shell in its order: the sidebar's rows, the page, its panel, the title bar left to right", function()
	-- (The title bar, the sidebar's head and the panel's edge are layers
	-- over what's under them: they stay where they paint, last, so a press
	-- reaches what it's over; zgx has no tab order of its own to give.)
	local app = open()
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	local stops = walk(app, 90)
	local from = find(stops, '"New chat"') :: number
	local round = table.move(stops, from, #stops, 1, {})
	local function at(what: string): number
		local i = find(round, what)
		if not i then
			error(`no stop at {what}:\n{table.concat(round, "\n")}`)
		end
		return i :: number
	end
	local order = {
		'"New chat"',
		'"Fix the build"',
		'textbox "Message"',
		'"Browser"',
		'"Resize files"',
		'"Toggle sidebar"',
		'"Back"',
		'"Panel"',
	}
	for i = 2, #order do
		if at(order[i - 1]) >= at(order[i]) then
			error(`{order[i - 1]} should come before {order[i]}:\n{table.concat(round, "\n")}`)
		end
	end
end)
test("I with no text field holding the keyboard goes into the composer; Escape there gives the keyboard back", function()
	local app = open()
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	app:click("role:textbox")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
	app:key("Escape")
	app:frames(5)
	expect(focused(app)).never.to_contain("textbox")
	app:key("i")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
	-- Typed into the composer, I is a letter of the draft.
	app:key("i")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
end)
test("a letter with a control of the page focused stays with that control", function()
	local app = open()
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	app:click("role:textbox")
	app:frames(5)
	app:key("Escape")
	app:frames(5)
	app:key("Tab")
	app:frames(5)
	local n = 0
	while n < 40 and not string.find(focused(app), 'button "Access"', 1, true) do
		app:key("Tab")
		app:frames(2)
		n += 1
	end
	expect(focused(app)).to_contain('button "Access"')
	app:key("x")
	app:frames(5)
	expect(focused(app)).to_contain('button "Access"')
end)

test("with nothing holding the keys a letter or Cmd+V goes to the composer; an arrow or another chord doesn't", function()
	local app = open()
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	app:click("role:textbox")
	app:frames(5)
	app:key("Escape")
	app:frames(5)
	expect(focused(app)).never.to_contain("textbox")
	app:key("ArrowDown")
	app:frames(5)
	expect(focused(app)).never.to_contain("textbox")
	app:key("cmd+shift+y")
	app:frames(5)
	expect(focused(app)).never.to_contain("textbox")
	app:key("q")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
	app:key("Escape")
	app:frames(5)
	expect(focused(app)).never.to_contain("textbox")
	app:key("cmd+v")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
end)
test("⌘F with nothing focused taking it opens a session's find bar, and nothing on a new chat", function()
	local app = open()
	-- (The bar draws over a transcript, which this fixture's chat has none
	-- of: whether it's open is what's asked.)
	app:command("shell.find")
	app:frames(20)
	expect(app:state("chat.find.shown") == true).to_be(false)
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	app:command("shell.find")
	app:frames(20)
	expect(app:state("chat.find.shown")).to_be(true)
end)

-- A selection over chat a1's reply (the fixture's stand-in for the host's), the press let go there.
local function select_reply(app: any, within: string?)
	local scope = if within then `{within} >> #transcript` else "#transcript"
	local line
	for l in app:tree(scope):gmatch("[^\n]+") do
		if l:find('"Line 1 of a1.', 1, true) or l:find('"This is a1"', 1, true) then
			local x, y, w = l:match(" (%-?%d+),(%-?%d+) (%d+)x%d+")
			if x then
				line = { x = tonumber(x) :: number + 10, y = tonumber(y) :: number, w = 120, h = 20 }
				break
			end
		end
	end
	expect(line ~= nil).to_be(true)
	app:set_state("fake_selection", { text = "This is a1", top = line, bottom = line })
	-- (Off the sidebar's row first: its card would stand over the reply.)
	app:hover(scope)
	app:frames(10)
	app:click(`{scope} >> ~"This is a1"`)
	app:frames(30)
	expect(app:has("#annotation-pill")).to_be(true)
end

test("with a selection's pill shown or put away, a letter or a paste still goes to the composer, and Space presses a word", function()
	local app = open()
	app:set_state("fixture.chats", { a1 = { session = { id = "a1", cwd = "/work/atlas", model = "opus", state = "ready" }, messages = { { id = "a1-0", role = "user", text = "Say who you are, please." }, { id = "a1-m", role = "assistant", text = "This is a1" } } } })
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	select_reply(app)
	-- (The pill takes no keys: nothing holds them, as before it showed.)
	expect(focused(app)).to_be("")
	app:key("q")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
	-- Escape puts it away, the keys still parked: not on the composer's Side chat tab.
	select_reply(app)
	app:key("Escape")
	app:frames(20)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(focused(app)).to_be("")
	app:key("q")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
	-- From a word Tab reached: Escape parks the keys, a letter or a paste goes to the composer.
	select_reply(app)
	app:key("Tab")
	expect(focused(app)).to_contain('button "Mention"')
	app:key("Escape")
	app:frames(20)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(focused(app)).to_be("")
	select_reply(app)
	app:key("Tab")
	app:key("ctrl+v")
	app:frames(5)
	expect(focused(app)).to_contain("textbox")
	-- Space presses the word that has the keys (Mention: the line in the draft), never the composer's Side chat tab.
	app:fill("role:textbox", "")
	select_reply(app)
	app:key("Tab")
	expect(focused(app)).to_contain('button "Mention"')
	app:key("Space")
	app:frames(30)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:state("chat.composer.text")).to_be("> Agent: This is a1\n\n")
	expect(focused(app)).to_contain("textbox")
	expect(app:has("#side-chat-panel") or app:has(".side-chat-tab")).to_be(false)
end)
