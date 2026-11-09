-- Text annotations, as Zeron's layer has them (crates/ui/src/annotations.rs),
-- offered from upstream zgx's glass pill under the selection's end (Mention,
-- Add to chat, Ask in side chat; a scroll or Escape puts it away): Mention
-- quoting into the draft at the caret, Add to chat opening a comment input on its first line,
-- the comment kept on Enter or a click outside and the annotation dropped on
-- Escape, a numbered badge on the annotated line, the composer's one chip and
-- its card, and the prompt carrying Zeron's annotation block. The selection
-- is the fixture's stand-in for the host's (`fake_selection`).

local function settle(app: any)
	app:frames(30)
end

local function open(app: any, id: string)
	app:command(`go_{id}`)
	settle(app)
end

local function box_of(app: any, sel: string, pattern: string): { x: number, y: number, w: number, h: number }?
	for line in app:tree(sel):gmatch("[^\n]+") do
		if line:find(pattern, 1, true) then
			local x, y, w, h = line:match(" (%-?%d+),(%-?%d+) (%d+)x(%d+)")
			if x then
				return { x = tonumber(x) :: number, y = tonumber(y) :: number, w = tonumber(w) :: number, h = tonumber(h) :: number }
			end
		end
	end
	return nil
end

-- `words` selected where `shown` is drawn (its first line), and the press let go there.
local function select(app: any, shown: string, words: string, w: number?)
	local b = box_of(app, "#transcript", shown) :: any
	expect(b ~= nil).to_be(true)
	local line = { x = b.x + 10, y = b.y, w = w or math.min(b.w - 10, 160), h = 20 }
	app:set_state("fake_selection", { text = words, top = line, bottom = line })
	app:click(`#transcript >> ~"{shown}"`)
	settle(app)
end

local function annotate(app: any, shown: string, words: string, comment: string)
	select(app, shown, words)
	app:click("#annotation-add-to-chat")
	settle(app)
	if comment ~= "" then
		app:type("#annotation-comment", comment)
	end
	app:key("Enter")
	settle(app)
end

-- The composer chip's card, shown as the pointer rests on the chip (a tooltip's real time).
local function card(app: any)
	app:hover("#composer-annotations")
	app:wait('"never shown, only waited past"', { timeout_ms = 200 })
	settle(app)
end

-- The pill's buttons, in order.
local function words_of(app: any): { string }
	local out: { string } = {}
	for line in app:tree("#annotation-pill"):gmatch("[^\n]+") do
		local name = line:match(' button "([^"]+)"')
		if name then
			table.insert(out, name)
		end
	end
	return out
end

local function send(app: any, words: string)
	app:type('role:textbox "Message"', words)
	app:key("Enter")
	app:wait('"never shown, only waited past"', { timeout_ms = 600 })
	settle(app)
end

test("annotations: the pill's three words stand under the selection's end; Add to chat takes a comment, and the prompt carries both in Zeron's block", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	select(app, "Hi! What shall we work on?", "What shall we work on?", 420)
	-- Upstream's pill, with ours: Mention, Add to chat, Ask in side chat; no Attach.
	expect(table.concat(words_of(app), "|")).to_be("Mention|Add to chat|Ask in side chat")
	expect(app:has('#annotation-pill >> role:button "Attach"')).to_be(false)
	-- Just under the selection's last line (6 clear), ending where it ends.
	local line = box_of(app, "#transcript", "Hi! What shall we work on?") :: any
	local pill = box_of(app, "#annotation-pill", "#annotation-pill") :: any
	local sel = app:state("fake_selection").bottom
	expect(pill.y).to_be(sel.y + sel.h + 6)
	expect(pill.x + pill.w).to_be(sel.x + sel.w)
	expect(pill.h).to_be(32)
	app:click("#annotation-add-to-chat")
	settle(app)
	-- The input, focused, in the pill's place; the selection gone.
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:tree("#annotation-comment")).to_contain(" focused")
	expect(app:state("fake_selection")).to_be(nil)
	expect(app:has('#chat-composer >> "1 annotation"')).to_be(true)
	app:type("#annotation-comment", "  Why this?  ")
	app:key("Enter")
	settle(app)
	-- Kept, trimmed; the composer takes the keys; the badge marks the line.
	expect(app:has("#annotation-editor")).to_be(false)
	expect(app:tree('role:textbox "Message"')).to_contain(" focused")
	local badge = box_of(app, "#transcript", "#annotation-badge-1") :: any
	expect(badge ~= nil).to_be(true)
	expect(math.abs(badge.y - (line.y - 28.5)) <= 1).to_be(true)
	expect(app:text("#annotation-badge-1")).to_be("1")
	send(app, "Look")
	expect(app:state("calls")).to_contain(
		"send:s2:Look\n\nAnnotations (numbered; each quotes the text the user selected, then the user's comment on it):\n1. Selected text:\n> What shall we work on?\nUser comment:\n> Why this?"
	)
	-- Sent: off the composer and the transcript; the bubble wears the badge.
	expect(app:has('#chat-composer >> "1 annotation"')).to_be(false)
	expect(app:has("#annotation-badge-1")).to_be(false)
	expect(app:has('#transcript >> .message-badge "1 annotation"')).to_be(true)
	expect(app:has('#transcript >> ~"Annotations (numbered"')).to_be(false)
