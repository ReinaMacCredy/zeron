-- U24's row kinds on ui/chat_fixture.luau (`kinds_turn`): file links, tool
-- rows by kind, questions, the secret card, Claude Code cards, and the
-- chat's find over all of them.

local function kinds(): any
	local app = mount("chat_fixture.luau")
	app:resize(1100, 2400)
	app:command("go_s2")
	app:frames(20)
	app:command("kinds_turn")
	app:frames(40)
	expect(app:error()).to_be(nil)
	return app
end

local function settle(app: any)
	for _ = 1, 4 do
		app:wait("#never-shown", { timeout_ms = 20 })
		app:frames(5)
	end
end

local function tally(app: any): string
	return app:tree("#chat-find"):match('text "(%d+ of %d+)"') or app:tree("#chat-find"):match('text "(No results)"') or ""
end

local function find(app: any, q: string): string
	app:command("find_open")
	settle(app)
	app:fill('role:textbox "Find"', q)
	settle(app)
	return tally(app)
end

test("kinds: a file link opens its file at its line in the chat's panel; one outside its folder opens nothing", function()
	local app = kinds()
	app:click('role:link "parse.rs"')
	app:click('role:link "main.rs"')
	app:click('role:link "this one"')
	app:frames(5)
	local calls = app:state("calls")
	expect(calls).to_contain("open_file:s2:src/parse.rs@12")
	expect(calls).to_contain("open_file:s2:src/main.rs@40")
	expect(calls).never.to_contain("hosts.conf")
end)

test("kinds: a resolved file link has its trailing glyph; a paragraph of one file link, or a bare file name, is a file row", function()
	local app = kinds()
	-- (A glyph keeps its room with word joiners, three a glyph: parse.rs and
	-- main.rs have one; the link outside the folder and the web's don't.)
	local words = app:text('role:text ~"The parser is"')
	local _, joiners = string.gsub(words, "\u{2060}", "")
	expect(joiners).to_be(6)
	local tree = app:tree("#transcript")
	local _, rows = string.gsub(tree, "img 211,%d+ 14x14\n", "")
	expect(rows >= 2).to_be(true)
	expect(app:has('.md-file-row >> "Cargo.toml"')).to_be(true)
	expect(app:has('.md-file-row >> "version 1.2"')).to_be(false)
	app:click('.md-file-row >> role:link "chat.rs"')
	app:frames(5)
	expect(app:state("calls")).to_contain("open_file:s2:src/chat.rs@7")
end)

test("kinds: a tool row's file name opens the file without folding the row; a skill's opens its SKILL.md", function()
	local app = kinds()
	app:click('role:button ~"Read 1 file"')
	app:frames(30)
	expect(app:has('role:button ~"Waited for agents"')).to_be(true)
	expect(app:has('role:button ~"zeron.device_screenshot"')).to_be(true)
	app:click('.tool-target >> "parse.rs"')
	app:click('.tool-target >> "zgx-port"')
	app:frames(5)
	local calls = app:state("calls")
	expect(calls).to_contain("open_file:s2:src/parse.rs")
	expect(calls).to_contain("open_file:s2:skills/zgx-port/SKILL.md")
	expect(app:has('role:button ~"Waited for agents"')).to_be(true)
	expect(app:tree("#transcript"):find("fn parse_all", 1, true)).to_be(nil)
	app:click('role:button ~"Read parse.rs"')
	app:frames(30)
	expect(app:tree("#transcript"):find("fn parse_all", 1, true) ~= nil).to_be(true)
end)

test("kinds: a question shows what was answered, one line a question; a waiting one says so", function()
	local app = kinds()
	local tree = app:tree("#transcript")
	expect(tree:find("2 questions", 1, true) ~= nil).to_be(true)
	expect(tree:find("Scope → Branch", 1, true) ~= nil).to_be(true)
	expect(tree:find("Reviewer → Codex", 1, true) ~= nil).to_be(true)
	expect(tree:find("Plan → Skipped", 1, true) ~= nil).to_be(true)
	expect(tree:find("Awaiting your answer…", 1, true) ~= nil).to_be(true)
end)

