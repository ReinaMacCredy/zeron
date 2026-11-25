-- The sessions' tabs on the top bar: they start past Back and Forward (over the page
-- with the sidebar open, closed up on its toggle with it shut), each chat opened keeps
-- its tab, and the one showing is never left in the overflow. And a new chat's page
-- stays a card with the sidebar closed. On the shell in ui/shell_fixture.luau.

local CHATS = { "Fix the build", "Write the docs", "Tidy the tests", "Ship the release", "Answer the review" }
local IDS = { "a1", "a2", "a3", "b1", "b2" }

local function box(app: any, target: string): { x: number, w: number, y: number }
	local line = app:tree(target):match("[^\n]+") :: string
	local x, y, w = line:match(" (%-?[%d%.]+),(%-?[%d%.]+) ([%d%.]+)x[%d%.]+")
	return { x = tonumber(x) :: number, y = tonumber(y) :: number, w = tonumber(w) :: number }
end

local function open(app: any, title: string)
	app:click(`#shell-sidebar >> role:button "{title}"`)
	app:frames(30)
end

-- A tab for each chat, each composed with a partner (a chat alone has none), the
-- last showing.
local function all_open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	app:command("partner_chats")
	app:frames(10)
	open(app, CHATS[#CHATS])
	local tabs = {}
	for i, id in IDS do
		table.insert(tabs, { kind = "row", key = id, children = { { kind = "leaf", id = id }, { kind = "leaf", id = `p{i}` } } })
	end
	app:set_state("inset_split_trees", tabs)
	app:frames(30)
	return app
end

-- Wholly in the strip's room.
local function in_view(app: any, id: string)
	local room, tab = box(app, "#session-tabs-scroll"), box(app, `#session-tab-{id}`)
	expect(tab.x >= room.x - 0.5).to_be(true)
	expect(tab.x + tab.w <= room.x + room.w + 0.5).to_be(true)
end

test("Back and Forward stand over the page, the tabs past them, closing up on the toggle with the sidebar shut", function()
	local app = all_open()
	local page = box(app, "#shell-page")
	local back, forward = box(app, '#titlebar >> role:button "Back"'), box(app, '#titlebar >> role:button "Forward"')
	expect(back.x >= page.x).to_be(true)
	local open_x = box(app, "#session-tabs").x
	expect(open_x >= forward.x + forward.w).to_be(true)
	expect(app:has("#titlebar-wordmark")).to_be(true)
	expect(app:has("#titlebar-new-tab")).to_be(true)
	-- The sidebar's "…" at its end, by them.
	local dots = box(app, 'role:button "Sidebar view options"')
	expect(dots.x + dots.w < back.x).to_be(true)
	expect(back.x - (dots.x + dots.w) < 24).to_be(true)

	app:click('#titlebar >> "Toggle sidebar"')
	app:frames(60)
	local toggle = box(app, '#titlebar >> role:button "Toggle sidebar"')
	back = box(app, '#titlebar >> role:button "Back"')
	expect(back.x - (toggle.x + toggle.w) < 16).to_be(true)
	expect(app:has("#titlebar-wordmark")).to_be(false)
	-- (New tab past Forward, and the tabs past that.)
	local plus = box(app, '#titlebar >> role:button "New tab"')
	local shut_x = box(app, "#session-tabs").x
	expect(shut_x >= plus.x + plus.w).to_be(true)
	expect(shut_x < open_x).to_be(true)
	app:click('#titlebar >> role:button "New tab"')
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/")
	app:frames(60)
	expect(app:has("#titlebar-new-tab")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("the sidebar's menu still takes a click with the tabs along the top bar", function()
	local app = all_open()
	app:click('role:button "Sidebar view options"')
	app:frames(20)
	expect(app:wait('"Compact"', { timeout_ms = 5000 })).to_be(true)
	expect(app:error()).to_be_nil()
end)

-- Nothing laid over the page takes its clicks: the composer takes the keyboard, the
-- model picker opens.
local function page_takes_clicks(app: any)
	app:click('#chat-composer-dock >> role:textbox "Message"')
	app:frames(10)
	local focused = ""
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(" focused", 1, true) then
			focused = line
		end
	end
	expect(focused).to_contain("textbox")
	app:click('#chat-composer-dock >> role:button "Model"')
	expect(app:wait(".dropdown-panel", { timeout_ms = 5000 })).to_be(true)
	app:key("Escape")
	app:frames(30)
end

test("the page takes clicks: a new chat, a chat alone, and panes side by side", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	page_takes_clicks(app)

	open(app, "Fix the build")
	page_takes_clicks(app)

	app:set_state("inset_split_trees", { { kind = "row", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(30)
	page_takes_clicks(app)
	-- (Each pane, under the same layers, takes the focus at a click: the composer
	-- goes to it.)
	app:click("#split-pane-a1")
	app:frames(30)
	local at_a1 = box(app, "#chat-composer-dock").x
	app:click("#split-pane-a2")
	app:frames(30)
	expect(box(app, "#chat-composer-dock").x ~= at_a1).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("settings float over the chats: the tabs stay where they were under its scrim, and its × closes it", function()
	local app = all_open()
	local tab = box(app, "#session-tab-b2")
	local path = app:state("shell_route_path")
	app:command("shell.settings")
	app:frames(30)
	expect(app:has("#session-tab-b2")).to_be(true)
	expect(box(app, "#session-tab-b2").x).to_be(tab.x)
	-- Claude's size: 1020 wide at most, centred, 48 from the window's head at 800 tall.
	local settings = box(app, "#shell-settings")
	expect(`{settings.x},{settings.y},{settings.w}`).to_be("90,48,1020")
	-- The tab, under the scrim: a click there is the scrim's.
	local ok, why = pcall(app.click, app, "#session-tab-b2")
	expect(ok).to_be(false)
	expect(tostring(why)).to_contain("#settings-scrim")
	app:click("#settings-close")
	expect(app:wait("#shell-settings", { gone = true, timeout_ms = 1000 })).to_be(true)
	expect(app:state("shell_route_path")).to_be(path)
	expect(app:error()).to_be_nil()
end)

test("a new chat's page has a panel, for the project its composer has chosen", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	expect(box(app, "#shell-files").w > 300).to_be(true)
	expect(app:wait("#panel-open-terminal", { timeout_ms = 5000 })).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("with no project chosen, a new chat's panel still has what needs no folder", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	app:set_state("chat_project", "none")
	app:frames(30)
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	expect(app:wait("#panel-open-terminal", { timeout_ms = 5000 })).to_be(true)
	expect(app:has("#panel-open-browser")).to_be(true)
	expect(app:has("#panel-open-files")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("an open panel stays as it was under settings", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	local panel = box(app, "#shell-files")
	app:command("shell.settings")
	app:frames(60)
	local under = box(app, "#shell-files")
	expect(`{under.x},{under.w}`).to_be(`{panel.x},{panel.w}`)
	app:command("shell.settings")
	expect(app:wait("#shell-settings", { gone = true, timeout_ms = 1000 })).to_be(true)
	local after = box(app, "#shell-files")
	expect(`{after.x},{after.w}`).to_be(`{panel.x},{panel.w}`)
	expect(app:error()).to_be_nil()
end)

test("typing a trigger in a split opens one menu, not one for the hidden composer too", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	app:set_state("inset_split_trees", { { kind = "row", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(30)
	app:type('#chat-composer-dock >> role:textbox "Message"', "/")
	app:frames(30)
	-- (Counted by its id: the whole window's tree stops at a budget the rail's marks pass.)
	expect(app:count("#mention-panel")).to_be(1)
	expect(app:error()).to_be_nil()
end)

test("each tab opened is in view, with the panel open and the strip narrow", function()
	local app = all_open()
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	-- (By the keys, Ctrl held through Ctrl+Tab's cards to the one wanted: with two
	-- panes and the panel, the sidebar has stepped aside.)
	local function go(id: string)
		app:hold("ctrl")
		app:frames(3)
		for _ = 1, 12 do
			if app:state("shell_switcher_cards") == nil then
				app:command("shell.next_session")
			elseif (app:state("shell_switcher_cards") :: any)[app:state("shell_switcher_hi") :: number] == id then
				app:release()
				app:frames(30)
				expect(tostring(app:state("shell_route_path")):find(id, 1, true) ~= nil).to_be(true)
				return
			else
				app:command("shell.next_session")
			end
			app:frames(10)
		end
		error(`never reached {id}`)
	end
	for i, id in IDS do
		if i < #IDS then
			go(id)
			in_view(app, id)
		end
	end
	-- Back along them, each hidden in turn by the one before.
	for i = #IDS, 1, -1 do
		go(IDS[i])
		in_view(app, IDS[i])
	end
	expect(app:error()).to_be_nil()
end)

test("the panel opening keeps the tab showing in view as the strip narrows", function()
	local app = all_open()
	in_view(app, "b2")
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	in_view(app, "b2")
	expect(app:error()).to_be_nil()
end)

test("a new chat's page stays a card with the sidebar closed", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	local before = box(app, "#shell-page")
	app:click('#titlebar >> "Toggle sidebar"')
	-- (No flat page on the way: the same set in throughout its slide.)
	for _ = 1, 8 do
		app:frames(5)
		expect(box(app, "#shell-page").y).to_be(before.y)
	end
	app:frames(30)
	expect(box(app, "#shell-page").x > 0).to_be(true)
	expect(box(app, "#shell-page").y).to_be(before.y)
	expect(app:error()).to_be_nil()
end)

-- The tabs as they stand, by chat.
local function tabs_shown(app: any): { [string]: boolean }
	local out: { [string]: boolean } = {}
	for id in app:tree("#session-tabs"):gmatch("#session%-tab%-([%w_]+)") do
		out[id] = true
	end
	return out
end

local function route_has(app: any, id: string): boolean
	return tostring(app:state("shell_route_path")):find(id, 1, true) ~= nil
end

local function choose(app: any, tab: string, item: string)
	app:right_click(`#session-tab-{tab}`)
	expect(app:wait(`role:menuitem "{item}"`, { timeout_ms = 5000 })).to_be(true)
	app:click(`role:menuitem "{item}"`)
	app:frames(30)
end

test("a chat tab's menu has the panel tabs' choices", function()
	local app = all_open()
	app:right_click("#session-tab-a2")
	for _, item in { "Close tab", "Close others", "Close to the left", "Close to the right" } do
		expect(app:wait(`role:menuitem "{item}"`, { timeout_ms = 5000 })).to_be(true)
	end
	expect(app:error()).to_be_nil()
end)

test("Close tab closes that tab, and the chat showing stays if it was another", function()
	local app = all_open()
	choose(app, "a2", "Close tab")
	local now = tabs_shown(app)
	expect(now.a2).to_be_nil()
	expect(now.a1).to_be(true)
	expect(tostring(app:state("shell_route_path")):find("b2", 1, true) ~= nil).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("Close others leaves that tab, and goes to it when the chat showing was closed", function()
	local app = all_open()
	choose(app, "a2", "Close others")
	local now = tabs_shown(app)
	expect(now.a2).to_be(true)
	for _, id in { "a1", "a3", "b1", "b2" } do
		expect(now[id]).to_be_nil()
	end
	expect(tostring(app:state("shell_route_path")):find("a2", 1, true) ~= nil).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("Close to the left and to the right close only that side", function()
	local app = all_open()
	-- (b2 shows, to the right: left of a3 goes, and the chat showing stays.)
	choose(app, "a3", "Close to the left")
	local now = tabs_shown(app)
	expect(now.a1).to_be_nil()
	expect(now.a2).to_be_nil()
	expect(now.a3).to_be(true)
	expect(now.b2).to_be(true)
	expect(tostring(app:state("shell_route_path")):find("b2", 1, true) ~= nil).to_be(true)
	-- (Right of a3 takes the chat showing with it: a3 opens.)
	choose(app, "a3", "Close to the right")
	now = tabs_shown(app)
	expect(now.a3).to_be(true)
	expect(now.b1).to_be_nil()
	expect(now.b2).to_be_nil()
	expect(tostring(app:state("shell_route_path")):find("a3", 1, true) ~= nil).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("a tab still opens its chat on a click, and reorders by a drag, with its menu on", function()
	local app = all_open()
	app:click("#session-tab-a1")
	app:frames(20)
	expect(tostring(app:state("shell_route_path")):find("a1", 1, true) ~= nil).to_be(true)
	expect(box(app, "#session-tab-a1").x < box(app, "#session-tab-a2").x).to_be(true)
	-- (It takes the slot under the pointer, those between closing up.)
	app:drag("#session-tab-a1", "#session-tab-a3")
	app:frames(40)
	expect(box(app, "#session-tab-a2").x < box(app, "#session-tab-a3").x).to_be(true)
	expect(box(app, "#session-tab-a3").x < box(app, "#session-tab-a1").x).to_be(true)
	expect(app:has("#session-tabs-band")).to_be(false)
	expect(app:error()).to_be_nil()
end)

-- Those chats split side by side, the focus on the first.
local function split(app: any, chats: { string })
	local children = {}
	for _, id in chats do
		table.insert(children, { kind = "leaf", id = id })
	end
	app:set_state("inset_split_trees", { { kind = "row", children = children } })
	app:frames(30)
end

local function letter(app: any, tab: string, at: number): string?
	local line = (app:tree(`#session-tab-{tab}-tile-{at}`) :: string):match("[^\n]+") or ""
	return line:match('"(.-)"')
end

test("each chat opened keeps its tab, in the order opened; opened again, it goes to that tab", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	open(app, "Write the docs")
	open(app, "Ship the release")
	local now = tabs_shown(app)
	expect(now.a1).to_be(true)
	expect(now.a2).to_be(true)
	expect(now.b1).to_be(true)
	expect(box(app, "#session-tab-a1").x < box(app, "#session-tab-a2").x).to_be(true)
	expect(box(app, "#session-tab-a2").x < box(app, "#session-tab-b1").x).to_be(true)
	open(app, "Fix the build")
	expect(#(app:state("inset_split_trees") :: any)).to_be(3)
	expect(route_has(app, "a1")).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("closing a tab goes to the next, else the one before, the last to a new chat; never the window", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	open(app, "Write the docs")
	open(app, "Ship the release")
	open(app, "Write the docs")
	app:command("shell.close")
	app:frames(40)
	expect(route_has(app, "b1")).to_be(true)
	app:command("shell.close")
	app:frames(40)
	expect(route_has(app, "a1")).to_be(true)
	app:command("shell.close")
	app:frames(40)
	expect(app:state("shell_route_path")).to_be("/")
	expect(#(app:state("inset_split_trees") :: any)).to_be(0)
	expect(app:has("#session-tab-draft")).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("a split's pane closed leaves the tab with the other chat", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	split(app, { "a1", "a2" })
	app:command("shell.close")
	app:frames(40)
	expect(#(app:state("inset_split_trees") :: any)).to_be(1)
	expect(app:state("shell_route_path") ~= "/").to_be(true)
	expect(app:error()).to_be_nil()
end)

test("the new chat's page is a tab of its own, last; closed, it goes to the tab before", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	expect(app:has("#session-tab-draft")).to_be(true)
	open(app, "Fix the build")
	app:frames(30)
	expect(app:has("#session-tab-draft")).to_be(false)
	expect(tabs_shown(app).a1).to_be(true)
	app:click("#rail-new-chat")
	app:frames(30)
	expect(box(app, "#session-tab-a1").x < box(app, "#session-tab-draft").x).to_be(true)
	expect(tabs_shown(app).a1).to_be(true)
	app:command("shell.close")
	app:frames(40)
	expect(route_has(app, "a1")).to_be(true)
	expect(app:has("#session-tab-draft")).to_be(false)
	expect(app:error()).to_be_nil()
end)

local function alone_open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	for _, title in CHATS do
		open(app, title)
	end
	return app
end

local function tab_order(app: any): { string }
	local out: { string } = {}
	for id in app:tree("#session-tabs"):gmatch("#session%-tab%-([%w_]+) tab") do
		table.insert(out, id)
	end
	return out
end

test("a tab pinned is a tile at the head of the strip, in the order pinned; unpinned, it leads the rest", function()
	local app = alone_open()
	choose(app, "a3", "Pin tab")
	expect(tab_order(app)[1]).to_be("a3")
	expect(box(app, "#session-tab-a3").w).to_be(36)
	choose(app, "b2", "Pin tab")
	local now = tab_order(app)
	expect(now[1]).to_be("a3")
	expect(now[2]).to_be("b2")
	expect(box(app, "#session-tab-b2").x > box(app, "#session-tab-a3").x).to_be(true)
	app:hover("#session-tab-a3")
	app:frames(20)
	app:click("#session-tab-a3-unpin")
	app:frames(30)
	now = tab_order(app)
	expect(now[1]).to_be("b2")
	expect(now[2]).to_be("a3")
	expect(box(app, "#session-tab-a3").w > 36).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("a pinned tab stays till unpinned: Close others, a middle click and ⌘W leave it", function()
	local app = alone_open()
	choose(app, "a2", "Pin tab")
	choose(app, "b1", "Close others")
	local now = tabs_shown(app)
	expect(now.a2).to_be(true)
	expect(now.b1).to_be(true)
	for _, id in { "a1", "a3", "b2" } do
		expect(now[id]).to_be_nil()
	end
	app:click("#session-tab-a2")
	app:frames(30)
	expect(route_has(app, "a2")).to_be(true)
	app:command("shell.close")
	app:frames(30)
	expect(tabs_shown(app).a2).to_be(true)
	expect(route_has(app, "a2")).to_be(true)
	choose(app, "a2", "Unpin tab")
	app:command("shell.close")
	app:frames(30)
	expect(tabs_shown(app).a2).to_be_nil()
	expect(app:error()).to_be_nil()
end)

test("under the pointer a tab has Pin tab between Settle and Close, a press pinning it", function()
	local app = alone_open()
	app:hover("#session-tab-a2")
	app:frames(20)
	local pin, settle = box(app, "#session-tab-a2-pin"), box(app, "#session-tab-a2-settle")
	local close = box(app, '#session-tab-a2 >> role:button "Close tab"')
	expect(pin.x >= settle.x + settle.w).to_be(true)
	expect(close.x >= pin.x + pin.w).to_be(true)
	local full = box(app, "#session-tab-a2").w
	app:click("#session-tab-a2-pin")
	app:frames(4)
	local folding = box(app, "#session-tab-a2").w
	expect(folding > 36 and folding < full).to_be(true)
	app:frames(30)
	expect(tab_order(app)[1]).to_be("a2")
	expect(box(app, "#session-tab-a2").w).to_be(36)
	expect(app:error()).to_be_nil()
end)

test("a split's tab can't be pinned", function()
	local app = all_open()
	app:right_click("#session-tab-a2")
	expect(app:wait('role:menuitem "Close tab"', { timeout_ms = 5000 })).to_be(true)
	expect(app:has('role:menuitem "Pin tab"')).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("Close idle tabs closes the tabs with nothing going on, not the one showing", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	-- (Fix the build is at work; Ship the release awaits you; the rest, neither.)
	open(app, "Fix the build")
	open(app, "Write the docs")
	open(app, "Tidy the tests")
	open(app, "Ship the release")
	choose(app, "a3", "Close idle tabs")
	local now = tabs_shown(app)
	expect(now.a1).to_be(true)
	expect(now.b1).to_be(true)
	expect(now.a2).to_be_nil()
	expect(now.a3).to_be_nil()
	expect(route_has(app, "b1")).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("a tab's panes stack, three at most, the pane focused at the far left", function()
	local app = all_open()
	split(app, { "a1", "b1" })
	expect(app:wait("#session-tab-a1-tile-2", { timeout_ms = 2000 })).to_be(true)
	expect(app:wait("#session-tab-a1-tile-3", { timeout_ms = 200 })).to_be(false)

	split(app, { "a1", "b1", "a2", "a3" })
	expect(app:wait("#session-tab-a1-tile-3", { timeout_ms = 2000 })).to_be(true)
	expect(app:wait("#session-tab-a1-tile-4", { timeout_ms = 200 })).to_be(false)
	app:frames(30)
	local front, middle, back = box(app, "#session-tab-a1-tile-1"), box(app, "#session-tab-a1-tile-2"), box(app, "#session-tab-a1-tile-3")
	-- (The pane focused at the far left, the others behind it to the right.)
	expect(middle.x > front.x).to_be(true)
	expect(back.x > middle.x).to_be(true)
	expect(front.x < box(app, "#session-tab-a1").x + 12).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("the pane focused comes to the front of the stack, the one before it behind it", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	open(app, "Ship the release")
	split(app, { "a1", "b1" })
	open(app, "Fix the build")
	app:frames(40)
	expect(letter(app, "a1", 1)).to_be("A")
	expect(letter(app, "a1", 2)).to_be("B")
	open(app, "Ship the release")
	app:frames(40)
	expect(letter(app, "a1", 1)).to_be("B")
	expect(letter(app, "a1", 2)).to_be("A")
	expect(app:error()).to_be_nil()
end)

local function panes_of(app: any, key: string): { string }
	for _, t in app:state("inset_split_trees") :: any do
		if (t.key or "") == key or (not t.key and (t.id == key)) then
			local out: { string } = {}
			local function walk(n: any)
				if n.kind == "leaf" then
					table.insert(out, n.id)
				else
					for _, c in n.children do
						walk(c)
					end
				end
			end
			walk(t)
			return out
		end
	end
	return {}
end

-- A chat alone in a tab, and two in another.
local function two_tabs(app: any)
	app:set_state("inset_split_trees", {
		{ kind = "leaf", id = "a1", key = "solo" },
		{ kind = "row", key = "pair", children = { { kind = "leaf", id = "a2" }, { kind = "leaf", id = "a3" } } },
		{ kind = "leaf", id = "b1", key = "other" },
	})
	app:frames(30)
end

test("a chat kept in two tabs stays in the first", function()
	local app = all_open()
	app:set_state("inset_split_trees", {
		{ kind = "row", key = "first", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } },
		{ kind = "row", key = "both", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a3" }, { kind = "leaf", id = "b1" } } },
	})
	app:frames(30)
	expect(table.concat(panes_of(app, "first"), ",")).to_be("a1,a2")
	expect(table.concat(panes_of(app, "both"), ",")).to_be("a3,b1")
	expect(app:error()).to_be_nil()
end)

test("a closed split's tab stays closed: a chat of its opened again has a tab of its own", function()
	local app = all_open()
	app:set_state("inset_split_trees", {
		{ kind = "row", key = "both", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "b1" } } },
		{ kind = "leaf", id = "a2", key = "other" },
	})
	app:frames(30)
	open(app, "Write the docs")
	choose(app, "both", "Close tab")
	expect(tabs_shown(app).both).to_be_nil()
	open(app, "Fix the build")
	app:frames(30)
	expect(tabs_shown(app).both).to_be_nil()
	local kept = app:state("inset_split_trees") :: any
	expect(#kept).to_be(2)
	expect(kept[2].kind).to_be("leaf")
	expect(kept[2].id).to_be("a1")
	-- Nor is the other chat kept anywhere.
	expect(app:has("#split-pane-b1")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("tabs kept from before keep their order, a split where its first chat stood", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	app:set_state("inset_split_trees", { { kind = "row", children = { { kind = "leaf", id = "a2" }, { kind = "leaf", id = "a3" } } } })
	app:set_state("shell_session_tabs", { "a1", "a2", "b1", "a3" })
	app:frames(30)
	local kept = app:state("inset_split_trees") :: any
	expect(#kept).to_be(3)
	expect(kept[1].id).to_be("a1")
	expect(kept[2].kind).to_be("row")
	expect(kept[3].id).to_be("b1")
	expect(#(app:state("shell_session_tabs") :: any)).to_be(0)
	expect(app:error()).to_be_nil()
end)

local function has(app: any, target: string, ms: number?): boolean
	return app:wait(target, { timeout_ms = ms or 2000 })
end

test("a tab's corner says a chat awaits you over one at work, and its × comes under the pointer", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	-- (Fix the build is at work; Ship the release awaits you; Write the docs, neither.)
	split(app, { "a1", "a2" })
	expect(has(app, "#session-tab-a1-state")).to_be(true)
	expect(app:tree("#session-tab-a1-state"):find("Working", 1, true) ~= nil).to_be(true)

	-- In a split with one awaiting you, that's what it says.
	split(app, { "a1", "b1" })
	expect(has(app, "#session-tab-a1-state")).to_be(true)
	local both = app:tree("#session-tab-a1-state") :: string
	expect(both:find("Working", 1, true)).to_be_nil()

	app:hover("#session-tab-a1")
	app:frames(10)
	expect(has(app, '#session-tab-a1 >> role:button "Close tab"')).to_be(true)
	expect(has(app, "#session-tab-a1-state", 200)).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("a chat opened with no tab gets one at the end", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	open(app, "Write the docs")
	open(app, "Tidy the tests")
	local kept = app:state("inset_split_trees") :: any
	expect(#kept).to_be(3)
	expect(kept[3].id).to_be("a3")
	expect(app:error()).to_be_nil()
end)

test("a chat opened from a split gets a new tab, the split left as it is", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	app:set_state("inset_split_trees", { { kind = "row", key = "pair", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(30)
	open(app, "Ship the release")
	app:frames(30)
	expect(table.concat(panes_of(app, "pair"), ",")).to_be("a1,a2")
	local kept = app:state("inset_split_trees") :: any
	expect(#kept).to_be(2)
	expect(kept[2].id).to_be("b1")
	expect(app:error()).to_be_nil()
end)

test("a chat in another tab goes to that tab, replacing nothing", function()
	local app = all_open()
	open(app, "Fix the build")
	app:frames(30)
	expect(#(app:state("inset_split_trees") :: any)).to_be(5)
	expect(route_has(app, "a1")).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("from a new chat's page a chat opened gets a tab, the others kept", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	app:click("#rail-new-chat")
	app:frames(30)
	open(app, "Write the docs")
	app:frames(30)
	local kept = app:state("inset_split_trees") :: any
	expect(#kept).to_be(2)
	expect(kept[2].id).to_be("a2")
	expect(app:error()).to_be_nil()
end)

test("a split turned from a row to a column has a divider that goes up and down", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	app:set_state("inset_split_trees", { { kind = "row", key = "p", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(30)
	expect(app:wait("#split-divider-row:/1", { timeout_ms = 2000 })).to_be(true)
	app:set_state("inset_split_trees", { { kind = "col", key = "p", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(30)
	expect(app:wait("#split-divider-col:/1", { timeout_ms = 2000 })).to_be(true)
	expect(app:wait("#split-divider-row:/1", { timeout_ms = 200 })).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("a chat starting a run gets its tab back; closed, it stays closed till the next", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Write the docs")
	expect(tabs_shown(app).a3).to_be_nil()
	app:command("start_a3")
	app:frames(30)
	expect(tabs_shown(app).a3).to_be(true)
	expect(route_has(app, "a2")).to_be(true)
	choose(app, "a3", "Close tab")
	expect(tabs_shown(app).a3).to_be_nil()
	-- (Still at it, and another chat moving: closed.)
	app:command("finish")
	app:frames(30)
	expect(tabs_shown(app).a3).to_be_nil()
	app:command("finish_a3")
	app:frames(30)
	expect(tabs_shown(app).a3).to_be_nil()
	app:command("start_a3")
	app:frames(30)
	expect(tabs_shown(app).a3).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("a chat in its own work tree wears how far ahead it is, or that it's in", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	expect(app:has("#session-tab-a1-tree")).to_be(false)
	app:command("tree_ahead")
	app:frames(10)
	expect(app:tree("#session-tab-a1-tree"):find("3 commits not in fix/build", 1, true) ~= nil).to_be(true)
	app:command("tree_merged")
	app:frames(10)
	expect(app:tree("#session-tab-a1-tree"):find("In fix/build, 1 uncommitted file", 1, true) ~= nil).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("the panel's seam double-clicked goes back to the columns split equally", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	local function width(of: string): number
		local line = app:tree(of):match("[^\n]+") :: string
		return tonumber(line:match(" ([%d%.]+)x[%d%.]+")) :: number
	end
	app:click('#titlebar >> role:button "Panel"')
	app:frames(40)
	local panel = width("#shell-files")
	app:set_state("shell_files_width", 820)
	app:frames(40)
	expect(width("#shell-files") ~= panel).to_be(true)
	app:double_click('role:slider "Resize files"')
	app:frames(40)
	expect(width("#shell-files")).to_be(panel)
	expect(app:error()).to_be_nil()
end)

local function focused_in(app: any, target: string): boolean
	return app:tree(target):find(" focused", 1, true) ~= nil
end

test("a step back lands in the composer", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	open(app, "Write the docs")
	app:click('#titlebar >> role:button "Back"')
	app:frames(30)
	expect(route_has(app, "a1")).to_be(true)
	expect(focused_in(app, "#chat-composer")).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("leaving settings puts the keyboard back in the composer", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	app:command("shell.settings")
	app:frames(20)
	app:key("escape")
	app:frames(30)
	expect(focused_in(app, "#chat-composer")).to_be(true)
	expect(app:error()).to_be_nil()
end)

local function tab_ids(app: any): { string }
	local out: { string } = {}
	for _, t in app:state("inset_split_trees") :: any do
		table.insert(out, t.key or t.id or "")
	end
	return out
end
local function shell_with(titles: { string }): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	for _, title in titles do
		open(app, title)
	end
	return app
end

test("chats heard before the live states don't reopen a closed tab on a launch", function()
	local app = shell_with({ "Fix the build", "Tidy the tests" })
	app:command("start_a3")
	app:frames(5)
	app:click('#session-tab-a1')
	app:frames(20)
	choose(app, "a3", "Close tab")
	app:frames(20)
	expect(app:has("#session-tab-a3")).to_be(false)
	-- As a launch: nothing seen yet, the chats first, their live states after.
	app:set_state("shell_tabs_statuses", {})
	app:command("chats_before_live")
	app:frames(5)
	app:command("live_arrives")
	app:frames(20)
	expect(app:has("#session-tab-a3")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("a run gone quiet and back doesn't reopen a closed tab; a run after it does", function()
	local app = shell_with({ "Fix the build", "Tidy the tests" })
	app:command("start_a3")
	app:frames(5)
	app:click('#session-tab-a1')
	app:frames(20)
	choose(app, "a3", "Close tab")
	app:frames(20)
	app:command("stale_a3")
	app:frames(5)
	app:command("start_a3")
	app:frames(20)
	expect(app:has("#session-tab-a3")).to_be(false)
	app:command("finish_a3")
	app:frames(5)
	app:command("start_a3")
	app:frames(20)
	expect(app:has("#session-tab-a3")).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("the chat showing deleted elsewhere: its tab goes, a new chat in its place, Back never lands on it", function()
	local app = shell_with({ "Fix the build", "Write the docs" })
	app:command("gone_a2")
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/")
	expect(app:has("#session-tab-a2")).to_be(false)
	for _ = 1, 3 do
		if app:state("shell_route_path"):find("a2", 1, true) then
			break
		end
		app:click('#titlebar >> role:button "Back"')
		app:frames(10)
	end
	expect(app:state("shell_route_path"):find("a2", 1, true)).to_be_nil()
	expect(app:error()).to_be_nil()
end)

test("a chat behind in the history deleted elsewhere: a step back passes it", function()
	local app = shell_with({ "Fix the build", "Write the docs" })
	app:command("gone_a1")
	app:frames(20)
	expect(#(app:state("shell_route_history") :: any).entries).to_be(2)
	app:click('#titlebar >> role:button "Back"')
	app:frames(20)
	expect(app:state("shell_route_path")).to_be("/")
	expect(app:error()).to_be_nil()
end)

test("a split's chat deleted: the tab keeps the rest, alone as a chat's own", function()
	local app = shell_with({ "Fix the build" })
	app:set_state("inset_split_trees", { { kind = "row", key = "pair", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(20)
	app:command("gone_a2")
	app:frames(20)
	local kept = app:state("inset_split_trees") :: any
	expect(#kept).to_be(1)
	expect(kept[1].kind).to_be("leaf")
	expect(kept[1].id).to_be("a1")
	expect(app:error()).to_be_nil()
end)

test("a split with all its chats deleted leaves the store", function()
	local app = shell_with({ "Fix the build" })
	app:set_state("inset_split_trees", { { kind = "leaf", id = "a1" }, { kind = "row", key = "both", children = { { kind = "leaf", id = "a2" }, { kind = "leaf", id = "a3" } } } })
	app:frames(20)
	app:command("gone_a2")
	app:command("gone_a3")
	app:frames(20)
	expect(#tab_ids(app)).to_be(1)
	expect(app:error()).to_be_nil()
end)

test("dragging a divider after a deleted chat changed the tree's shape works", function()
	local app = shell_with({ "Write the docs" })
	app:set_state("inset_split_trees", { { kind = "row", key = "t", children = {
		{ kind = "leaf", id = "a1" },
		{ kind = "col", children = { { kind = "leaf", id = "a2" }, { kind = "leaf", id = "a3" }, { kind = "leaf", id = "b1" } } },
	} } })
	app:frames(20)
	app:command("gone_a1")
	app:frames(20)
	app:drag("#split-divider-col:/2", "#split-pane-a2")
	app:frames(10)
	expect(app:error()).to_be_nil()
end)

test("a split stays as it was under Settings, its panes' cards and no page card", function()
	local app = shell_with({ "Write the docs" })
	app:set_state("inset_split_trees", { { kind = "row", key = "t", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
	app:frames(20)
	expect(app:tree("#shell-page-fill")).to_contain("hidden")
	app:command("shell.settings")
	app:frames(20)
	expect(app:has("#shell-settings")).to_be(true)
	expect(app:tree("#shell-page-fill")).to_contain("hidden")
end)

test("the tabs give the panel its room as it glides open, and take it back as it closes", function()
	local app = shell_with({ "Write the docs" })
	local full = box(app, "#session-tabs-scroll").w
	app:command("shell.toggle_files")
	app:frames(3)
	local early = box(app, "#session-tabs-scroll").w
	expect(early < full).to_be(true)
	app:frames(60)
	local open_w = box(app, "#session-tabs-scroll").w
	expect(open_w < early).to_be(true)
	expect(box(app, "#session-tabs-scroll").x + open_w <= box(app, "#shell-files").x).to_be(true)
	app:command("shell.toggle_files")
	app:frames(3)
	local closing = box(app, "#session-tabs-scroll").w
	expect(closing > open_w and closing < full).to_be(true)
	app:frames(60)
	expect(box(app, "#session-tabs-scroll").w).to_be(full)
	expect(app:error()).to_be_nil()
end)

test("a plain navigation to where Forward would go plays no slide", function()
	local app = shell_with({ "Fix the build", "Write the docs", "Fix the build" })
	app:click('#titlebar >> role:button "Back"')
	app:frames(30)
	local seq = (app:state("shell_route_stepped") :: any).seq
	open(app, "Fix the build")
	expect((app:state("shell_route_stepped") :: any).seq).to_be(seq)
	local h = app:state("shell_route_history") :: any
	expect(h.at).to_be(#h.entries)
	-- A real step forward does.
	app:click('#titlebar >> role:button "Back"')
	app:frames(30)
	app:click('#titlebar >> role:button "Forward"')
	app:frames(30)
	expect((app:state("shell_route_stepped") :: any).seq).to_be(seq + 2)
	expect(app:error()).to_be_nil()
end)

test("a tab carried into the page lands beside the chat there, as a sidebar row does", function()
	local app = mount("shell_fixture.luau")
	app:resize(1400, 800)
	app:frames(30)
	open(app, "Fix the build")
	open(app, "Write the docs")
	-- (A test's drag runs no frame between press and release: the landing a carried tab puts up is put up first.)
	app:set_state("split.dragging", true)
	app:frames(10)
	app:drag("#session-tab-a1", "#split-zone-a2-right")
	app:frames(40)
	local kept = app:state("inset_split_trees") :: any
	local pair = nil
	for _, t in kept do
		if t.kind == "row" then
			pair = t
		end
	end
	expect(pair ~= nil).to_be(true)
	expect(pair.children[1].id).to_be("a2")
	expect(pair.children[2].id).to_be("a1")
	expect(app:state("split.dragging")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("a chat's own agents fold into its mark on its tab and its row; another maker's still fan out", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	open(app, "Fix the build")
	open(app, "Write the docs")
	app:command("agents_at_work")
	app:frames(30)
	expect(app:has("#session-tab-a1")).to_be(true)
	expect(app:has("#session-tab-a1-agents")).to_be(false)
	expect(app:has("#sidebar-fan-a1")).to_be(false)
	expect(app:has("#session-tab-a2-agents")).to_be(true)
	expect(app:has("#sidebar-fan-a2")).to_be(true)
	expect(app:error()).to_be(nil)
end)