end)

test("annotations: typing in the comment never reaches the composer, and Escape takes the new one away first", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	app:click("#annotation-add-to-chat")
	settle(app)
	app:type("#annotation-comment", "not for the draft")
	expect(app:text('role:textbox "Message"')).to_be("")
	app:key("Escape")
	settle(app)
	expect(app:has("#annotation-editor")).to_be(false)
	expect(app:has('#chat-composer >> "1 annotation"')).to_be(false)
	expect(app:has("#annotation-badge-1")).to_be(false)
	expect(app:text('role:textbox "Message"')).to_be("")
	expect(app:state("calls")).never.to_contain("stop:")
	-- No comment at all: still an annotation, sent as Zeron's "(none)".
	annotate(app, "Hi! What shall we work on?", "What shall we work on?", "")
	send(app, "")
	expect(app:state("calls")).to_contain("send:s2:Refer to the selected text below.\n\nAnnotations (numbered; each quotes the text the user selected, then the user's comment on it):\n1. Selected text:\n> What shall we work on?\nUser comment: (none)")
end)

test("annotations: a click outside keeps the comment and leaves the composer be", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	app:click("#annotation-add-to-chat")
	settle(app)
	app:type("#annotation-comment", "kept")
	app:click('#transcript >> ~"Hello there"')
	settle(app)
	expect(app:has("#annotation-editor")).to_be(false)
	expect(app:tree('role:textbox "Message"')).never.to_contain(" focused")
	expect(app:has("#annotation-badge-1")).to_be(true)
	send(app, "Go")
	expect(app:state("calls")).to_contain("User comment:\n> kept")
end)

test("annotations: badges number the chat's annotations on their own lines, through a reload, a switch and a removal", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	-- (A plain selection staged first: annotations count among themselves.)
	app:command("stage_selection")
	annotate(app, "Hi! What shall we work on?", "What shall we work on?", "second asked")
	annotate(app, "Hello there", "Hello there", "first said")
	local reply = box_of(app, "#transcript", "Hi! What shall we work on?") :: any
	local prompt = box_of(app, "#transcript", "Hello there") :: any
	local function on_line(n: number, line: any): boolean
		local b = box_of(app, "#transcript", `#annotation-badge-{n}`)
		return b ~= nil and math.abs(b.y - (line.y - 28.5)) <= 1
	end
	expect(on_line(1, reply)).to_be(true)
	expect(on_line(2, prompt)).to_be(true)
	-- The page made again (a reload), and another chat shown and back.
	app:command("remake_session")
	settle(app)
	expect(on_line(1, reply)).to_be(true)
	expect(on_line(2, prompt)).to_be(true)
	open(app, "s1")
	expect(app:has("#annotation-badge-1")).to_be(false)
	expect(app:has('#chat-composer >> "2 annotations"')).to_be(false)
	open(app, "s2")
	expect(app:has('#chat-composer >> "2 annotations"')).to_be(true)
	expect(on_line(1, reply)).to_be(true)
	expect(on_line(2, prompt)).to_be(true)
	-- The first taken off its card: the other is 1 now, on its line.
	card(app)
	expect(app:has('#composer-annotations-card >> "What shall we work on?"')).to_be(true)
	expect(app:has('#composer-annotations-card >> "first said"')).to_be(true)
	app:click('role:button "Remove annotation 1"')
	settle(app)
	expect(app:has("#annotation-badge-2")).to_be(false)
	expect(on_line(1, prompt)).to_be(true)
	send(app, "Both")
	local calls = app:state("calls")
	expect(calls).to_contain("1. Selected text:\n> Hello there\nUser comment:\n> first said")
	expect(calls).never.to_contain("second asked")
end)

