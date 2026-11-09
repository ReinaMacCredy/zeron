-- The transcript's row kinds on ui/chat_fixture.luau (`kinds_turn`): file links, tool rows by kind, questions, the secret card, Claude Code cards, and find over them.

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

test("kinds: a file link opens its file at its line in the chat's panel; one outside its folder by its whole path", function()
	local app = kinds()
	app:click('role:link "parse.rs"')
	app:click('role:link "main.rs"')
	app:click('role:link "this one"')
	app:frames(5)
	local calls = app:state("calls")
	expect(calls).to_contain("open_file:s2:src/parse.rs@12")
	expect(calls).to_contain("open_file:s2:src/main.rs@40")
	expect(calls).to_contain("open_file:s2:/etc/hosts.conf")
end)

test("kinds: a resolved file link has its trailing glyph; a paragraph of one file link, or a bare file name, is a file row", function()
	local app = kinds()
	-- (Each glyph's room is zgx's placeholder joiner, two a glyph and no text of its own: parse.rs, main.rs and the link outside the folder have one, the web's none.)
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

test("kinds: find in chat finds words in every new row kind, and marks each where it's drawn", function()
	local app = kinds()
	for _, q in { "Reviewer → Codex", "awaiting your", "tailscale auth", "secret saved", "total duration", "verbose", "feat/kinds", "/doctor", "/status", "changes this chat's model" } do
		expect(find(app, q)).to_be("1 of 1")
		-- (What it counts, it marks: the current match's words washed, in view.)
		expect(app:count(".find-current")).to_be(1)
		expect(string.lower(app:text(".find-current")):find(string.lower(q), 1, true) ~= nil).to_be(true)
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
	-- (Held at its end, the reply's last line stays where it was read: the steer and the reply going on stand above it.)
	local tail = y_of(app, "Here is a longer answer")
	expect(tail ~= nil).to_be(true)
	app:command("kinds_steer")
	for _ = 1, 8 do
		app:frames(2)
		expect(y_of(app, "Going on with the tests")).to_be(tail)
	end
	settle(app)
	expect(y_of(app, "Going on with the tests")).to_be(tail)
	expect(y_of(app, "Also check the tests") ~= nil).to_be(true)
	expect(app:has('role:button "Jump to latest"')).to_be(false)
end)

test("kinds: a steer while reading further up moves nothing in view", function()
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
	app:scroll("#transcript", -600)
	settle(app)
	local n, before = first_shown(app)
	app:command("kinds_steer")
	for _ = 1, 8 do
		app:frames(2)
		expect(y_of(app, `Reply {n} says`)).to_be(before)
	end
	settle(app)
	expect(y_of(app, `Reply {n} says`)).to_be(before)
end)

test("kinds: reading place holds while a picture above the view loads", function()
	local app = long_middle()
	local n, before = first_shown(app)
	app:set_state("kinds_at", n - 1)
	app:command("kinds_image_above")
	settle(app)
	expect(y_of(app, `Reply {n} says`)).to_be(before)
	local loading = app:tree("#transcript"):match('button "late, loading" [^\n]* %d+x(%d+)')
	expect(loading ~= nil).to_be(true)
	app:command("kinds_image_lands")
	settle(app)
	-- (It came in a different shape: the row above changed height.)
	local landed = app:tree("#transcript"):match('button "Open late" [^\n]* %d+x(%d+)')
	expect(landed ~= nil and landed ~= loading).to_be(true)
	expect(y_of(app, `Reply {n} says`)).to_be(before)
end)

test("kinds: reading place holds while a tool group above the view opens and folds", function()
	local app = long_middle()
	local n, _ = first_shown(app)
	app:set_state("kinds_at", n - 1)
	app:command("kinds_tools_above")
	settle(app)
	local before = y_of(app, `Reply {n} says`)
	expect(before ~= nil).to_be(true)
	app:set_state("ds/activity.luau::activity_groups_chosen", { [`la{n - 1}:g`] = true })
	settle(app)
	expect(y_of(app, "file3.rs") ~= nil).to_be(true)
	expect(y_of(app, `Reply {n} says`)).to_be(before)
	app:set_state("ds/activity.luau::activity_groups_chosen", { [`la{n - 1}:g`] = false })
	settle(app)
	expect(y_of(app, "file3.rs")).to_be(nil)
	expect(y_of(app, `Reply {n} says`)).to_be(before)
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
	-- (Past the reply's own last stops: the errors' Log in and Copy error, then its Copy.)
	for _ = 1, 6 do
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
	-- Each chip copies its own words, whole.
	expect(app:count('.transcript-error >> role:button "Copy error"')).to_be(2)
	app:click('.transcript-error >> role:button "Copy error"[1]')
	app:frames(3)
	expect(app:clipboard()).to_contain("Codex exited with status 101")
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

test("kinds: a tool's file outside the folder opens read only by its whole path, as a skill's SKILL.md does", function()
	local app = kinds()
	app:command("kinds_outside")
	app:frames(30)
	app:click('role:button ~"Read 2 files · outside"')
	app:frames(30)
	app:click('.tool-target >> "review"')
	app:frames(30)
	app:click('.tool-target >> "hosts.conf"')
	app:frames(30)
	expect(app:state("calls")).to_contain("open_file:s2:/etc/hosts.conf")
	expect(app:state("calls")).to_contain("SKILL.md")
end)

test("kinds: a file named with no `.` is a link that opens, inside the folder or outside it", function()
	local app = kinds()
	app:command("kinds_outside")
	app:frames(30)
	local words = app:text('role:text ~"Edit Makefile"')
	-- (Two glyphs, two joiners each: Makefile's and LICENSE's outside.)
	local _, joiners = string.gsub(words, "\u{2060}", "")
	expect(joiners).to_be(4)
	app:click('role:link "Makefile"')
	app:click('role:link "LICENSE"')
	app:frames(5)
	expect(app:state("calls")).to_contain("open_file:s2:Makefile@3")
	expect(app:state("calls")).to_contain("LICENSE")
end)

test("kinds: on a host without the secret action the card says it can't save, and its buttons stay", function()
	local app = kinds()
	app:command("kinds_old_host")
	app:fill("#secret-card-ka1:x1 >> role:textbox", "tskey-123")
	app:click("#secret-save-ka1:x1")
	app:frames(10)
	expect(app:error()).to_be(nil)
	expect(app:has("#secret-card-ka1:x1")).to_be(true)
	expect(app:tree("#secret-card-ka1:x1"):find("can't save the secret here", 1, true) ~= nil).to_be(true)
	expect(app:tree("#secret-card-ka1:x1"):match('[^\n]*"Save securely"[^\n]*'):find("disabled", 1, true)).to_be(nil)
	expect(app:tree("#secret-card-ka1:x1"):match('[^\n]*"Decline"[^\n]*'):find("disabled", 1, true)).to_be(nil)
end)

test("kinds: a secret whose answer goes nowhere isn't said to be saved", function()
	local app = kinds()
	app:command("kinds_no_answer")
	app:fill("#secret-card-ka1:x1 >> role:textbox", "tskey-123")
	app:key("Enter")
	app:frames(10)
	expect(app:has("#secret-card-ka1:x1")).to_be(true)
	expect(app:tree("#secret-card-ka1:x1"):find("can't save the secret here", 1, true) ~= nil).to_be(true)
	-- (The one saved before is the only "Secret saved".)
	local _, saved = string.gsub(app:tree("#transcript"), "Secret saved", "")
	expect(saved).to_be(1)
end)

test("kinds: the secret field has no reveal, and a card settled elsewhere drops what was typed", function()
	local app = kinds()
	expect(app:has('#secret-card-ka1:x1 >> role:button "Show password"')).to_be(false)
	app:fill("#secret-card-ka1:x1 >> role:textbox", "tskey-typed")
	app:command("kinds_secret_timed_out")
	app:frames(10)
	expect(app:has("#secret-card-ka1:x1")).to_be(false)
	expect(app:tree("#transcript"):find("Timed out", 1, true) ~= nil).to_be(true)
	app:command("kinds_secret_timed_out")
	app:frames(10)
	expect(app:text("#secret-card-ka1:x1 >> role:textbox")).to_be("")
end)

test("kinds: a sent secret stays sent as its row is made again", function()
	local app = kinds()
	app:fill("#secret-card-ka1:x1 >> role:textbox", "tskey-123")
	app:key("Enter")
	app:frames(10)
	expect(app:has("#secret-card-ka1:x1")).to_be(false)
	app:command("kinds_rows_away")
	app:frames(10)
	expect(app:has('"Secret saved"')).to_be(false)
	app:command("kinds_rows_back")
	app:frames(30)
	expect(app:has("#secret-card-ka1:x1")).to_be(false)
	expect(app:tree("#transcript"):find("Secret saved", 1, true) ~= nil).to_be(true)
end)

test("kinds: a one-ink mark on a thread chip takes the text's ink; a coloured one keeps its own", function()
	local Mark = require("@ds/mark")
	expect(Mark.chip_icon("openai", 14, 3).tint).to_be("currentColor")
	expect(Mark.chip_icon("grok", 14, 3).tint).to_be("currentColor")
	expect(Mark.chip_icon("claude", 14, 3).tint).to_be(nil)
	expect(Mark.chip_icon("claude", 14, 3).src).to_be("assets/icons/claude-mark.svg")
	expect(Mark.chip_icon(nil, 14, 3)).to_be(nil)
	expect(Mark.chip_icon("acp.gemini", 14, 3)).to_be(nil)
end)

test("kinds: Tab reaches every message's strip in a long chat of plain prompts and replies", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	app:command("go_s2")
	app:frames(20)
	app:command("kinds_long")
	app:frames(40)
	settle(app)
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
	-- (Back from the composer, past its own buttons: the last reply's lane, then the last prompt's, each showing its strip.)
	for _ = 1, 4 do
		app:key("shift+Tab")
		app:frames(4)
		if focused() ~= "" then
			break
		end
	end
	expect(focused():find('"Message actions"', 1, true) ~= nil).to_be(true)
	expect(app:count('role:button "Copy message"')).to_be(1)
	app:key("shift+Tab")
	app:frames(4)
	expect(focused():find('"Message actions"', 1, true) ~= nil).to_be(true)
	expect(app:count('role:button "Copy message"')).to_be(1)
	app:key("Tab")
	app:frames(4)
	expect(focused():find('"Copy message"', 1, true) ~= nil).to_be(true)
	app:key("Enter")
	app:frames(3)
	expect(app:clipboard()).to_be("Prompt 40")
	-- (Focus gone from the transcript, the strip goes with it.)
	app:click("#chat-composer >> role:textbox")
	app:frames(5)
	expect(app:has('role:button "Copy message"')).to_be(false)
end)

test("kinds: a split's pane saves a secret, signs in, picks a model and opens links as the chat page does", function()
	local app = mount("shell_fixture.luau")
	app:resize(1500, 1000)
	app:frames(30)
	local reply = "See [parse.rs](src/parse.rs:12) here."
	app:set_state("fixture.chats", {
		a2 = {
			session = { id = "a2", cwd = "/work/atlas", model = "sonnet", state = "ready" },
			messages = {
				{ id = "pu1", role = "user", text = "Set it up", at = "Sep 8, 1:50 PM", copy = "Set it up" },
				{
					id = "pa1",
					role = "assistant",
					text = reply,
					model = "Opus",
					at = "Sep 8, 1:51 PM",
					copy = reply,
					blocks = {
						{ id = "t", kind = "text", text = reply },
						{ id = "x", kind = "line", line = "secret", request = "req-9", title = "Deploy key", reason = "For the box.", status = "pending" },
						{ id = "e", kind = "line", line = "error", text = "Claude Code is not logged in. Please run /login.", problem = "signed_out" },
					},
				},
				{ id = "pm1", role = "system", line = "claude_command", text = "Claude Code /model", card = { command = "/model", text = "", pending = false, model = "opus", models = { { id = "sonnet", label = "Sonnet" }, { id = "haiku", label = "Haiku" } } } },
			},
		},
	})
	app:set_state("inset_split_trees", { { kind = "row", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(60)
	local pane = "#lane-split-a2"
	expect(app:has(`{pane} >> #secret-card-pa1:x`)).to_be(true)
	-- (The pane's file link has its glyph: two joiners in the node's text.)
	local _, joiners = string.gsub(app:text(`{pane} >> role:text ~"See parse.rs"`), "\u{2060}", "")
	expect(joiners).to_be(2)
	app:fill(`{pane} >> #secret-card-pa1:x >> role:textbox`, "tskey-9")
	app:click(`{pane} >> #secret-save-pa1:x`)
	app:frames(10)
	expect(app:state("calls")).to_contain("secret:a2:req-9:tskey-9")
	expect(app:has(`{pane} >> #secret-card-pa1:x`)).to_be(false)
	app:click(`{pane} >> role:button "Haiku"`)
	app:frames(10)
	expect(app:state("calls")).to_contain("configure:a2:model=haiku")
	app:click(`{pane} >> role:button "Log in"`)
	app:frames(20)
	expect(app:state("shell_route_path")).to_contain("providers")
end)
