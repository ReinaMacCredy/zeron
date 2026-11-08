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

-- The menu itself, on a plain field (completions_fixture.luau): upstream's
-- popover at the caret, wired as ds/composer wires it.

-- Whether the menu shows `target` (a closed popover keeps its rows, hidden).
local function shown(app: any, target: string): boolean
	if not app:has(target) or not app:has("#mention-popover") then
		return false
	end
	return string.find(string.match(app:tree("#mention-popover"), "^[^\n]*") or "", " hidden", 1, true) == nil
end

local function menu_app(): any
	local app = mount("completions_fixture.luau")
	app:resize(900, 700)
	app:frames(5)
	return app
end

test("the mention menu: Escape holds while the caret moves in the token, an edit opens it again", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(true)
	app:key("Escape")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(false)
	expect(app:text("#field")).to_be("/rev")
	app:key("ArrowLeft")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(false)
	app:key("Backspace")
	app:frames(2)
	expect(shown(app, "role:option")).to_be(true)
end)

test("the mention menu: a row pressed is picked and closes it", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	app:click('role:option "/review"')
	app:frames(2)
	expect(app:text("#field")).to_be("/review ")
	expect(shown(app, 'role:option "/review"')).to_be(false)
	app:key("ArrowLeft")
	app:key("ArrowLeft")
	app:frames(2)
	-- The caret back in what was picked: closed, it's a reference now.
	expect(shown(app, 'role:option "/review"')).to_be(false)
end)

-- Upstream's popover stands at the caret and lets presses through: a press
-- outside it still acts where it landed, and puts it away as Escape does.
test("the mention menu: a press outside closes it and still acts, a press on the draft closes it", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(true)
	app:click("#other")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(false)
	expect(app:state("calls")).to_contain("other")
	-- Back in the draft at the token's end: put away until an edit.
	app:click("#field")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(false)
	app:fill("#field", "/revi")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(true)
	-- A press on the draft itself puts it away too.
	app:click("#field")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(false)
end)

test("the mention menu: a row pressed leaves the keys in the draft, the caret where the pick put it", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	app:click('role:option "/review"')
	app:frames(2)
	expect(app:text("#field")).to_be("/review ")
	expect(app:tree("#field")).to_contain(" focused")
	expect(app:selection("#field")).to_be(8)
	-- Two steps back from past the space: the keys never left the draft.
	app:key("ArrowLeft")
	app:key("ArrowLeft")
	app:frames(2)
	expect(app:selection("#field")).to_be(6)
	expect(app:tree("#field")).to_contain(" focused")
end)

test("the mention menu: a new draft's token where the last one's was opens it", function()
	local app = menu_app()
	app:type("#field", "/rev")
	app:frames(2)
	app:key("Enter")
	app:frames(2)
	expect(app:text("#field")).to_be("/review ")
	app:fill("#field", "")
	app:frames(1)
	app:type("#field", "/rev")
	app:frames(2)
	expect(shown(app, 'role:option "/review"')).to_be(true)
end)

test("the mention menu: a burst of typing searches once", function()
	local app = menu_app()
	app:set_state("search_script", { "hold" })
	app:type("#field", "@a")
	app:type("#field", "b")
	expect(app:wait('~"search:ab"', { timeout_ms = 300 })).to_be(false)
	expect(app:state("calls")).to_contain("search:ab;")
	expect(app:state("calls")).never.to_contain("search:a;")
end)

test("the mention menu: an empty `@` says so by what was typed", function()
	local app = menu_app()
	app:set_state("found", {})
	app:type("#field", "@")
	expect(app:wait('~"No files available"', { timeout_ms = 1000 })).to_be(true)
	app:type("#field", "zz")
	expect(app:wait('~"No matching files"', { timeout_ms = 1000 })).to_be(true)
end)