test("annotations: the card's pencil opens the comment again; Cancel keeps it and shows its tip, Save changes it", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	annotate(app, "Hi! What shall we work on?", "What shall we work on?", "first thought")
	card(app)
	app:click('role:button "Edit annotation 1"')
	settle(app)
	expect(app:has("#annotation-save")).to_be(true)
	expect(app:text("#annotation-comment")).to_be("first thought")
	expect(app:tree("#annotation-comment")).to_contain(" focused")
	app:type("#annotation-comment", " dropped")
	app:click("#annotation-cancel")
	settle(app)
	expect(app:has("#annotation-editor")).to_be(false)
	expect(app:has('#annotation-tip >> "first thought"')).to_be(true)
	card(app)
	app:click('role:button "Edit annotation 1"')
	settle(app)
	app:type("#annotation-comment", ", then more")
	app:click("#annotation-save")
	settle(app)
	send(app, "Edited")
	expect(app:state("calls")).to_contain("User comment:\n> first thought, then more")
end)

test("annotations: a hovered badge shows its comment; the composer chip's × takes every annotation", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	annotate(app, "Hi! What shall we work on?", "What shall we work on?", "why this")
	annotate(app, "Hello there", "Hello there", "")
	app:hover("#annotation-badge-1")
	settle(app)
	expect(app:has('#annotation-tip >> "why this"')).to_be(true)
	-- (No comment, no tip.)
	app:hover("#annotation-badge-2")
	settle(app)
	expect(app:has("#annotation-tip")).to_be(false)
	card(app)
	app:click("#composer-annotations-remove")
	settle(app)
	expect(app:has("#composer-annotations")).to_be(false)
	expect(app:has("#annotation-badge-1")).to_be(false)
	send(app, "Plain")
	expect(app:state("calls")).to_contain("send:s2:Plain")
	expect(app:state("calls")).never.to_contain("Annotations (numbered")
end)

test("annotations: a plain selection staged is Zeron's one selections chip, sent quoted after any annotations", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	annotate(app, "Hi! What shall we work on?", "What shall we work on?", "")
	app:command("stage_selection")
	settle(app)
	expect(app:has('#chat-composer >> "1 selection"')).to_be(true)
	send(app, "Both kinds")
	expect(app:state("calls")).to_contain("User comment: (none)\n\nSelected text from this conversation (quoted):\n> Layout dominates on both machines.")
	expect(app:has('#transcript >> .message-badge "1 selection"')).to_be(true)
end)

test("annotations: a scroll takes the pill away, fading where it stood, and leaves the selection", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 640)
	open(app, "s1")
	select(app, "Part 29 works like this.", "Part 29 works like this.")
	local before = box_of(app, "#annotation-pill", "#annotation-pill") :: any
	app:scroll("#transcript", -60)
	app:frames(2)
	-- (Going: where it stood, not riding its line.)
	local going = box_of(app, "#annotation-pill", "#annotation-pill")
	if going then
		expect(going.y).to_be(before.y)
	end
	settle(app)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:state("fake_selection") ~= nil).to_be(true)
	-- Still there, nothing comes back on its own as the view rests.
	app:frames(30)
	expect(app:has("#annotation-pill")).to_be(false)
end)

test("annotations: the pill rises a little into place as it fades in; with motion reduced it only fades", function()
	for _, reduce in { false, true } do
		local app = mount("chat_fixture.luau")
		app:resize(960, 700)
		app:set_state("motion_reduce", reduce)
		open(app, "s2")
		local b = box_of(app, "#transcript", "Hi! What shall we work on?") :: any
		local line = { x = b.x + 10, y = b.y, w = 160, h = 20 }
		app:set_state("fake_selection", { text = "What shall we work on?", top = line, bottom = line })
		app:click('#transcript >> ~"Hi! What shall we work on?"')
		app:frames(1)
		local first = (box_of(app, "#annotation-pill", "#annotation-pill") :: any).y
		settle(app)
		local rest = (box_of(app, "#annotation-pill", "#annotation-pill") :: any).y
		expect(rest).to_be(line.y + line.h + 6)
		if reduce then
			expect(first).to_be(rest)
		else
			expect(first > rest and first <= rest + 4).to_be(true)
		end
	end
end)

