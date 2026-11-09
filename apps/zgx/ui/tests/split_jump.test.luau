-- "Jump to latest" in a split's pane: the pill has the keys as it's clicked and
-- leaves as the view arrives. The focus stays on that pane's chat, not on the
-- next control's, another pane's composer. On the shell in ui/shell_fixture.luau.

local function long_chat(id: string): { any }
	local messages = {}
	for i = 1, 60 do
		table.insert(messages, { id = `{id}-{i}`, role = "assistant", text = string.rep(`Line {i} of {id}. `, 40) })
	end
	return messages
end

local function leaf(id: string): any
	return { kind = "leaf", id = id }
end

local function scrolled_up(app: any, id: string)
	app:scroll(`#lane-split-{id} >> #transcript`, -1500)
	app:frames(30)
	expect(app:wait(`#lane-split-{id} >> role:button "Jump to latest"`, { timeout_ms = 2000 })).to_be(true)
end

test("Jump to latest keeps the focus on the pane it was pressed in", function()
	local app = mount("shell_fixture.luau")
	app:resize(1500, 800)
	app:frames(30)
	app:set_state("fixture.chats", { a1 = { messages = long_chat("a1") }, a2 = { messages = long_chat("a2") } })
	app:set_state("inset_split_trees", { { kind = "row", children = { leaf("a1"), leaf("a2") } } })
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(60)
	expect(app:state("fixture_open_chat")).to_be("a1")
	scrolled_up(app, "a1")
	app:click('#lane-split-a1 >> role:button "Jump to latest"')
	expect(app:wait('#lane-split-a1 >> role:button "Jump to latest"', { gone = true, timeout_ms = 2000 })).to_be(true)
	app:frames(30)
	expect(app:state("fixture_open_chat")).to_be("a1")
end)

test("Jump to latest in a pane that wasn't focused focuses that pane", function()
	local app = mount("shell_fixture.luau")
	app:resize(1500, 800)
	app:frames(30)
	app:set_state("fixture.chats", { a1 = { messages = long_chat("a1") }, a2 = { messages = long_chat("a2") } })
	app:set_state("inset_split_trees", { { kind = "row", children = { leaf("a1"), leaf("a2") } } })
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(60)
	scrolled_up(app, "a2")
	app:click('#lane-split-a2 >> role:button "Jump to latest"')
	expect(app:wait('#lane-split-a2 >> role:button "Jump to latest"', { gone = true, timeout_ms = 2000 })).to_be(true)
	app:frames(30)
	expect(app:state("fixture_open_chat")).to_be("a2")
end)

-- A selection over pane `id`'s reply (the fixture's stand-in for the host's), the press let go there.
local function two_chats(app: any)
	local function chat(id: string): any
		return {
			session = { id = id, cwd = "/work/atlas", model = "opus", state = "ready" },
			messages = { { id = `{id}-0`, role = "user", text = "Say who you are, please." }, { id = `{id}-m`, role = "assistant", text = `This is {id}` } },
		}
	end
	app:set_state("fixture.chats", { a1 = chat("a1"), a2 = chat("a2") })
	app:set_state("inset_split_trees", { { kind = "row", children = { leaf("a1"), leaf("a2") } } })
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(60)
	expect(app:state("chat.composer.here")).to_be("a1")
end
local function select_in(app: any, id: string)
	local scope = `#lane-split-{id} >> #transcript`
	local line
	for l in app:tree(scope):gmatch("[^\n]+") do
		if l:find(`"This is {id}"`, 1, true) then
			local x, y = l:match(" (%-?%d+),(%-?%d+) %d+x%d+")
			line = { x = tonumber(x) :: number + 10, y = tonumber(y) :: number, w = 60, h = 20 }
		end
	end
	expect(line ~= nil).to_be(true)
	app:set_state("fake_selection", { text = `This is {id}`, top = line, bottom = line })
	app:hover(scope)
	app:frames(10)
	app:click(`{scope} >> ~"This is {id}"`)
	app:frames(30)
	expect(app:has(`#lane-split-{id} >> #annotation-pill`)).to_be(true)
