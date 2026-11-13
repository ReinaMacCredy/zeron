local function open_b1(app: any)
	app:resize(1000, 760)
	app:command("go_b1")
	app:frames(30)
end

local function wait(app: any, ms: number)
	app:wait("#never-shown", { timeout_ms = ms })
end

local function ids(app: any): string
	local out = {}
	for _, m in app:state("chat.messages") do
		table.insert(out, m.id)
	end
	return table.concat(out, ",")
end

test("branch actions: a person's message offers Edit and Rewind, a reply Fork", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	app:hover('~"Make it friendlier"')
	app:frames(20)
	expect(app:has('role:button "Edit message"')).to_be(true)
	expect(app:has('role:button "Rewind to here"')).to_be(true)
	expect(app:has('role:button "Fork from here"')).to_be(false)
	app:hover('~"Friendlier, with shipping"')
	app:frames(20)
	expect(app:has('role:button "Fork from here"')).to_be(true)
	expect(app:has('role:button "Edit message"')).to_be(false)
end)

test("branch actions: fork calls ForkChat with the reply's entry and opens the new chat", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	app:hover('~"Friendlier, with shipping"')
	app:frames(20)
	app:click('role:button "Fork from here"')
	wait(app, 600)
	expect(app:state("calls")).to_contain("fork_from:b1:ba2")
	expect(app:has('~"Forked from Checkout page copy"')).to_be(true)
	expect(app:has('~"Now a version for the mobile banner"')).to_be(false)
	local s = app:state("chat.session")
	expect(s.id).never.to_be("b1")
	expect(s.title).to_be("Fork: Checkout page copy")
end)

test("branch actions: an edit sent rewinds at that prompt and sends the new words in its place", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	app:hover('~"Make it friendlier"')
	app:frames(20)
	app:click('role:button "Edit message"')
	wait(app, 300)
	expect(app:has("#message-edit")).to_be(true)
	expect(app:text('role:textbox "Your message"')).to_be("Make it friendlier and mention free shipping.")
	app:fill('role:textbox "Your message"', "Make it warmer and mention free shipping.")
	app:key("Enter")
	wait(app, 500)
	expect(app:state("calls")).to_contain("rewind_send:b1:bu2:Make it warmer and mention free shipping.:true")
	expect(ids(app)).to_be("bu1,ba1,bu2")
	expect(app:state("chat.messages")[3].text).to_be("Make it warmer and mention free shipping.")
	expect(app:has('~"Make it warmer and mention free shipping."')).to_be(true)
	expect(app:has("#message-edit")).to_be(false)
end)

test("branch actions: cancel or Escape puts the bubble back as it was", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	for _, how in { "cancel", "escape" } do
		app:hover('~"Make it friendlier"')
		app:frames(20)
		app:click('role:button "Edit message"')
		wait(app, 300)
		app:fill('role:textbox "Your message"', "Something else")
		if how == "cancel" then
			app:click('#message-edit >> role:button "Cancel"')
		else
			app:key("Escape")
		end
		wait(app, 300)
		expect(app:has("#message-edit")).to_be(false)
		expect(app:has('~"Make it friendlier and mention free shipping."')).to_be(true)
		expect(app:has('~"Something else"')).to_be(false)
	end
	expect(app:state("calls") or "").never.to_contain("rewind_send")
end)

test("branch actions: a refused edit keeps the editor open with the engine's words", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	app:command("branch_fails")
	app:hover('~"Make it friendlier"')
	app:frames(20)
	app:click('role:button "Edit message"')
	wait(app, 300)
	app:fill('role:textbox "Your message"', "Warmer")
	app:key("Enter")
	wait(app, 600)
	expect(app:has("#message-edit")).to_be(true)
	expect(app:has('~"No checkpoint for this reply"')).to_be(true)
	expect(ids(app)).to_contain("bu2,ba2,bu3,ba3")
	app:fill('role:textbox "Your message"', "Warmer still")
	app:key("Enter")
	wait(app, 600)
	expect(app:state("calls")).to_contain("rewind_send:b1:bu2:Warmer still:true")
end)

test("branch actions: rewind asks first, then puts the prompt back in the composer", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	app:hover('~"Make it friendlier"')
	app:frames(20)
	app:click('role:button "Rewind to here"')
	wait(app, 300)
	expect(app:has('~"Before “Make it friendlier and mention free shipping.”"')).to_be(true)
	expect(app:has('~"Files on disk stay as they are."')).to_be(true)
	app:click('role:button "Rewind"')
	wait(app, 600)
	expect(app:state("calls")).to_contain("rewind:b1:bu2")
	expect(ids(app)).to_be("bu1,ba1")
	expect(app:text('role:textbox "Message"')).to_be("Make it friendlier and mention free shipping.")
end)

test("branch actions: a refused rewind brings the turns back and says why", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	app:command("branch_fails")
	app:hover('~"Make it friendlier"')
	app:frames(20)
	app:click('role:button "Rewind to here"')
	wait(app, 300)
	app:click('role:button "Rewind"')
	wait(app, 800)
	expect(app:has('~"No checkpoint for this reply"')).to_be(true)
	expect(ids(app)).to_be("bu1,ba1,bu2,ba2,bu3,ba3")
	app:hover('~"Make it friendlier"')
	app:frames(20)
	expect(app:has('role:button "Rewind to here"')).to_be(true)
end)

