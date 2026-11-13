-- The prompt rail at the conversation's left edge (ds/message_rail, gpui's
-- rail.rs), on chat_fixture's s1: thirty prompts, each with its reply.

local function box(app: any, target: string): (number?, number?, number?, number?)
	for line in app:tree(target):gmatch("[^\n]+") do
		local x, y, w, h = line:match(" (%-?%d+),(%-?%d+) (%d+)x(%d+)")
		if x then
			return tonumber(x) :: number, tonumber(y) :: number, tonumber(w) :: number, tonumber(h) :: number
		end
	end
	return nil, nil, nil, nil
end

-- The top of the text holding `words` in the transcript, if it shows there.
local function on_screen(app: any, words: string): number?
	local _, top, _, h = box(app, "#transcript")
	assert(top and h, "no transcript")
	for line in app:tree("#transcript"):gmatch("[^\n]+") do
		local said, y = line:match('text "(.-)" %-?%d+,(%-?%d+) %d+x%d+')
		local at = tonumber(y)
		if said and at and string.find(said, words, 1, true) and at >= top and at < top + h then
			return at
		end
	end
	return nil
end

local function settle(app: any)
	app:wait("#never-shown", { timeout_ms = 250 })
	app:frames(10)
end

local function open(w: number, h: number, command: string?): any
	local app = mount("chat_fixture.luau")
	app:resize(w, h)
	app:frames(20)
	if command then
		app:command(command)
	end
	app:command(if command then "go_s7" else "go_s1")
	app:frames(30)
	settle(app)
	return app
end

local function lit(app: any): (number, number)
	local l = app:state("message_rail.lit")
	return l.first, l.last
end

test("message rail: a rail tick clicked brings its prompt's row on screen", function()
	local app = open(1200, 800)
	expect(on_screen(app, "Question number 5:")).to_be_nil()
	app:click("#message-rail-u5")
	app:frames(45)
	settle(app)
	local _, top, _, h = box(app, "#transcript")
	local at = on_screen(app, "Question number 5:")
	expect(at ~= nil).to_be(true)
	expect((at :: number) < (top :: number) + (h :: number) / 2).to_be(true)
	expect(select(1, lit(app))).to_be(5)
end)

test("message rail: a rail tick clicked glides there rather than jumping", function()
	local app = open(1200, 800)
	app:click("#message-rail-u20")
	app:frames(3)
	expect(on_screen(app, "Question number 20:")).to_be_nil()
	app:frames(40)
	settle(app)
	expect(on_screen(app, "Question number 20:") ~= nil).to_be(true)
end)

test("message rail: the rail's lit tick follows the conversation as it scrolls", function()
	local app = open(1200, 800)
	local first, last = lit(app)
	expect(last).to_be(30)
	for _ = 1, 30 do
		app:wheel("#transcript", { dy = -200 })
		app:frames(2)
	end
	settle(app)
	local up_first, up_last = lit(app)
	expect(up_first < first).to_be(true)
	expect(on_screen(app, `Question number {up_first}:`) ~= nil).to_be(true)
	expect(on_screen(app, `Question number {up_last}:`) ~= nil).to_be(true)
	for _ = 1, 200 do
		app:wheel("#transcript", { dy = -400 })
		app:frames(1)
	end
	settle(app)
	expect(select(1, lit(app))).to_be(1)
end)

test("message rail: the rail is hidden while the conversation is narrower than 768", function()
	local app = open(1200, 800)
	expect(app:has("#message-rail")).to_be(true)
	app:resize(760, 800)
	app:frames(10)
	expect(app:has("#message-rail")).to_be(false)
	app:resize(777, 800)
	app:frames(10)
	expect(app:has("#message-rail")).to_be(false)
	app:resize(778, 800)
	app:frames(10)
	expect(app:has("#message-rail")).to_be(true)
end)

test("message rail: a rail tick under the pointer grows and shows its prompt and the opening of its reply", function()
	local app = open(1200, 800)
	expect(app:has("#message-rail-card")).to_be(false)
	app:hover("#message-rail-u12")
	app:frames(5)
	expect(app:has("#message-rail-card")).to_be(true)
	local card = app:tree("#message-rail-card")
	expect(card).to_contain("Question number 12: how does part 12 work?")
	expect(card).to_contain("Part 12 works like this.")
	local function bar(id: string): number?
		return select(3, box(app, `#message-rail-bar-{id}`))
	end
	expect(bar("u12")).to_be(24)
	expect(bar("u11")).to_be(16)
	expect(bar("u14")).to_be(10)
	expect(bar("u20")).to_be(8)
	app:hover("#transcript")
	app:frames(5)
	expect(app:has("#message-rail-card")).to_be(false)
	expect(bar("u12")).to_be(8)
end)

test("message rail: a long conversation's rail scrolls its ticks to keep the lit ones in view", function()
	local app = open(1200, 800, "long_chat")
	expect(app:has("#message-rail-scroll")).to_be(true)
	local _, sy, _, sh = box(app, "#message-rail-scroll")
	local _, last = lit(app)
	local _, top = box(app, "#message-rail-ticks")
	local ty = (top :: number) + 1 + (last - 1) * 8
	expect(last > 100).to_be(true)
	expect(ty >= (sy :: number) and ty + 8 <= (sy :: number) + (sh :: number)).to_be(true)
	expect((top :: number) < (sy :: number)).to_be(true)
	for _ = 1, 300 do
		app:wheel("#transcript", { dy = -800 })
		app:frames(1)
	end
	settle(app)
	local first = lit(app)
	local _, moved = box(app, "#message-rail-ticks")
	local fy = (moved :: number) + 1 + (first - 1) * 8
	expect(first < last - 20).to_be(true)
	expect((moved :: number) > (top :: number)).to_be(true)
	expect(fy >= (sy :: number) and fy + 8 <= (sy :: number) + (sh :: number)).to_be(true)
end)