test("kinds: a secret card saves what's typed, then says it's saved; nothing typed saves nothing", function()
	local app = kinds()
	expect(app:has("#secret-card-ka1:x1")).to_be(true)
	app:click("#secret-save-ka1:x1")
	app:frames(5)
	expect(app:state("calls")).never.to_contain("secret:s2:req-1")
	app:fill('#secret-card-ka1:x1 >> role:textbox', "tskey-123")
	app:key("Enter")
	app:frames(10)
	expect(app:state("calls")).to_contain("secret:s2:req-1:tskey-123")
	expect(app:has("#secret-card-ka1:x1")).to_be(false)
	expect(app:tree("#transcript"):find("Secret saved", 1, true) ~= nil).to_be(true)
end)

test("kinds: Claude Code's cards: an answer with Copy as Markdown, a command still running, one only its terminal runs", function()
	local app = kinds()
	local tree = app:tree("#transcript")
	expect(tree:find("Claude Code · command output", 1, true) ~= nil).to_be(true)
	expect(tree:find("Total duration: 4m", 1, true) ~= nil).to_be(true)
	expect(tree:find("Running in Claude Code…", 1, true) ~= nil).to_be(true)
	expect(tree:find("only runs in Claude Code's terminal.", 1, true) ~= nil).to_be(true)
	expect(tree:find("Theme → dark", 1, true) ~= nil).to_be(true)
	app:click('role:button "Copy as Markdown"')
	app:frames(3)
	expect(app:clipboard()).to_be("Total cost: 1.2M tokens\nTotal duration: 4m")
	expect(app:has('role:button "Copied"')).to_be(true)
end)

test("kinds: find in chat finds words in every new row kind", function()
	local app = kinds()
	for q, n in { ["Reviewer → Codex"] = "1 of 1", ["awaiting your"] = "1 of 1", ["tailscale auth"] = "1 of 1", ["secret saved"] = "1 of 1", ["total duration"] = "1 of 1", ["verbose"] = "1 of 1", ["feat/kinds"] = "1 of 1", ["/doctor"] = "1 of 1" } do
		expect(find(app, q)).to_be(n)
	end
end)

-- The y of the first line holding `words` in the transcript's tree, or nil.
local function y_of(app: any, words: string): number?
	for line in app:tree("#transcript"):gmatch("[^\n]+") do
		if line:find(words, 1, true) then
			return tonumber(line:match(" %-?%d+,(%-?%d+) "))
		end
	end
	return nil
end

local function long_middle(): any
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	app:command("go_s2")
	app:frames(20)
	app:command("kinds_long")
	app:frames(40)
	settle(app)
	app:scroll("#transcript", -1600)
	settle(app)
	return app
end

-- The first reply whose line shows in the view, and its y.
local function first_shown(app: any): (number, number)
	local tree = app:tree("#transcript")
	local top = tonumber(tree:match("^[^\n]* %-?%d+,(%-?%d+) ")) or 0
	for line in tree:gmatch("[^\n]+") do
		local n = line:match('"Reply (%d+) says')
		local y = tonumber(line:match(" %-?%d+,(%-?%d+) "))
		if n and y and y >= top then
			return tonumber(n) :: number, y
		end
	end
	error("no reply in view")
end

test("kinds: reading place holds while the reply just above the view grows", function()
	local app = long_middle()
	local n, before = first_shown(app)
	app:set_state("kinds_at", n - 1)
	app:command("kinds_grow_above")
	settle(app)
	expect(y_of(app, `Reply {n} says`)).to_be(before)
	expect(y_of(app, `Reply {n + 1} says`) ~= nil).to_be(true)
end)

test("kinds: reading place holds while a question just above the view is answered", function()
	local app = long_middle()
	local n, before = first_shown(app)
	app:set_state("kinds_at", n - 1)
	app:command("kinds_answer_above")
	settle(app)
	expect(y_of(app, `Reply {n} says`)).to_be(before)
end)

test("kinds: a steer in a long chat lands under the turn without jumping the view", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	app:command("go_s2")
	app:frames(20)
	app:command("kinds_long")
	app:frames(40)
	settle(app)
	app:command("start_reply")
	for _ = 1, 4 do
		app:command("stream_words")
		app:frames(4)
	end
	settle(app)
	expect(y_of(app, "Reply 40 says") ~= nil).to_be(true)
	app:command("kinds_steer")
	for _ = 1, 8 do
		app:frames(2)
		expect(y_of(app, "Prompt 1 asks")).to_be(nil)
		expect(y_of(app, "Prompt 2 asks")).to_be(nil)
	end
	settle(app)
	expect(y_of(app, "Also check the tests") ~= nil).to_be(true)
	expect(app:has('role:button "Jump to latest"')).to_be(false)
end)

