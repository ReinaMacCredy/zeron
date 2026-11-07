-- Text annotations, as Zeron's layer has them (crates/ui/src/annotations.rs):
-- a selection's pill, Add to chat opening a comment input on its first line,
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
local function select(app: any, shown: string, words: string)
	local b = box_of(app, "#transcript", shown) :: any
	expect(b ~= nil).to_be(true)
	local line = { x = b.x + 10, y = b.y, w = math.min(b.w - 10, 160), h = 20 }
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

local function send(app: any, words: string)
	app:type('role:textbox "Message"', words)
	app:key("Enter")
	app:wait('"never shown, only waited past"', { timeout_ms = 600 })
	settle(app)
end

test("annotations: the pill's Add to chat takes a comment, and the prompt carries both in Zeron's block", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 700)
	open(app, "s2")
	select(app, "Hi! What shall we work on?", "What shall we work on?")
	expect(app:has('#annotation-pill >> role:button "Add to chat"')).to_be(true)
	expect(app:has('#annotation-pill >> role:button "Ask in side chat"')).to_be(true)
	-- Over the selection's first line, centred on it.
	local line = box_of(app, "#transcript", "Hi! What shall we work on?") :: any
	local pill = box_of(app, "#annotation-pill", "#annotation-pill") :: any
	expect(pill.y + pill.h).to_be(line.y - 8)
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

test("annotations: the pill follows its text as the transcript scrolls", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 640)
	open(app, "s1")
	select(app, "Part 29 works like this.", "Part 29 works like this.")
	local before = box_of(app, "#annotation-pill", "#annotation-pill") :: any
	local line = box_of(app, "#transcript", "Part 29 works like this.") :: any
	app:scroll("#transcript", -60)
	settle(app)
	local after = box_of(app, "#annotation-pill", "#annotation-pill") :: any
	local moved = box_of(app, "#transcript", "Part 29 works like this.") :: any
	expect(moved.y ~= line.y).to_be(true)
	expect(after.y - before.y).to_be(moved.y - line.y)
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
	local pill = box_of(app, "#annotation-pill", "#annotation-pill") :: any
	local view = box_of(app, "#transcript", "#transcript") :: any
	-- (The first line sits under the view's top, too near it for the pill above.)
	expect(line.y - view.y - 8 - 30 < 8).to_be(true)
	expect(pill.y).to_be(line.y + 20 + 8)
	app:click("#annotation-add-to-chat")
	settle(app)
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
