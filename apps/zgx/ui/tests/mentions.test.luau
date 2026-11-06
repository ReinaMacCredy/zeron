-- The composer's mentions (ds/mention_tokens, the words' side of
-- ds/composer_mentions): the token the caret is in, and what a pick puts
-- in its place. Run with `zgx test ui`.

local mentions = require("@ds/mention_tokens")

local FILES: { mentions.Item } = {
	{ value = "crates/ui/src/queue.rs", label = "queue.rs", file = true },
	{ value = "crates/engine/tests/message_queue.rs", label = "message_queue.rs", file = true },
	{ value = "apps/zgx/ui/ds/init.luau", label = "init.luau", file = true },
	{ value = "apps/zgx/ui/screens/onboarding/init.luau", label = "init.luau", file = true },
}
local COMMANDS: { mentions.Item } = {
	{ value = "commit", label = "/commit" },
	{ value = "compact", label = "/compact" },
}
local SKILLS: { mentions.Item } = {
	{ value = "code-review", label = "Code review" },
	{ value = "release", label = "Release" },
}
local SOURCES: mentions.Sources = { ["@"] = FILES, ["/"] = COMMANDS, ["$"] = SKILLS }
local MESSAGE_QUEUE = FILES[2]

-- A pick with the caret at `caret` (bytes from 0) in `text`: the text
-- after it.
local function picked(text: string, caret: number, item: mentions.Item): string
	local from, to, replacement = mentions.pick(text, caret, item, SOURCES)
	assert(from and to and replacement, "no token at the caret")
	return string.sub(text, 1, from) .. replacement .. string.sub(text, to + 1)
end

test("the token runs from its trigger to its end, past the caret", function()
	local text = "Compare @queue.rs with"
	-- The caret after "@que": the query is what was typed, the token all of it.
	local t = mentions.token(text, 12)
	expect(t).to_be_truthy()
	assert(t, "no token at the caret")
	expect(t.trigger).to_be("@")
	expect(t.query).to_be("que")
	expect(t.from).to_be(8)
	expect(t.to).to_be(17)
end)

test("a pick replaces the whole token, caret at its end, middle or start", function()
	local text = "Compare @queue.rs with"
	expect(picked(text, 17, MESSAGE_QUEUE)).to_be("Compare @message_queue.rs with")
	expect(picked(text, 12, MESSAGE_QUEUE)).to_be("Compare @message_queue.rs with")
	expect(picked(text, 9, MESSAGE_QUEUE)).to_be("Compare @message_queue.rs with")
end)

test("a pick adds one space, never a second", function()
	expect(picked("Read @que", 9, MESSAGE_QUEUE)).to_be("Read @message_queue.rs ")
	expect(picked("Read @que then", 9, MESSAGE_QUEUE)).to_be("Read @message_queue.rs then")
end)

test("two tokens side by side: only the one the caret is in changes", function()
	local text = "@queue.rs @init.luau"
	expect(picked(text, 4, MESSAGE_QUEUE)).to_be("@message_queue.rs @init.luau")
	-- The last token, at the draft's end: a space follows the pick.
	expect(picked(text, 14, FILES[1])).to_be("@queue.rs @queue.rs ")
end)

test("commands and skills replace to their end too", function()
	expect(picked("then $code-review it", 9, SKILLS[2])).to_be("then $release it")
	expect(picked("and /commit now", 8, COMMANDS[2])).to_be("and /compact now")
	-- A command's run stops at a slash: `/co` inside a path is no command.
	expect(mentions.token("see /usr/bin", 12)).to_be_nil()
	expect(mentions.token("costs $12", 7)).to_be_nil()
end)

test("a file is named by the shortest end of its path that is its own", function()
	expect(mentions.short(FILES, FILES[1])).to_be("queue.rs")
	expect(mentions.short(FILES, FILES[2])).to_be("message_queue.rs")
	expect(mentions.short(FILES, FILES[3])).to_be("ds/init.luau")
	expect(mentions.short(FILES, FILES[4])).to_be("onboarding/init.luau")
end)

test("the draft's highlight styles a token in type alone", function()
	local spans = mentions.highlight("Read @queue.rs and /commit", 0)
	local glyphs, names = 0, 0
	-- A span is `{ from, to, color }` with its faces as fields too.
	for _, span in spans do
		local s = span :: { [number]: any, weight: number?, bg: string? }
		if s[3] == "$zeron.composer.token.trigger" then
			glyphs += 1
		elseif s.weight == 500 then
			names += 1
			expect(s.bg).to_be_nil()
		end
	end
	expect(glyphs).to_be(2)
	expect(names).to_be(2)
end)

