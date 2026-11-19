-- The composer's footer over a new chat's wallpaper (ui/composer_place_
-- fixture.luau): the place chip ([tile] project › branch), the faint
-- backing its chips take only over art, which part of the chip gives way
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

-- A part's laid-out width: the clip around its text (the first box the
-- selector prints).
local function width(app: any, sel: string): number
	local first = string.match(app:tree(sel .. "[1]"), "[^\n]+") or ""
	return tonumber(string.match(first, " %-?%d+,%-?%d+ (%d+)x%d+")) or -1
end

test("composer place: over a wallpaper the footer sits on a tray, bare without", function()
	local app = mounted()
	-- Over art: the pill's glass continued under the row, one tray; no
	-- backing of any chip's own.
	expect(app:count(".composer-tray")).to_be(1)
	expect(app:count(".composer-over-art")).to_be(0)
	-- No art: no tray (gone once it's faded).
	app:set_state("test_place_art", false)
	rested(app)
	expect(app:count(".composer-tray")).to_be(0)
	-- And back.
	app:set_state("test_place_art", true)
	rested(app)
	expect(app:count(".composer-tray")).to_be(1)
	expect(app:error()).to_be(nil)
end)

test("composer place: where it runs leads, a worktree's glyph or a folder, then project › branch", function()
	local app = mounted()
	rested(app)
	-- A worktree: its glyph, the project's name › the branch. No tile.
	expect(app:has("#composer-place-lead >> .place-lead-worktree")).to_be(true)
	expect(app:text("#composer-place-lead")).to_be("")
	expect(app:text("#composer-place-project[1]")).to_be("zeron")
	expect(showing(app, "#composer-place-branch")).to_be(true)
	expect(app:text("#composer-place-branch[1]")).to_be("zeron/create-readme-update-for-onboarding")
	-- The project's own checkout: a folder.
	app:set_state("test_place_checkout", "local")
	rested(app)
	expect(app:has("#composer-place-lead >> .place-lead-folder")).to_be(true)
	-- No project: the folder, "No project", no branch.
	app:set_state("test_place_checkout", "worktree")
	app:set_state("test_place_project", "none")
	rested(app)
	expect(app:has("#composer-place-lead >> .place-lead-folder")).to_be(true)
	expect(app:text("#composer-place-project[1]")).to_be("No project")
	expect(showing(app, "#composer-place-branch")).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("composer place: short of room the branch gives way first, then the project keeps its least", function()
	local app = mounted()
	app:set_state("test_place_project", "atlas")
	app:set_state("test_place_w", 840)
	rested(app)
	-- Roomy: the project's name at its most, the branch at its most.
	local project_wide = width(app, "#composer-place-project")
	local branch_wide = width(app, "#composer-place-branch")
	expect(project_wide).to_be(140)
	expect(branch_wide).to_be(180)
	-- Narrower: the branch gives way, the project's name doesn't (the
	-- access mode folds to its icon to keep some of the branch).
	app:set_state("test_place_w", 440)
	rested(app)
	expect(width(app, "#composer-place-project")).to_be(project_wide)
	local b = width(app, "#composer-place-branch")
	expect(b < branch_wide and b > 36).to_be(true)
	-- Narrower still: the branch down to its sliver, and only then the
	-- project's name, to no less than 44 of it.
	app:set_state("test_place_w", 360)
	rested(app)
	expect(width(app, "#composer-place-branch")).to_be(36)
	local p = width(app, "#composer-place-project")
	expect(p < project_wide and p >= 44).to_be(true)
	-- Past that: the glyph alone.
	app:set_state("test_place_w", 270)
	rested(app)
	expect(showing(app, "#composer-place-project")).to_be(false)
	expect(app:has("#composer-place-lead >> .place-lead-worktree")).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("composer place: the context ring opens a card of the numbers, as Zeron writes them", function()
	local app = mounted()
	app:hover("#composer-context")
	rested(app)
	expect(app:has("#composer-context-card")).to_be(true)
	expect(app:text("#composer-context-used")).to_be("408,389 / 1,000,000 tokens")
	expect(app:text("#composer-context-left")).to_be("591,611 tokens remaining")
	expect(app:has('#composer-context-card >> "41%"')).to_be(true)
	-- Past the window: nothing left, not less than nothing.
	app:set_state("test_place_usage", { tokens = 1048576, window = 1000000 })
	settle(app)
	expect(app:text("#composer-context-left")).to_be("0 tokens remaining")
	expect(app:has('#composer-context-card >> "100%"')).to_be(true)
	-- A window with nothing counted yet.
	app:set_state("test_place_usage", { window = 200000 })
	settle(app)
	expect(app:text("#composer-context-used")).to_be("200,000 token capacity")
	expect(app:text("#composer-context-left")).to_be("Waiting for context usage")
	expect(app:error()).to_be(nil)
end)

test("composer place: the context ring shows only once a window is reported", function()
	local app = mounted()
	expect(showing(app, "#composer-context")).to_be(true)
	app:set_state("test_place_usage", nil)
	settle(app)
	expect(showing(app, "#composer-context")).to_be(false)
	-- Tokens alone, no window: still none (an agent that never says one).
	app:set_state("test_place_usage", { tokens = 5417 })
	settle(app)
	expect(showing(app, "#composer-context")).to_be(false)
	app:set_state("test_place_usage", { tokens = 5417, window = 1048576 })
	settle(app)
	expect(showing(app, "#composer-context")).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("composer place: the footer's chips are one height, the ring one of them", function()
	local app = mounted()
	rested(app)
	local function box(sel: string): (number, number)
		local first = string.match(app:tree(sel), "[^\n]+") or ""
		local w, h = string.match(first, " %-?%d+,%-?%d+ (%d+)x(%d+)")
		return tonumber(w) or -1, tonumber(h) or -1
	end
	local _, access = box('role:button "Access"')
	local _, place = box('role:button "Branch"')
	local _, model = box('role:button "Model"')
	local _, ring = box("#composer-context")
	expect(access).to_be(24)
	expect(place).to_be(24)
	expect(model).to_be(24)
	expect(ring).to_be(24)
	-- The ring at gpui's footer size (16) and the percent, 6 in at each side.
	local w = box("#composer-context")
	local pct = box("#composer-context-percent")
	expect(w).to_be(6 + 16 + 5 + pct + 6)
end)

-- A hover card is framed once (ds/hover_card): zgx's layer hugs the card
-- exactly (its box the card's), and one frame paints in it. A card that
-- painted its own shadow, or a second frame round it, shows as a dark rim
-- round a bigger box in the popup's window.
test("composer place: the context card is framed once, its layer hugging it", function()
	local app = mounted()
	app:hover("#composer-context")
	rested(app)
	local function box(sel: string): { number }
		local first = string.match(app:tree(sel), "[^\n]+") or ""
		local x, y, w, h = string.match(first, " (%-?%d+),(%-?%d+) (%d+)x(%d+)")
		return { tonumber(x) or -1, tonumber(y) or -1, tonumber(w) or -1, tonumber(h) or -1 }
	end
	local layer = box('role:dialog "Tooltip"')
	local card = box("#composer-context-card")
	for i = 1, 4 do
		expect(math.abs(layer[i] - card[i]) <= 0.5).to_be(true)
	end
	expect(app:count('role:dialog "Tooltip" >> .hover-card-frame')).to_be(1)
	expect(app:error()).to_be(nil)
end)

-- On the tray the row keeps the pill's spacing: its chips sit as far below
-- the pill's edge as + sits above it, and line up with + and Send.
test("composer place: on the tray the chips keep the pill's inset, lined up with + and Send", function()
	local app = mounted()
	rested(app)
	local function box(sel: string): { number }
		local first = string.match(app:tree(sel), "[^\n]+") or ""
		local x, y, w, h = string.match(first, " (%-?%d+),(%-?%d+) (%d+)x(%d+)")
		return { tonumber(x) or -1, tonumber(y) or -1, tonumber(w) or -1, tonumber(h) or -1 }
	end
	local plus, send = box('role:button "Attach"'), box('role:button "Send message"')
	local access, ring = box('role:button "Access"'), box("#composer-context")
	-- + sits 12 above the pill's edge (11 inside its hairline); the chips
	-- A tight step (6) below the pill: 18 from +'s foot to the chips' top.
	expect(access[2] - (plus[2] + plus[4])).to_be(18)
	-- Their outer edges on +'s and Send's.
	expect(access[1]).to_be(plus[1])
	expect(ring[1] + ring[3]).to_be(send[1] + send[3])
	-- Bare: the footer as ever (8 under the pill, 10 in).
	app:set_state("test_place_art", false)
	rested(app)
	access, plus = box('role:button "Access"'), box('role:button "Attach"')
	expect(access[2] - (plus[2] + plus[4])).to_be(20)
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

-- A text's laid-out width in a chip (0: not showing).
local function text_w(app: any, name: string, text: string): number
	for line in app:tree(`role:button "{name}"`):gmatch("[^\n]+") do
		if not line:find(" hidden", 1, true) and line:find(`"{text}"`, 1, true) then
			return tonumber(line:match(" %-?%d+,%-?%d+ (%d+)x%d+")) or 0
		end
	end
	return 0
end

test("composer footer: short of room the model's name shortens first, its effort stays whole", function()
	local app = mounted()
	app:set_state("test_place_model", "sonnet")
	app:set_state("test_place_w", 800)
	rested(app)
	-- Roomy: all of it, the chip as wide as what it says.
	local name, effort = text_w(app, "Model", "Claude Sonnet 4.5"), text_w(app, "Model", "High")
	expect(name > 60 and effort > 0).to_be(true)
	-- Narrowing: the name shortens while the access mode still has its
	-- name, and the effort is whole at every width.
	local shortened, access_folded = nil :: number?, nil :: number?
	for w = 640, 180, -4 do
		app:set_state("test_place_w", w)
		app:frames(10)
		expect(text_w(app, "Model", "High")).to_be(effort)
		if not shortened and text_w(app, "Model", "Claude Sonnet 4.5") < name then
			shortened = w
		end
		if not access_folded and chip_text(app, "Access") == "" then
			access_folded = w
		end
	end
	expect(shortened ~= nil and access_folded ~= nil).to_be(true)
	expect((shortened :: number) > (access_folded :: number)).to_be(true)
	-- At the last of it the name is shortened, still there.
	local least = text_w(app, "Model", "Claude Sonnet 4.5")
	expect(least > 0 and least < name).to_be(true)
	-- Wide again, all of it.
	app:set_state("test_place_w", 800)
	rested(app)
	expect(text_w(app, "Model", "Claude Sonnet 4.5")).to_be(name)
	expect(text_w(app, "Model", "High")).to_be(effort)
	expect(chip_text(app, "Access")).never.to_be("")
	expect(app:error()).to_be_nil()
end)

test("composer footer: everything else folded, the model keeps its name's start, its effort and the ring", function()
	local app = mounted()
	app:set_state("test_place_model", "sonnet")
	app:set_state("test_place_w", 800)
	rested(app)
	local name, effort = text_w(app, "Model", "Claude Sonnet 4.5"), text_w(app, "Model", "High")
	app:set_state("test_place_w", 310)
	rested(app)
	expect(chip_text(app, "Access")).to_be("")
	expect(chip_text(app, "Branch")).to_be("")
	local kept = text_w(app, "Model", "Claude Sonnet 4.5")
	expect(kept > 32 and kept < name).to_be(true)
	expect(text_w(app, "Model", "High")).to_be(effort)
	-- Past that: the name at its least, never gone; the effort and the ring stay.
	app:set_state("test_place_w", 230)
	rested(app)
	local least = text_w(app, "Model", "Claude Sonnet 4.5")
	expect(least > 0 and least < kept).to_be(true)
	expect(text_w(app, "Model", "High")).to_be(effort)
	expect(showing(app, "#composer-context")).to_be(true)
	-- Back with room, all of it.
	app:set_state("test_place_w", 800)
	rested(app)
	expect(text_w(app, "Model", "Claude Sonnet 4.5")).to_be(name)
	expect(app:error()).to_be_nil()
end)

-- Made narrow at once (a side chat opened in a small window, a split): the
-- access mode's chip alone wider than the room left, the place chip squeezed
-- to nothing. It still folds, rather than taking that for not laid out and
-- clipping every chip where it stood.
test("composer footer: narrowed at once past the place chip, it still folds", function()
	local app = mounted()
	app:set_state("test_place_model", "sonnet")
	app:set_state("test_place_w", 800)
	rested(app)
	local effort = text_w(app, "Model", "High")
	app:set_state("test_place_w", 200)
	rested(app)
	expect(chip_text(app, "Access")).to_be("")
	expect(chip_text(app, "Branch")).to_be("")
	expect(text_w(app, "Model", "High")).to_be(effort)
	expect(app:error()).to_be_nil()
end)

test("composer place: a long draft collapses to its last rows by its toggle or ⌘⇧↓, keeps taking words, and a send opens it again", function()
	local app = mounted()
	app:set_state("test_place_art", false)
	local function lines(n: number): string
		local out = {}
		for i = 1, n do
			table.insert(out, `line {i}`)
		end
		return table.concat(out, "\n")
	end
	local function said(): string
		return string.match(app:tree("#composer-collapse"), '"([^"]+)"') or ""
	end
	local function height(sel: string): number
		local first = string.match(app:tree(sel), "[^\n]+") or ""
		return tonumber(string.match(first, " %-?%d+,%-?%d+ %d+x(%d+)")) or -1
	end
	app:set_state("test_place_draft", lines(6))
	rested(app)
	expect(showing(app, "#composer-collapse")).to_be(false)
	app:set_state("test_place_draft", lines(12))
	rested(app)
	expect(showing(app, "#composer-collapse")).to_be(true)
	expect(said()).to_be("Collapse")
	local tall = height("role:textbox")
	app:click("#composer-collapse")
	rested(app)
	expect(said()).to_be("Expand")
	expect(height("role:textbox")).to_be(66)
	expect(tall > 66).to_be(true)
	-- Typed into while collapsed: still collapsed, the words at its end.
	app:set_state("test_place_draft", lines(13))
	rested(app)
	expect(height("role:textbox")).to_be(66)
	app:click("role:textbox")
	app:key("cmd+shift+ArrowDown")
	rested(app)
	expect(said()).to_be("Collapse")
	app:key("cmd+shift+ArrowDown")
	rested(app)
	expect(said()).to_be("Expand")
	-- Sent (the draft emptied): the next draft starts opened.
	app:set_state("test_place_draft", "")
	rested(app)
	app:set_state("test_place_draft", lines(12))
	rested(app)
	expect(said()).to_be("Collapse")
end)

test("composer place: Work in submenus open immediately on hover", function()
	local app = mounted()
	app:set_state("test_place_devices", {
		{ value = "fedora", label = "fedora", you = true },
		{ value = "mac", label = "Mac" },
	})
	app:set_state("test_place_device", "fedora")
	settle(app)
	app:click('role:button "Branch"')
	settle(app)
	app:hover('role:menuitem "Device"')
	expect(app:has('"Search devices…"')).to_be(true)
	expect(app:has('"Mac"')).to_be(true)
	app:hover('role:menuitem "Project"')
	expect(showing(app, '"Search devices…"')).to_be(false)
	expect(app:has('"Search projects…"')).to_be(true)
	expect(app:has('"Atlas Platform Monorepo"')).to_be(true)
	app:hover('role:menuitem "Checkout"')
	expect(showing(app, '"Search projects…"')).to_be(false)
	expect(app:has('"An isolated copy, on a branch of its own."')).to_be(true)
	app:hover('role:option "Local checkout Work in the project\'s own folder."')
	app:click('role:option "Local checkout Work in the project\'s own folder."')
	settle(app)
	expect(app:has("#composer-place-lead >> .place-lead-folder")).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("composer place: hover submenus close after leaving the row and panel", function()
	local app = mounted()
	app:click('role:button "Branch"')
	settle(app)
	app:hover('role:menuitem "Project"')
	expect(showing(app, '"Search projects…"')).to_be(true)
	-- Leaving the row without entering its panel closes it.
	app:hover('"Work in"')
	app:wait("#never-shown", { timeout_ms = 250 })
	expect(showing(app, '"Search projects…"')).to_be(false)
	app:hover('role:menuitem "Project"')
	-- Crossing into the panel cancels the pending close, so it remains
	-- possible to browse and select an option.
	app:hover('"Atlas Platform Monorepo"')
	app:wait("#never-shown", { timeout_ms = 250 })
	expect(showing(app, '"Search projects…"')).to_be(true)
	-- Returning to the row also keeps the submenu open.
	app:hover('role:menuitem "Project"')
	app:wait("#never-shown", { timeout_ms = 250 })
	expect(showing(app, '"Search projects…"')).to_be(true)
	app:hover('"Atlas Platform Monorepo"')
	app:hover('"Work in"')
	app:wait("#never-shown", { timeout_ms = 250 })
	expect(showing(app, '"Search projects…"')).to_be(false)
	expect(showing(app, '"Search branches…"')).to_be(true)
	expect(app:error()).to_be(nil)
end)

test("composer place: on Cloud, what its machine reaches stands in the checkout's place", function()
	local app = mounted()
	app:set_state("test_place_devices", {
		{ value = "fedora", label = "fedora", you = true },
		{ value = "cloud", label = "Cloud", cloud = true },
	})
	app:set_state("test_place_device", "fedora")
	settle(app)
	app:click('role:button "Branch"')
	settle(app)
	-- On a computer: a checkout, nothing of reach.
	expect(app:has('role:menuitem "Checkout"')).to_be(true)
	expect(app:has('role:menuitem "Reaches"')).to_be(false)
	-- On Cloud: its machine's reach instead, the account's default shown.
	app:set_state("test_place_device", "cloud")
	settle(app)
	expect(showing(app, 'role:menuitem "Checkout"')).to_be(false)
	expect(showing(app, 'role:menuitem "Reaches"')).to_be(true)
	expect(app:has('"All devices"')).to_be(true)
	expect(app:has("#composer-place-lead >> .place-lead-cloud")).to_be(true)
	-- Picked: the new chat's own.
	app:hover('role:menuitem "Reaches"')
	expect(app:has('"Only its own machine and the chats it starts."')).to_be(true)
	app:click('role:option "Isolated Only its own machine and the chats it starts."')
	settle(app)
	expect(app:state("test_place_reach")).to_be("isolated")
	expect(app:error()).to_be(nil)
end)

test("composer place: a new Cloud chat shows the account's default reach until one is picked", function()
	local app = mounted()
	app:set_state("test_place_devices", { { value = "cloud", label = "Cloud", cloud = true } })
	app:set_state("test_place_device", "cloud")
	app:set_state("test_place_reach_default", "isolated")
	settle(app)
	app:click('role:button "Branch"')
	settle(app)
	expect(showing(app, 'role:menuitem "Reaches"')).to_be(true)
	expect(app:has('"Isolated"')).to_be(true)
	expect(app:state("test_place_reach")).to_be(nil)
	expect(app:error()).to_be(nil)
end)

test("composer place: an existing chat's chip opens where it works, with nothing to change", function()
	local app = mounted()
	app:set_state("test_place_fixed", { device = "Mac", name = "main", worktree = true, project = "zeron", hue = "violet" })
	rested(app)
	expect(app:has("#composer-place-lead >> .place-lead-worktree")).to_be(true)
	app:click('role:button "Branch main"')
	settle(app)
	expect(app:has('"Work in"')).to_be(true)
	expect(app:has('role:menuitem "Device"')).to_be(true)
	expect(app:has('role:menuitem "Project"')).to_be(true)
	expect(app:has('role:menuitem "Checkout"')).to_be(true)
	-- Its rows open nothing: a chat can't move.
	app:hover('role:menuitem "Device"')
	app:click('role:menuitem "Device"')
	settle(app)
	expect(app:has('"Search devices…"')).to_be(false)
	app:hover('role:menuitem "Project"')
	expect(app:has('"Search projects…"')).to_be(false)
	-- And no branches to pick, nor a reach off Cloud.
	expect(app:has('"Search branches…"')).to_be(false)
	expect(app:has('role:menuitem "Reaches"')).to_be(false)
	expect(app:error()).to_be(nil)
end)

test("composer place: an existing Cloud chat leads with Cloud's glyph, and its reach can change", function()
	local app = mounted()
	app:set_state("test_place_fixed", { device = "Cloud", name = "main", worktree = false, project = "zeron", cloud = true, reach = "devices" })
	rested(app)
	expect(app:has("#composer-place-lead >> .place-lead-cloud")).to_be(true)
	app:click('role:button "Branch main"')
	settle(app)
	expect(app:has('role:menuitem "Checkout"')).to_be(false)
	expect(showing(app, 'role:menuitem "Reaches"')).to_be(true)
	expect(app:has('"All devices"')).to_be(true)
	app:hover('role:menuitem "Reaches"')
	app:click('role:option "Isolated Only its own machine and the chats it starts."')
	settle(app)
	expect(app:state("test_place_reach_picked")).to_be("isolated")
	expect(app:error()).to_be(nil)
end)

test("composer place: an edge that can't keep a reach offers no choice of it", function()
	local app = mounted()
	app:set_state("test_place_devices", { { value = "cloud", label = "Cloud", cloud = true } })
	app:set_state("test_place_device", "cloud")
	app:set_state("test_place_reach_default", nil)
	settle(app)
	app:click('role:button "Branch"')
	settle(app)
	expect(app:has('role:menuitem "Device"')).to_be(true)
	expect(showing(app, 'role:menuitem "Reaches"')).to_be(false)
	expect(app:error()).to_be(nil)
end)
