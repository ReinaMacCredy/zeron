local function box(app: any, target: string): { x: number, y: number, w: number, h: number }
	local line = app:tree(target):match("[^\n]+") :: string
	local x, y, w, h = line:match(" (%-?[%d%.]+),(%-?[%d%.]+) ([%d%.]+)x([%d%.]+)")
	return { x = tonumber(x) :: number, y = tonumber(y) :: number, w = tonumber(w) :: number, h = tonumber(h) :: number }
end

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	app:click('#shell-sidebar >> role:button "Fix the build"')
	app:frames(30)
	return app
end

test("a chat carried over a lone chat slides gpui's two split cards in from its edges, tall and 160 wide", function()
	local app = open()
	expect(app:has("#split-card-a1-left")).to_be(false)
	app:set_state("split.dragging", true)
	app:frames(2)
	local page = box(app, "#split-cards-a1")
	expect(box(app, "#split-card-a1-left").x < page.x + 11).to_be(true)
	app:frames(30)
	local left, right = box(app, "#split-card-a1-left"), box(app, "#split-card-a1-right")
	expect(math.abs(left.x - (page.x + 12)) < 1).to_be(true)
	expect(math.abs(right.x + right.w - (page.x + page.w - 12)) < 1).to_be(true)
	expect(math.abs(left.w - 160) < 1).to_be(true)
	expect(math.abs(left.h - (page.h - 36)) < 1).to_be(true)
	app:set_state("split.dragging", false)
	app:frames(2)
	expect(app:has("#split-card-a1-left")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("in a split, the cards come in on the focused pane until the chat is carried over another", function()
	local app = open()
	app:set_state("inset_split_trees", { { kind = "row", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(30)
	app:set_state("split.dragging", true)
	app:frames(30)
	local pane, card = box(app, "#split-cards-a1"), box(app, "#split-card-a1-left")
	expect(math.abs(card.x - (pane.x + 12)) < 1).to_be(true)
	local other, out = box(app, "#split-cards-a2"), box(app, "#split-card-a2-left")
	expect(out.x + out.w <= other.x).to_be(true)
	expect(app:error()).to_be_nil()
end)
