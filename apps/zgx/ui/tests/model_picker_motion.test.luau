-- The model picker's motion, on the picker in ui/picker_fixture.luau: its
-- panel is never see-through (opening, changing page, following its trigger),
-- changing page changes its size once (the panel is a popup window: a size
-- eased over frames resized it on every one), Escape steps back from the list
-- whatever in it has the keys, and the reasoning thumb is where the pointer
-- is, not springing after it.

local function settle(app: any)
	app:frames(30)
end

-- A node's box (x, y, w, h), from its tree line. (The panel by its class: a
-- tooltip is a dialog too.)
local function box(app: any, target: string): (number, number, number, number)
	local line = app:tree(target):match("[^\n]+") or ""
	local x, y, w, h = line:match("(%-?[%d%.]+),(%-?[%d%.]+) ([%d%.]+)x([%d%.]+)")
	return tonumber(x) or -1, tonumber(y) or -1, tonumber(w) or -1, tonumber(h) or -1
end
local function height(app: any): number
	local _, _, _, h = box(app, ".dropdown-panel")
	return h
end
local function alpha(app: any): { panel: number }
	return app:state("test_picker_alpha") :: any
end
local function watch(app: any)
	app:set_state("test_picker_alpha", { panel = 1 })
end
-- Whether the panel shows (not only laid out, hidden, as a closed one is).
local function listed(app: any): boolean
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(" dialog ", 1, true) and not line:find(" hidden", 1, true) and not line:find('"Tooltip"', 1, true) then
			return true
		end
	end
	return false
end

local function opened(app: any)
	app:resize(600, 700)
	settle(app)
	watch(app)
	app:click('role:button "Model"')
	settle(app)
end

test("the picker opens and changes page solid, its size changing once", function()
	local app = mount("picker_fixture.luau")
	opened(app)
	local folded_h = height(app)
	expect(alpha(app).panel).to_be(1)

	-- The list: its whole size the first frame, nothing in it fading in.
	app:click("#model-picker-title")
	app:frames(1)
	local out_h = height(app)
	expect(out_h > folded_h).to_be(true)
	settle(app)
	expect(height(app)).to_be(out_h)
	expect(alpha(app).panel).to_be(1)

	-- Back (Escape): the panel's size the first frame.
	app:key("Escape")
	app:frames(1)
	expect(height(app)).to_be(folded_h)
	settle(app)
	expect(height(app)).to_be(folded_h)
	expect(alpha(app).panel).to_be(1)
	expect(listed(app)).to_be(true)
end)

test("the panel stays solid as its trigger moves", function()
	local app = mount("picker_fixture.luau")
	opened(app)
	app:click("#model-picker-title")
	settle(app)
	watch(app)
	local _, y0 = box(app, ".dropdown-panel")
	app:set_state("test_picker_above", 80)
	settle(app)
	local _, y1 = box(app, ".dropdown-panel")
	expect(y1 - y0).to_be(80)
	app:set_state("test_picker_above", 0)
	settle(app)
	expect(alpha(app).panel).to_be(1)
end)

-- Escape steps back from the list whatever in it has the keys (it reached the
-- popover from a jump button, which closed it); on the panel it closes, from
-- the slider too.
test("Escape steps back from the list, and closes the panel", function()
	local app = mount("picker_fixture.luau")
	opened(app)
	local panel_h = height(app)
	app:click("#model-picker-title")
	settle(app)
	app:click('role:button "Jump to Codex"')
	settle(app)
	app:key("Escape")
	settle(app)
	expect(listed(app)).to_be(true)
	expect(height(app)).to_be(panel_h)
	app:click('role:group "Reasoning"')
	settle(app)
	app:key("Escape")
	settle(app)
	expect(listed(app)).to_be(false)
end)

-- A press on the chip of an open picker closes it.
test("the chip of an open picker closes it", function()
	local app = mount("picker_fixture.luau")
	opened(app)
	app:click("#model-picker-title")
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	expect(listed(app)).to_be(false)
	-- And opens it again, on the panel.
	app:click('role:button "Model"')
	settle(app)
	expect(listed(app)).to_be(true)
end)

-- Dragged, the thumb is where the pointer is (set as it moves, not sprung
-- after the level), and the value is the level it is let go over, set as
-- it is let go.
test("the reasoning thumb follows the pointer, the frame it moves", function()
	local app = mount("picker_fixture.luau")
	app:set_state("test_picker_effort", "low")
	opened(app)
	-- Where the thumb sits (its left, along the rail) at a fraction of the
	-- way: 44 wide, flush with the rail's ends.
	local _, _, gw = box(app, 'role:group "Reasoning"')
	local function at(f: number): number
		return math.floor(f * (gw - 44) + 0.5)
	end
	expect(app:state("test_picker_thumb")).to_be(at(0))
	-- From Low (the rail's start) to its middle, Medium: the thumb there
	-- the frame it's let go (it sprang after the level, a frame on still
	-- most of the way back).
	app:drag('role:group "Reasoning"', 'role:group "Reasoning"')
	app:frames(1)
	expect(app:state("test_picker_thumb")).to_be(at(0.5))
	expect(app:state("test_picker_effort")).to_be("medium")
	settle(app)
	expect(app:state("test_picker_thumb")).to_be(at(0.5))
end)

-- A new chat's composer, mid-page, grows down as its draft grows: its chip
-- held while its model picker is open (and the picker hanging from it, still),
-- it goes to its place as the picker closes. (On ui/chat_fixture.luau.)
test("a new chat's composer holds still under its open model picker", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 760)
	app:command("go_home")
	settle(app)
	local function chip_y(): number
		local _, y = box(app, '#chat-composer >> role:button "Model"')
		return y
	end
	local before = chip_y()
	app:click('#chat-composer >> role:button "Model"')
	settle(app)
	expect(listed(app)).to_be(true)
	local _, panel_y = box(app, ".dropdown-panel")
	app:set_state("chat.composer.text", "one\ntwo\nthree\nfour")
	settle(app)
	expect(chip_y()).to_be(before)
	local _, panel_now = box(app, ".dropdown-panel")
	expect(panel_now).to_be(panel_y)
	-- Closed: to its place (lower, as it grew down).
	app:key("Escape")
	settle(app)
	expect(listed(app)).to_be(false)
	expect(chip_y() > before).to_be(true)
	app:set_state("chat.composer.text", "")
	settle(app)
end)

-- The compact list scrolled into a group past the strip's room: that group's
-- tab lights and the strip scrolls it into sight, as the full layout's tabs do.
test("the compact list scrolled to a far group brings its tab into the strip", function()
	local app = mount("picker_fixture.luau")
	app:set_state("test_picker_many", true)
	opened(app)
	app:click("#model-picker-title")
	settle(app)
	local px, _, pw = box(app, ".dropdown-panel")
	local tx, _, tw = box(app, 'role:button "Jump to Agent 6"')
	expect(tx + tw > px + pw).to_be(true)
	app:scroll('role:option ~"Mini"', 4000)
	settle(app)
	tx, _, tw = box(app, 'role:button "Jump to Agent 6"')
	expect(tx + tw <= px + pw).to_be(true)
	expect(tx >= px).to_be(true)
end)