test("branch actions: a refused action is dimmed, its reason its tooltip, and a click says it", function()
	local app = mount("chat_fixture.luau")
	open_b1(app)
	app:set_state("test_patch", { running = true })
	app:command("patch_open")
	app:frames(10)
	app:hover('~"Friendlier, with shipping"')
	app:frames(20)
	app:hover('role:button "Fork from here"')
	wait(app, 500)
	expect(app:has('~"Wait for the reply to finish."')).to_be(false)
	expect(app:has('~"Stop the running turn before rewinding."')).to_be(false)
	app:hover('~"Make it friendlier"')
	app:frames(20)
	app:hover('role:button "Edit message"')
	wait(app, 500)
	expect(app:has('~"Stop the running turn before rewinding."')).to_be(true)
	app:click('role:button "Edit message"')
	wait(app, 300)
	expect(app:has("#message-edit")).to_be(false)
end)

test("branch actions: an agent that can't branch dims all three, saying so", function()
	local app = mount("chat_fixture.luau")
	app:command("no_branching")
	open_b1(app)
	app:hover('~"Friendlier, with shipping"')
	app:frames(20)
	expect(app:state("calls")).to_contain("branching:claude-code")
	app:hover('role:button "Fork from here"')
	wait(app, 500)
	expect(app:has('~"cannot fork or rewind a session."')).to_be(true)
end)

local function y_of(app: any, words: string): number?
	for line in app:tree("#transcript"):gmatch("[^\n]+") do
		if line:find(words, 1, true) then
			local y = line:match(" %-?%d+,(%-?%d+) %d+x%d+")
			return tonumber(y)
		end
	end
	return nil
end

local function box_of(app: any, id: string): (number?, number?)
	for line in app:tree("#transcript"):gmatch("[^\n]+") do
		if line:find(id, 1, true) then
			local w, h = line:match(" %-?%d+,%-?%d+ (%d+)x(%d+)")
			return tonumber(w), tonumber(h)
		end
	end
	return nil, nil
end

local function edit_and_send(app: any, words: string, own_words: string): (number?, { number })
	app:hover(`~"{own_words}"`)
	app:frames(20)
	local before = y_of(app, "Friendlier, with shipping:")
	app:click('role:button "Edit message"')
	local ys: { number } = {}
	local heights: { number } = {}
	for _ = 1, 16 do
		app:frames(1)
		table.insert(ys, y_of(app, "Friendlier, with shipping:") :: number)
		local _, h = box_of(app, "#message-edit-box")
		table.insert(heights, h or 0)
	end
	expect(heights[#heights] > heights[1]).to_be(true)
	local own = y_of(app, 'textbox "Your message"')
	app:fill('role:textbox "Your message"', words)
	app:key("Enter")
	local owns: { number } = {}
	for _ = 1, 14 do
		app:frames(1)
		table.insert(ys, y_of(app, "Friendlier, with shipping:") :: number)
		table.insert(owns, (y_of(app, 'textbox "Your message"') or y_of(app, `"{words}"`)) :: number - (own :: number))
	end
	return before, ys, owns
end

local REPLY = "Here is the banner, friendlier: Free shipping on every order, and returns are easy for thirty days."

local function stream_reply(app: any, ys: { number })
	local ms = table.clone(app:state("chat.messages"))
	ms[#ms] = table.clone(ms[#ms])
	ms[#ms].waiting = nil
	table.insert(ms, { id = "r1", role = "assistant", text = "", model = "Opus", streaming = true, started = 0, blocks = { { id = "t0", kind = "text", text = "" } } })
	app:set_state("chat.messages", ms)
	app:frames(2)
	table.insert(ys, y_of(app, "Here is a short version:") :: number)
	for k = 8, #REPLY, 8 do
		local part = string.sub(REPLY, 1, k)
		app:set_state("chat.draft", { id = "r1", text = part, part = "t0", part_text = part })
		app:frames(1)
		table.insert(ys, y_of(app, "Here is a short version:") :: number)
	end
end

local function still_through(app: any, h: number)
	app:resize(1000, h)
	app:command("go_b1")
	app:frames(30)
	app:hover('~"Make it friendlier"')
	app:frames(20)
	local before = y_of(app, "Here is a short version:")
	app:click('role:button "Edit message"')
	local ys: { number } = {}
	local heights: { number } = {}
	for _ = 1, 16 do
		app:frames(1)
		table.insert(ys, y_of(app, "Here is a short version:") :: number)
		local _, bh = box_of(app, "#message-edit-box")
		table.insert(heights, bh or 0)
	end
	expect(heights[#heights] > heights[1]).to_be(true)
	app:fill('role:textbox "Your message"', "Make it warmer and mention free shipping.")
	app:key("Enter")
	for _ = 1, 24 do
		app:frames(1)
		table.insert(ys, y_of(app, "Here is a short version:") :: number)
	end
	expect(app:state("calls")).to_contain("rewind_send:b1:bu2:")
	expect(ids(app)).to_be("bu1,ba1,bu2")
	stream_reply(app, ys)
	return before :: number, ys
end

test("branch actions: with the turns in view, nothing above the edited message moves through open, send, collapse, landing and the reply", function()
	local app = mount("chat_fixture.luau")
	local before, ys = still_through(app, 900)
	for _, y in ys do
		expect(y).to_be(before)
	end
end)

test("branch actions: scrolled to the end, nothing above moves after the point stepped off the end, through the collapse, landing and the reply", function()
	local app = mount("chat_fixture.luau")
	local before, ys = still_through(app, 600)
	expect(ys[1] - before <= 1).to_be(true)
	for _, y in ys do
		expect(y).to_be(ys[1])
	end
end)