end
local function focused(app: any): string
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(" focused", 1, true) then
			return line
		end
	end
	return ""
end
local function draft_of(app: any, id: string): string
	local d = (app:state("chat.composer.drafts") or {})[id]
	return if d then d.text or "" else ""
end
local WINDOW = "#shell-page >> role:textbox"

test("a selection's pill in a split's pane never moves the split: Escape or a word leaves the focused pane as it was", function()
	local app = mount("shell_fixture.luau")
	app:resize(1500, 800)
	app:frames(30)
	two_chats(app)
	app:type(WINDOW, "hello world")
	select_in(app, "a1")
	expect(focused(app)).to_be("")
	app:key("Escape")
	app:frames(30)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:state("chat.composer.here")).to_be("a1")
	expect(app:state("fixture_open_chat")).to_be("a1")
	expect(app:text(WINDOW)).to_be("hello world")
	-- From a word Tab reached, too.
	select_in(app, "a1")
	app:key("Tab")
	expect(focused(app)).to_contain('button "Mention"')
	app:key("Escape")
	app:frames(30)
	expect(app:state("chat.composer.here")).to_be("a1")
	expect(focused(app)).to_be("")
	-- A letter then goes on to the window's composer, a1's.
	app:key("q")
	app:frames(5)
	expect(focused(app)).to_contain('textbox "Message" ="hello world"')
end)

test("Mention in a split's pane writes into that pane's chat at the window composer's caret; the other chat's draft is untouched", function()
	local app = mount("shell_fixture.luau")
	app:resize(1500, 800)
	app:frames(30)
	two_chats(app)
	app:type(WINDOW, "hello world")
	-- (The caret before "world".)
	for _ = 1, 5 do
		app:key("Left")
	end
	select_in(app, "a1")
	app:click("#lane-split-a1 >> #annotation-mention")
	app:frames(30)
	expect(app:state("chat.composer.here")).to_be("a1")
	expect(app:text(WINDOW)).to_be("hello\n\n> Agent: This is a1\n\nworld")
	expect(focused(app)).to_contain(`textbox "Message" ="hello`)
	expect(draft_of(app, "a2")).never.to_contain("This is a1")
	-- The other pane: pressed in, it's focused, and the line goes into its chat.
	select_in(app, "a2")
	expect(app:state("chat.composer.here")).to_be("a2")
	app:click("#lane-split-a2 >> #annotation-mention")
	app:frames(30)
	expect(app:state("chat.composer.here")).to_be("a2")
	expect(app:text(WINDOW)).to_contain("> Agent: This is a2")
	expect(app:text(WINDOW)).never.to_contain("This is a1")
	expect(draft_of(app, "a1")).to_be("hello\n\n> Agent: This is a1\n\nworld")
end)

test("Add to chat in a split's pane gives the comment the keys: Enter takes the comment, not the window's composer", function()
	local app = mount("shell_fixture.luau")
	app:resize(1500, 800)
	app:frames(30)
	two_chats(app)
	app:type(WINDOW, "hello")
	select_in(app, "a1")
	app:click("#lane-split-a1 >> #annotation-add-to-chat")
	app:frames(30)
	expect(app:tree("#annotation-comment")).to_contain(" focused")
	app:key("w")
	app:frames(5)
	expect(app:tree("#annotation-comment")).to_contain(" focused")
	app:type("#annotation-comment", "why?")
	app:key("Enter")
	app:frames(30)
	expect(app:has("#lane-split-a1 >> #annotation-badge-1")).to_be(true)
	expect(app:text(WINDOW)).to_be("hello")
	expect(app:state("chat.composer.here")).to_be("a1")
	expect(focused(app)).to_contain('textbox "Message" ="hello"')
end)