test("kinds: a tool row without a file still opens from a click on its words", function()
	local app = kinds()
	app:click('role:button ~"Read 1 file"')
	app:frames(30)
	app:click('.tool-target >> "1/3 done"')
	app:frames(30)
	expect(app:tree("#transcript"):find("[ ] b", 1, true) ~= nil).to_be(true)
end)

test("kinds: focus coming into a reply shows its lane, its Copy the next stop", function()
	local app = kinds()
	expect(app:has('role:button "Copy message"')).to_be(false)
	app:command("agent_without_plan")
	app:click("#chat-composer >> role:textbox")
	app:frames(5)
	local function focused(): string
		for line in app:tree("#transcript"):gmatch("[^\n]+") do
			if line:find(" focused", 1, true) then
				return line
			end
		end
		return ""
	end
	for _ = 1, 10 do
		app:key("shift+Tab")
		app:frames(4)
		if focused():find('"Decline"', 1, true) then
			break
		end
	end
	expect(focused():find('"Decline"', 1, true) ~= nil).to_be(true)
	expect(app:has('role:button "Copy message"')).to_be(true)
	-- (Past the reply's own last stops: the error's Log in, then its Copy.)
	for _ = 1, 3 do
		app:key("Tab")
		app:frames(4)
		if focused():find('"Copy message"', 1, true) then
			break
		end
	end
	expect(focused():find('"Copy message"', 1, true) ~= nil).to_be(true)
	app:key("Enter")
	app:frames(3)
	expect(app:clipboard()).to_contain("The parser is")
end)

test("kinds: Decline sends no secret and the card says it was declined", function()
	local app = kinds()
	app:click("#secret-decline-ka1:x1")
	app:frames(10)
	expect(app:state("calls")).to_contain("secret:s2:req-1:declined")
	expect(app:has("#secret-card-ka1:x1")).to_be(false)
	expect(app:tree("#transcript"):find("Declined", 1, true) ~= nil).to_be(true)
end)

test("kinds: a tool row opened shows the pictures it touched; one opens the lightbox with the rest", function()
	local app = kinds()
	app:click('role:button ~"Read 1 file"')
	app:frames(30)
	expect(app:has(".tool-pictures")).to_be(false)
	app:click('role:button ~"zeron.device_screenshot"')
	app:frames(30)
	expect(app:has(".tool-pictures")).to_be(true)
	app:click('"Open shell.png"')
	app:frames(60)
	local open = app:state("ds/lightbox.luau::open")
	expect(open).never.to_be_nil()
	expect(#open.items).to_be(2)
	expect(open.items[open.index].src).to_be("assets/composer/shell.png")
end)

test("kinds: a sent prompt's pasted text is its chip, which opens the viewer", function()
	local app = kinds()
	expect(app:state("ds/pasted_text.luau::open")).to_be(nil)
	app:click('"Pasted text 2"')
	app:frames(10)
	local open = app:state("ds/pasted_text.luau::open")
	expect(open ~= nil and open.path == "/tmp/pasted/x.txt").to_be(true)
end)

test("kinds: an error is its chip, its words whole; one that signing in fixes says so, and Log in goes to Providers", function()
	local app = kinds()
	local tree = app:tree("#transcript")
	expect(tree:find("Codex exited with status 101", 1, true) ~= nil).to_be(true)
	expect(tree:find('"Signed out"', 1, true) ~= nil).to_be(true)
	expect(app:count('role:button "Log in"')).to_be(1)
	expect(find(app, "exited with status 101")).to_be("1 of 1")
	app:key("Escape")
	app:frames(5)
	app:click('role:button "Log in"')
	app:frames(10)
	expect(app:state("shell_route_path")).to_contain("providers")
end)

test("kinds: Claude Code's /model card lights the chat's model, and a pick changes it", function()
	local app = kinds()
	local function lit(name: string): boolean
		return app:tree("#transcript"):find(`button "{name}"[^\n]* checked`) ~= nil
	end
	-- (The chat's model as it is now, not the one the card was answered under.)
	expect(lit("Sonnet")).to_be(true)
	expect(lit("Opus")).to_be(false)
	app:click('role:button "Haiku"')
	app:frames(10)
	expect(app:state("calls")).to_contain("configure:s2:model=haiku")
	expect(lit("Haiku")).to_be(true)
	expect(app:tree("#transcript"):find("changes this chat's model chip", 1, true) ~= nil).to_be(true)
end)