test("annotations: Mention puts the passage in the draft at the caret, sent in full; nothing staged, no badge", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	app:type('role:textbox "Message"', "Look at this: why?")
	-- (The caret before "why?".)
	for _ = 1, 4 do
		app:key("Left")
	end
	local long = "What shall we work on? A passage long enough that its line in the draft is cut short, well past the excerpt's seventy-two characters."
	select(app, "Hi! What shall we work on?", long)
	app:click("#annotation-mention")
	settle(app)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:has("#annotation-editor")).to_be(false)
	expect(app:state("fake_selection")).to_be(nil)
	local draft = app:text('role:textbox "Message"')
	expect(draft).to_contain("Look at this:\n\n> Opus: What shall we work on? A passage long enough")
	expect(draft).to_contain("…\n\nwhy?")
	expect(draft).never.to_contain("seventy-two characters.")
	-- The keys back in the draft; nothing on the composer's strip or the line.
	expect(app:tree('role:textbox "Message"')).to_contain(" focused")
	expect(app:has('#chat-composer >> "1 annotation"')).to_be(false)
	expect(app:has('#chat-composer >> "1 selection"')).to_be(false)
	expect(app:has("#annotation-badge-1")).to_be(false)
	app:key("Enter")
	app:wait('"never shown, only waited past"', { timeout_ms = 600 })
	settle(app)
	expect(app:state("calls")).to_contain(`send:s2:Look at this:\n\n> Opus:\n> {long}\n\nwhy?`)
	expect(app:state("calls")).never.to_contain("Annotations (numbered")
end)

test("annotations: two passages whose lines read the same each go in full, and one mentioned twice goes in full twice", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	local head = "What shall we work on? A passage long enough that its line in the draft is cut short"
	local one, two = `{head}, ending one way.`, `{head}, ending another way.`
	for _, words in { one, two, one } do
		select(app, "Hi! What shall we work on?", words)
		app:click("#annotation-mention")
		settle(app)
	end
	local draft = app:text('role:textbox "Message"')
	expect(draft).to_contain("…\n\n> Opus: What shall we work on?")
	expect(draft).to_contain("… (2)\n\n")
	app:key("Enter")
	app:wait('"never shown, only waited past"', { timeout_ms = 600 })
	settle(app)
	expect(app:state("calls")).to_contain(`send:s2:> Opus:\n> {one}\n\n> Opus:\n> {two}\n\n> Opus:\n> {one}`)
end)

-- The focused node's line, or "" with focus parked.
local function focused(app: any): string
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(" focused", 1, true) then
			return line
		end
	end
	return ""
end

test("annotations: the pill leaves the keys where they were: Tab walks its words, Space presses one, Escape or a scroll puts it away and parks them", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	-- (The selection's press parks the keys; the pill takes none, so typing and a paste go on to the composer.)
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	expect(app:has("#annotation-pill")).to_be(true)
	expect(focused(app)).to_be("")
	app:key("Tab")
	expect(app:tree("#annotation-mention")).to_contain(" focused")
	app:key("Tab")
	expect(app:tree("#annotation-add-to-chat")).to_contain(" focused")
	app:key("Tab")
	expect(app:tree("#annotation-ask-side-chat")).to_contain(" focused")
	-- Escape on a word: the pill goes and the keys are parked, not on the next control.
	app:key("Escape")
	settle(app)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:state("fake_selection") ~= nil).to_be(true)
	expect(focused(app)).to_be("")
	expect(app:state("calls")).never.to_contain("stop:")
	expect(app:text('role:textbox "Message"')).to_be("")
	-- Escape with the keys parked: the window's root hands it to the pill.
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	app:key("Escape")
	settle(app)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:state("fake_selection") ~= nil).to_be(true)
	expect(focused(app)).to_be("")
	-- Shift+Tab reaches its last word; Space presses the word that has the keys.
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	app:key("shift+Tab")
	expect(app:tree("#annotation-ask-side-chat")).to_contain(" focused")
	app:key("shift+Tab")
	app:key("shift+Tab")
	expect(app:tree("#annotation-mention")).to_contain(" focused")
	app:key("Space")
	settle(app)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:text('role:textbox "Message"')).to_contain("> Opus: What shall we work on?")
	expect(app:tree('role:textbox "Message"')).to_contain(" focused")
	-- Taken by the keys: Add to chat opens the input as a click does.
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	app:key("Tab")
	app:key("Tab")
	app:key("Enter")
	settle(app)
	expect(app:tree("#annotation-comment")).to_contain(" focused")
	-- A scroll with a word holding the keys: parked too.
	app:key("Escape")
	settle(app)
	open(app, "s1")
	select(app, "Part 29 works like this.", "Part 29 works like this.")
	app:key("Tab")
	expect(app:tree("#annotation-mention")).to_contain(" focused")
	app:scroll("#transcript", -60)
	settle(app)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(focused(app)).to_be("")