test("a `$` that is money, a variable or a path opens nothing", function()
	expect(mentions.token("$HOME", 5)).to_be_nil()
	expect(mentions.token("$12", 3)).to_be_nil()
	expect(mentions.token("$a/b", 4)).to_be_nil()
	expect(mentions.token("$code-review", 12)).to_be_truthy()
end)

test("a skill's name takes its own ink, a command's the accent", function()
	local function ink(line: string): string?
		for _, span in mentions.highlight(line, 0) do
			local s = span :: { [number]: any, weight: number? }
			if s.weight == 500 then
				return s[3]
			end
		end
		return nil
	end
	expect(ink("$code-review")).to_be("$zeron.composer.token.skill")
	expect(ink("/commit")).to_be("$zeron.composer.token.action")
end)

test("a staged attachment matches ignoring case and spaces, and lands as plain words", function()
	local staged: { mentions.Item } = {
		{ value = "/tmp/a.png", label = "Screen Shot.png", src = "/tmp/a.png" },
		{ value = "/tmp/b.txt", label = "notes.txt", file = true },
	}
	local rows = mentions.attachments(staged, "screensh")
	expect(#rows).to_be(1)
	expect(rows[1].label).to_be("Screen Shot.png")
	expect(picked("see @screensh", 13, rows[1])).to_be("see Screen Shot.png ")
end)

test("what a search found stays as it found it", function()
	local fuzzy: { mentions.Item } = { { value = "src/queue.rs", label = "queue.rs", searched = true } }
	expect(#mentions.filter(fuzzy, "qrs")).to_be(1)
	expect(#mentions.filter(COMMANDS, "qrs")).to_be(0)
end)

-- The menu itself, on a plain field (completions_fixture.luau).

local PICTURE = "assets/icons/antigravity-mark.png"

local function menu_app(): any
	local app = mount("completions_fixture.luau")
	app:resize(900, 700)
	app:frames(5)
	return app
end

-- Where a node is: x, y, width and height, as the tree prints them.
local function box(app: any, target: string): (number, number, number, number)
	local x, y, w, h = string.match(app:tree(target), "(%-?[%d%.]+),(%-?[%d%.]+) (%d[%d%.]*)x(%d[%d%.]*)")
	assert(x and y and w and h, `no box for {target}`)
	return tonumber(x) :: number, tonumber(y) :: number, tonumber(w) :: number, tonumber(h) :: number
end

test("the mention menu sits over the pill, as wide, and never reaches down over the draft", function()
	local app = menu_app()
	app:type("#field", "first line\nsecond line /")
	app:frames(3)
	expect(app:has('role:option "/review"')).to_be(true)
	local px, py, pw = box(app, "#pill")
	local cx, cy, cw, ch = box(app, "#mention-popover")
	expect(cx).to_be(px)
	expect(cw).to_be(pw)
	expect(math.abs(cy + ch - (py - 6)) <= 1).to_be(true)
	-- A window with little room over the pill: the card is cut short, not moved.
	app:resize(900, 200)
	app:key("Backspace")
	app:type("#field", "/")
	app:frames(3)
	local _, py2 = box(app, "#pill")
	local _, cy2, _, ch2 = box(app, "#mention-popover")
	expect(cy2 + ch2 <= py2 - 5).to_be(true)
	expect(cy2 >= 0).to_be(true)
end)

test("the mention menu: Escape holds while the caret moves in the token, an edit opens it again", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(true)
	app:key("Escape")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(false)
	app:key("ArrowLeft")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(false)
	app:key("Backspace")
	app:frames(2)
	expect(app:has('role:option')).to_be(true)
end)

test("the mention menu: a press outside closes it and still acts, a press on the draft closes it", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	app:click("#other")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(false)
	expect(app:state("calls")).to_contain("other")
	-- Back in the draft at the token's end: the dismissal holds until an edit.
	app:click("#field")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(false)
	app:fill("#field", "/revi")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(true)
	app:click("#field")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(false)
end)

test("the mention menu: a selection closes it with no memory", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	app:key("shift+ArrowLeft")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(false)
	app:key("ArrowRight")
	app:frames(2)
	expect(app:has('role:option "/review"')).to_be(true)
end)

test("the mention menu: a row pressed is picked and the keys stay in the draft", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	app:click('role:option "/review"')
	app:frames(2)
	expect(app:text("#field")).to_be("/review ")
	expect(app:has('role:option "/review"')).to_be(false)
	app:key("ArrowLeft")
	app:key("ArrowLeft")
	app:frames(2)
	-- The caret back in what was picked: closed, it's a reference now.
	expect(app:has('role:option "/review"')).to_be(false)
	-- (Two steps back from past the space: the keys never left the draft.)
	local a = app:selection("#field")
	expect(a).to_be(6)
end)

test("the mention menu: a large catalog builds only the rows in view, and the keys reach the rest", function()
	local app = menu_app()
	local rows = {}
	for i = 1, 1200 do
		table.insert(rows, { kind = "command", name = `cmd{i}`, label = `/cmd{i}`, detail = "A command", link = `/cmd{i}` })
	end
	app:set_state("rows", rows)
	app:type("#field", "/")
	app:frames(3)
	expect(app:count("role:option") <= 16).to_be(true)
	local _, _, _, h = box(app, "#mention-popover")
	expect(h <= 320).to_be(true)
	app:key("ArrowUp")
	app:frames(3)
	-- Wrapped to the last, built on the way (Zeron's own end the list).
	expect(app:has('role:option "/stop"')).to_be(true)
	app:fill("#field", "/cmd119")
	app:frames(3)
	expect(app:has('role:option "/cmd1190"')).to_be(true)
	expect(app:count("role:option")).to_be(11)
end)

test("the mention menu: a heading goes over its row, uppercase and spaced", function()
	local app = menu_app()
	app:set_state("extra", { { value = "t1", label = "A thread", heading = "Threads" } })
	app:type("#field", "@")
	app:frames(3)
	expect(app:has(`~"T\u{200A}H\u{200A}R\u{200A}E\u{200A}A\u{200A}D\u{200A}S"`)).to_be(true)
	expect(app:has('role:option "A thread"')).to_be(true)
end)

test("the mention menu: `@` pins staged attachments first, and one picked lands as words", function()
	local app = menu_app()
	app:set_state("attachments", {
		{ value = "assets/shot.png", label = "shot.png", src = PICTURE },
		{ value = "/p/notes.txt", label = "notes.txt", file = true },
	})
	app:type("#field", "@")
	app:wait('role:option "queue.rs"', { timeout_ms = 1000 })
	expect(app:has('~"Attached image"')).to_be(true)
	expect(app:has('~"Attached file"')).to_be(true)
	local _, shot_y = box(app, 'role:option "shot.png"')
	local _, file_y = box(app, 'role:option "queue.rs"')
	expect(shot_y < file_y).to_be(true)
	app:key("Enter")
	app:frames(3)
	expect(app:text("#field")).to_be("shot.png ")
end)

test("the mention menu: a search that fails says why, retrying once when offline", function()
	local app = menu_app()
	app:set_state("search_script", { "offline", "offline" })
	app:type("#field", "@que")
	expect(app:wait(`~"The session's device is unreachable"`, { timeout_ms = 2000 })).to_be(true)
	local _, n = string.gsub(app:state("calls"), "search:que", "")
	expect(n).to_be(2)
	app:fill("#field", "")
	app:frames(2)
	app:set_state("search_script", { "Update Zeron to use this here." })
	app:type("#field", "@old")
	expect(app:wait('~"older zeron"', { timeout_ms = 2000 })).to_be(true)
	app:fill("#field", "")
	app:frames(2)
	app:set_state("search_script", { "boom" })
	app:type("#field", "@bad")
	expect(app:wait('~"File search failed"', { timeout_ms = 2000 })).to_be(true)
	-- A staged attachment that matches answers instead.
	app:set_state("attachments", { { value = "assets/bad.png", label = "bad.png", src = PICTURE } })
	app:frames(2)
	expect(app:has('~"File search failed"')).to_be(false)
	expect(app:has('role:option "bad.png"')).to_be(true)
end)

test("the mention menu: a burst of typing searches once, and a first search shows the skeleton alone", function()
	local app = menu_app()
	app:set_state("search_script", { "hold" })
	app:type("#field", "@a")
	app:type("#field", "b")
	expect(app:has("#mention-skeleton")).to_be(true)
	expect(app:wait('~"search:ab"', { timeout_ms = 300 })).to_be(false)
	expect(app:state("calls")).to_contain("search:ab;")
	expect(app:state("calls")).never.to_contain("search:a;")
	expect(app:has("#mention-empty")).to_be(false)
end)

test("the mention menu: `@` searches for the token the caret is in, not the draft's end", function()
	local app = menu_app()
	app:fill("#field", "see @que now")
	for _ = 1, 4 do
		app:key("ArrowLeft")
	end
	expect(app:wait('role:option "queue.rs"', { timeout_ms = 1000 })).to_be(true)
	expect(app:state("calls")).to_contain("search:que;")
end)

test("the mention menu: an empty `@` says so by what was typed", function()
	local app = menu_app()
	app:set_state("found", {})
	app:type("#field", "@")
	expect(app:wait('~"No files available"', { timeout_ms = 1000 })).to_be(true)
	app:type("#field", "zz")
	expect(app:wait('~"No matching files, threads or tasks"', { timeout_ms = 1000 })).to_be(true)
end)
