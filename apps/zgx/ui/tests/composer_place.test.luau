-- The composer over a new chat's wallpaper (ui/composer_place_fixture.luau):
-- its chips, the faint backing the footer takes only over art, what gives way
-- first as room runs short, and the card the context ring opens.

local function settle(app: any)
	app:frames(30)
end

-- Settled, and a label's roll (RollText, about 750 ms) run out.
local function rested(app: any)
	app:wait("#never-shown", { timeout_ms = 900 })
	settle(app)
end

local function mounted(): any
	local app = mount("composer_place_fixture.luau")
	app:resize(900, 360)
	settle(app)
	return app
end

-- Whether a node is on screen (the tree marks one that isn't).
local function showing(app: any, sel: string): boolean
	if not app:has(sel) then
		return false
	end
	local first = string.match(app:tree(sel .. "[1]"), "[^\n]+") or ""
	return string.find(first, " hidden", 1, true) == nil
end

-- What a chip says, a line each for what it shows: its text, nothing when folded
-- to its icon.
local function chip_text(app: any, name: string): string
	local out = {}
	for line in app:tree(`role:button "{name}"`):gmatch("[^\n]+") do
		if not line:find(" hidden", 1, true) then
			local text = line:match('text "([^"]+)"') or line:match('#[%w%-]+ "([^"]+)"')
			if text then
				table.insert(out, text)
			end
		end
	end
	return table.concat(out, " ")
end

local function box_of(app: any, sel: string): (number, number)
	local first = string.match(app:tree(sel), "[^\n]+") or ""
	local w, h = string.match(first, " %-?%d+,%-?%d+ (%d+)x(%d+)")
	return tonumber(w) or -1, tonumber(h) or -1
end

local function height(app: any, sel: string): number
	local _, h = box_of(app, sel)
	return h
end

-- A part's laid-out width: the clip around its text (the first box the
-- selector prints).
local function width(app: any, sel: string): number
	local first = string.match(app:tree(sel .. "[1]"), "[^\n]+") or ""
	return tonumber(string.match(first, " %-?%d+,%-?%d+ (%d+)x%d+")) or -1
end