end)

test("annotations: Ask in side chat stages the text on a side chat's draft, with nothing to badge here", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	app:click("#annotation-ask-side-chat")
	settle(app)
	expect(app:has("#annotation-pill")).to_be(false)
	expect(app:has("#annotation-editor")).to_be(false)
	expect(app:has('#chat-composer >> "1 annotation"')).to_be(false)
	expect(app:has("#annotation-badge-1")).to_be(false)
	local staged = nil :: any
	for key, d in app:state("chat.composer.drafts") do
		if key ~= "s2" and d.cargo and d.cargo[1] then
			staged = d.cargo[1]
		end
	end
	expect(staged ~= nil).to_be(true)
	expect(staged.kind).to_be("quote")
	expect(staged.text).to_be("What shall we work on?")
	expect(staged.comment).to_be("")
	expect(staged.anchor).to_be(nil)
end)

test("annotations: with no room over the first line the input stands under it; two on a line step their badges left", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	select(app, "Hello there", "Hello there")
	local line = box_of(app, "#transcript", "Hello there") :: any
	local view = box_of(app, "#transcript", "#transcript") :: any
	-- (The first line sits under the view's top, too near it for the input above.)
	expect(line.y - view.y - 7 - 45 < 8).to_be(true)
	app:click("#annotation-add-to-chat")
	settle(app)
	local input = box_of(app, "#annotation-editor", "#annotation-editor") :: any
	expect(input.y).to_be(line.y + 20 + 7)
	app:key("Enter")
	settle(app)
	annotate(app, "Hello there", "there", "")
	local one = box_of(app, "#transcript", "#annotation-badge-1") :: any
	local two = box_of(app, "#transcript", "#annotation-badge-2") :: any
	expect(two.y).to_be(one.y)
	expect(one.x - two.x).to_be(32)
end)

test("annotations: a chat switched away from keeps the comment being written; the edit card's trash deletes", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	app:click("#annotation-add-to-chat")
	settle(app)
	app:type("#annotation-comment", "half written")
	open(app, "s1")
	open(app, "s2")
	expect(app:has("#annotation-badge-1")).to_be(true)
	card(app)
	expect(app:has('#composer-annotations-card >> "half written"')).to_be(true)
	app:click('role:button "Edit annotation 1"')
	settle(app)
	app:click("#annotation-delete")
	settle(app)
	expect(app:has("#annotation-editor")).to_be(false)
	expect(app:has("#annotation-badge-1")).to_be(false)
	expect(app:has("#composer-annotations")).to_be(false)
end)

test("annotations: a badge scrolls with its line, and the pencil brings a line scrolled away back into view", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 640)
	open(app, "s1")
	annotate(app, "Part 29 works like this.", "Part 29 works like this.", "this part")
	local line = box_of(app, "#transcript", "Part 29 works like this.") :: any
	local badge = box_of(app, "#transcript", "#annotation-badge-1") :: any
	app:scroll("#transcript", -60)
	settle(app)
	local moved = box_of(app, "#transcript", "Part 29 works like this.") :: any
	local followed = box_of(app, "#transcript", "#annotation-badge-1") :: any
	expect(math.abs((followed.y - badge.y) - (moved.y - line.y)) <= 1).to_be(true)
	-- Far away (its row not laid out): the pencil scrolls back to it.
	app:scroll("#transcript", -4000)
	settle(app)
	expect(app:has('#transcript >> ~"Part 29 works like this."')).to_be(false)
	card(app)
	app:click('role:button "Edit annotation 1"')
	app:frames(60)
	expect(app:wait("#annotation-save", { timeout_ms = 2000 })).to_be(true)
	expect(app:has('#transcript >> ~"Part 29 works like this."')).to_be(true)
	expect(app:text("#annotation-comment")).to_be("this part")
end)