test("composer place: a new chat's project is in the pill, the checkout's glyph says where it runs", function()
	local app = mounted()
	rested(app)
	expect(app:text("#composer-place-project[1]")).to_be("zeron")
	expect(app:has('role:button "Checkout" >> .place-lead-worktree')).to_be(true)
	expect(showing(app, "#composer-place-branch")).to_be(true)
	expect(app:text("#composer-place-branch[1]")).to_be("zeron/create-readme-update-for-onboarding")
	-- The project's own checkout: a folder.
	app:set_state("test_place_checkout", "local")
	rested(app)
	expect(app:has('role:button "Checkout" >> .place-lead-folder')).to_be(true)
	-- No project: "No project", no branch and no checkout to choose.
	app:set_state("test_place_checkout", "worktree")
	app:set_state("test_place_project", "none")
	rested(app)
	expect(app:text("#composer-place-project[1]")).to_be("No project")
	expect(showing(app, "#composer-place-branch")).to_be(false)
	expect(showing(app, 'role:button "Checkout"')).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("composer place: words fold by the pill's width as gpui's, and no chip moves its height", function()
	local app = mounted()
	local function words(): (boolean, boolean)
		return showing(app, 'role:switch "Plan mode" >> "Plan"'), chip_text(app, "Access") ~= ""
	end
	app:set_state("test_place_w", 840)
	rested(app)
	local plan, access = words()
	expect(plan and access).to_be(true)
	local tall = height(app, 'role:button "Model"')
	-- Opened out (a new chat's), its words stay until the row can't hold them.
	app:set_state("test_place_w", 420)
	rested(app)
	plan, access = words()
	expect(plan and access).to_be(true)
	app:set_state("test_place_w", 360)
	rested(app)
	plan, access = words()
	expect(plan or access).to_be(false)
	expect(height(app, 'role:button "Model"')).to_be(tall)
	expect(app:error()).to_be(nil)
end)

test("composer place: each chip has gpui's height, the ring its box", function()
	local app = mounted()
	rested(app)
	local _, access = box_of(app, 'role:button "Access"')
	local _, place = box_of(app, 'role:button "Branch"')
	local _, model = box_of(app, 'role:button "Model"')
	local w, ring = box_of(app, "#composer-context")
	expect(access).to_be(24)
	expect(model).to_be(32)
	expect(place).to_be(20)
	expect(ring).to_be(24)
	expect(w > 24 and w <= 56).to_be(true)
end)

test("composer place: the context card opens on a click and stays", function()
	local app = mounted()
	app:hover("#composer-context")
	rested(app)
	expect(showing(app, "#composer-context-card")).to_be(false)
	app:click("#composer-context")
	rested(app)
	expect(showing(app, "#composer-context-card")).to_be(true)
	expect(app:text("#composer-context-used")).to_contain("408,389 / 1,000,000 tokens")
	expect(app:text("#composer-context-left")).to_contain("591,611 tokens remaining")
	expect(app:count(".composer-context-panel")).to_be(1)
	expect(app:count(".composer-context-panel >> .composer-context-panel")).to_be(0)
	expect(app:count(".composer-context-panel >> .hover-card-frame")).to_be(0)
	app:hover("#composer-context-percent")
	rested(app)
	expect(showing(app, "#composer-context-card")).to_be(true)
	app:click("#composer-context")
	rested(app)
	expect(showing(app, "#composer-context-card")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("composer place: the ring says how full, in its ink's steps", function()
	local app = mounted()
	rested(app)
	expect(app:text("#composer-context-percent")).to_be("41%")
	app:set_state("test_place_usage", { tokens = 950000, window = 1000000 })
	rested(app)
	expect(app:text("#composer-context-percent")).to_be("95%")
	app:set_state("test_place_usage", { tokens = 1000 })
	rested(app)
	expect(app:has("#composer-context")).to_be(false)
	expect(app:error()).to_be(nil)
end)

-- zgx's own menus (the built-in text menu) wear our menus' panel tint and
-- row wash (ds/tokens), in both themes, not zgx's highlight.
test("composer place: zgx's built-in menus take our menu tint and row wash", function()
	local app = mounted()
	for _, dark in { true, false } do
		app:set_state("test_place_dark", dark)
		app:frames(5)
		local t = app:state("test_place_menu_tokens") :: any
		expect(t.menu_bg).to_be(t.tint)
		expect(t.menu_hover).to_be(t.hover)
	end
	expect(app:error()).to_be(nil)
end)

test("composer: short of room the effort goes first, the model's name last", function()
	local app = mounted()
	app:set_state("test_place_effort", "high")
	app:set_state("test_place_w", 800)
	rested(app)
	local function right_of(sel: string): number
		local x, w = string.match(string.match(app:tree(sel), "[^\n]+") or "", " (%-?%d+),%-?%d+ (%d+)x%d+")
		return (tonumber(x) or 0) + (tonumber(w) or 0)
	end
	expect(right_of('role:button "Model" >> "High"') < right_of('role:button "Model"')).to_be(true)
	-- Held to less than it wants, the effort's words are past the chip's edge.
	app:set_state("test_place_w", 300)
	rested(app)
	local high = string.match(string.match(app:tree('role:button "Model" >> "High"'), "[^\n]+") or "", " (%-?%d+),%-?%d+ ")
	expect((tonumber(high) :: number) >= right_of('role:button "Model"') - 6).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("composer: the model's chip never leaves the row, down to its glyph", function()
	local app = mounted()
	app:set_state("test_place_w", 260)
	rested(app)
	local function x_w(sel: string): (number, number)
		local x, w = string.match(string.match(app:tree(sel), "[^\n]+") or "", " (%-?%d+),%-?%d+ (%d+)x%d+")
		return tonumber(x) or 0, tonumber(w) or 0
	end
	local mx, mw = x_w('role:button "Model"')
	local sx = x_w('role:button "Send message"')
	local ax = x_w('role:button "Access"')
	local px, pw = x_w("#composer-plan")
	expect(mx + mw <= sx).to_be(true)
	expect(ax >= px + pw).to_be(true)
	expect(chip_text(app, "Access")).to_be("")
	-- Back with room, its name again.
	app:set_state("test_place_w", 800)
	rested(app)
	expect(chip_text(app, "Model")).to_contain("Opus")
	expect(app:error()).to_be_nil()
end)

test("composer place: a plugin's toolbar slots stand with their chips, in the pill", function()
	local app = mounted()
	app:set_state("test_place_plugin", true)
	rested(app)
	local function at(sel: string): (number, number)
		local x, y = string.match(string.match(app:tree(sel), "[^\n]+") or "", " (%-?%d+),(%-?%d+) ")
		return tonumber(x) or -1, tonumber(y) or -1
	end
	local sx, sy = at("#plug-start")
	local ex, ey = at("#plug-end")
	local clip_x, clip_y = at('role:button "Attach"')
	local send_x = at('role:button "Send message"')
	local _, footer_y = at('role:button "Branch"')
	expect(sx > clip_x and sy < footer_y and sy >= clip_y - 8).to_be(true)
	expect(ex < send_x and ey < footer_y).to_be(true)
	expect(app:error()).to_be_nil()
end)
