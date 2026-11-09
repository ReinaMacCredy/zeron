-- Agent pages (`html_render`) and the options picked on them (DECISIONS 2026-11-06), on the fixture's pages.

local function settle(app: any)
	app:frames(30)
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

-- (A closed sheet stays in the tree, hidden, as zgx keeps a modal's content.)
local function sheet_open(app: any): boolean
	if not app:has("#html-sheet") then
		return false
	end
	local first = string.match(app:tree("#html-sheet"), "^[^\n]*") or ""
	return string.find(first, " hidden", 1, true) == nil
end

local FRAME = "p1-tc1-html"
local PICTURES = "p2-tc2-html"

local function seeded(app: any, which: string?)
	app:resize(960, 900)
	app:command("go_s2")
	settle(app)
	app:command(which or "pages_seed")
	settle(app)
end

-- Signed as the fixture's bootstrap signs: picks `p:<path>`, the rest `n:<path>`.
local function post_to(app: any, view: string, card: string, value: any)
	local path = if card == FRAME then "/u/html/s2/frame.html" else "/u/html/s2/pictures.html"
	local v = table.clone(value)
	v.kind = v.kind or "zeron.page"
	if v.nonce == nil then
		v.nonce = if v.pick ~= nil then `p:{path}` else `n:{path}`
	end
	app:set_state("page_message", { id = view, value = v })
	app:command("page_post")
	settle(app)
end

local function post(app: any, card: string, value: any)
	post_to(app, `html-page-{card}`, card, value)
end

local GROUPS = { { code = "A", name = "Góc trang" }, { code = "B", name = "Thanh tab" }, { code = "C", name = "Rail" } }
local LABELS = { A1 = "Bo góc thường", A2 = "Nổi sát đỉnh", B1 = "Có viền", B2 = "Không viền", C1 = "Ảnh sát mép", C2 = "Cùng tông" }

local function pick(app: any, code: string)
	post(app, FRAME, { pick = { group = string.sub(code, 1, 1), code = code, label = LABELS[code], groups = GROUPS, rect = { 10, 10, 120, 34 }, covered = false } })
end

local function send(app: any, words: string)
	if words ~= "" then
		app:type('role:textbox "Message"', words)
	else
		app:click('role:textbox "Message"')
	end
	app:key("Enter")
	app:wait('"never shown, only waited past"', { timeout_ms = 600 })
	settle(app)
end

local function last_run(app: any, card: string): string
	local runs = app:state("page_runs") or {}
	for i = #runs, 1, -1 do
		local id, js = string.match(runs[i], "^([^|]*)|(.*)$")
		if id == `html:{card}` and js and string.sub(js, 1, 1) == "{" then
			return js
		end
	end
	return ""
end

test("pages: a card shows its globe, title and expand over the page, its frame as tall as the page reports, capped once it overflowed", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	local card = `#html-card-{FRAME}`
	expect(app:has(card)).to_be(true)
	expect(app:tree(card)).to_contain('"Khung app"')
	expect(app:has(`#html-card-expand-{FRAME}`)).to_be(true)
	local page = `#html-page-{FRAME}`
	-- Before the page says, the agent's height.
	expect((box_of(app, page, page) :: any).h).to_be(600)
	post(app, FRAME, { height = 420 })
	expect((box_of(app, page, page) :: any).h).to_be(420)
	-- Within the drift it fits; past it, the agent's height for good.
	post(app, FRAME, { height = 640 })
	expect((box_of(app, page, page) :: any).h).to_be(640)
	post(app, FRAME, { height = 900 })
	expect((box_of(app, page, page) :: any).h).to_be(600)
	post(app, FRAME, { height = 300 })
	expect((box_of(app, page, page) :: any).h).to_be(600)
	-- A page the device can't read says so in its frame.
	expect(app:tree("#html-card-p2-tc3-html")).to_contain("This page is not available on this device.")
end)

test("pages: a dev server's page has its label, Reload and the browser; down, it says so and Retry brings it back", function()
	local app = mount("chat_fixture.luau")
	app:set_state("page_reach_up", false)
	seeded(app, "pages_seed_live")
	local card = "#html-card-p3-tc4-html"
	expect(app:tree(card)).to_contain('"localhost:5391/app"')
	expect(app:tree(card)).to_contain("localhost:5391 is not responding")
	expect(app:has("#html-page-p3-tc4-html")).to_be(false)
	app:set_state("page_reach_up", true)
	app:click("#live-page-retry-p3-tc4-html")
	settle(app)
	expect(app:has("#html-page-p3-tc4-html")).to_be(true)
	app:click("#live-page-open-p3-tc4-html")
	settle(app)
	expect(app:state("calls")).to_contain("open_url:http://localhost:5391/app/?x=1")
	local before = select(2, string.gsub(app:state("calls"), "reach", ""))
	app:click("#live-page-reload-p3-tc4-html")
	settle(app)
	expect(select(2, string.gsub(app:state("calls"), "reach", ""))).to_be(before + 1)
end)

test("pages: expand opens the sheet with Live and Source and Save .html; Escape closes it", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	app:click(`#html-card-expand-{FRAME}`)
	settle(app)
	expect(sheet_open(app)).to_be(true)
	expect(app:tree("#html-sheet")).to_contain('"Khung app"')
	expect(app:has("#html-sheet-page")).to_be(true)
	app:click("#html-sheet-source")
	settle(app)
	expect(app:tree("#html-sheet-source-text")).to_contain("/u/html/s2/frame.html")
	app:click("#html-sheet-live")
	settle(app)
	expect(app:has("#html-sheet-page")).to_be(true)
	app:key("Escape")
	settle(app)
	expect(sheet_open(app)).to_be(false)
	-- (A click outside is the modal's own: the tests can't press its backdrop; on the Dev.)
	-- Closed on Source, it opens again on Live.
	app:click(`#html-card-expand-{FRAME}`)
	settle(app)
	app:click("#html-sheet-source")
	settle(app)
	app:key("Escape")
	settle(app)
	app:click(`#html-card-expand-{FRAME}`)
	settle(app)
	expect(sheet_open(app)).to_be(true)
	expect(app:has("#html-sheet-page")).to_be(true)
	expect(app:has("#html-sheet-source-text")).to_be(false)
end)

test("picks: a click fills its group's slot in the tray; another option swaps it in place; the chosen one again or its x clears it", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	expect(app:has("#composer-picks")).to_be(false)
	pick(app, "A2")
	expect(app:has("#composer-picks")).to_be(true)
	local tray = app:tree("#composer-picks")
	expect(tray).to_contain(LABELS.A2)
	-- The groups not yet picked keep their empty slots, by name.
	expect(tray).to_contain('"Thanh tab"')
	expect(tray).to_contain('"Rail"')
	-- The page's buttons follow: A2 chosen, the count.
	expect(last_run(app, FRAME)).to_contain('{"code":"A2","group":"A"}')
	expect(last_run(app, FRAME)).to_contain("1/3 chosen")
	local slot = box_of(app, "#composer-picks", `#pick-slot-{FRAME}-A`) :: any
	pick(app, "A1")
	tray = app:tree("#composer-picks")
	expect(tray).to_contain(LABELS.A1)
	expect(string.find(tray, LABELS.A2, 1, true)).to_be(nil)
	-- In place: the same slot.
	local again = box_of(app, "#composer-picks", `#pick-slot-{FRAME}-A`) :: any
	expect(again.x).to_be(slot.x)
	pick(app, "A1")
	expect(app:has("#composer-picks")).to_be(false)
	pick(app, "B2")
	app:click(`#pick-clear-{FRAME}-B`)
	settle(app)
	expect(app:has("#composer-picks")).to_be(false)
	-- Picking never sends nor takes the keys.
	expect(string.find(app:state("calls"), "send:", 1, true)).to_be(nil)
end)

test("picks: the prompt carries the note then each group's pick and what's left; sent groups lock, the rest stay open", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	pick(app, "A2")
	pick(app, "B2")
	send(app, "+ header của C2")
	expect(app:state("calls")).to_contain('send:s2:+ header của C2\n\nĐã chọn trong "Khung app":\nA = A2 (Nổi sát đỉnh)\nB = B2 (Không viền)\nChưa chọn: C (Rail)')
	expect(app:has("#composer-picks")).to_be(false)
	app:command("picks_echo")
	settle(app)
	-- The sent message carries its chips and its note.
	expect(app:tree("#transcript")).to_contain(LABELS.B2)
	expect(app:tree("#transcript")).to_contain("header của C2")
	expect(last_run(app, FRAME)).to_contain("2/3 sent")
	expect(last_run(app, FRAME)).to_contain('"locked":["A","B"]')
	-- A sent group's options do nothing; the one left is still open.
	pick(app, "A1")
	expect(app:has("#composer-picks")).to_be(false)
	pick(app, "C1")
	expect(app:tree("#composer-picks")).to_contain(LABELS.C1)
	expect(last_run(app, FRAME)).to_contain("2/3 sent · 1 chosen")
	-- Only the open group has a slot.
	expect(app:has(`#pick-slot-{FRAME}-A`)).to_be(false)
	send(app, "")
	expect(app:state("calls")).to_contain('send:s2:Đã chọn trong "Khung app":\nC = C1 (Ảnh sát mép)')
	app:command("picks_echo")
	settle(app)
	expect(last_run(app, FRAME)).to_contain('"count":"Sent"')
end)

test("picks: a page without groups adds free chips, several, sent as label lines", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	local function free(code: string, label: string)
		post(app, PICTURES, { pick = { group = "", code = code, label = label, groups = {} } })
	end
	free("p1", "Sea")
	free("p3", "Hill")
	free("p2", "Field")
	free("p2", "Field")
	expect(app:tree("#composer-picks")).to_contain('"Sea"')
	expect(string.find(app:tree("#composer-picks"), "Field", 1, true)).to_be(nil)
	send(app, "")
	expect(app:state("calls")).to_contain('send:s2:Đã chọn trong "Ảnh":\n- Sea\n- Hill')
end)

test("picks: six groups wrap the tray to two rows", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	local six = {}
	for _, g in { "A", "B", "C", "D", "E", "F" } do
		table.insert(six, { code = g, name = `Group {g}` })
	end
	post(app, FRAME, { pick = { group = "A", code = "A1", label = "One", groups = six } })
	local a = box_of(app, "#composer-picks", `#pick-slot-{FRAME}-A`) :: any
	local d = box_of(app, "#composer-picks", `#pick-slot-{FRAME}-D`) :: any
	local c = box_of(app, "#composer-picks", `#pick-slot-{FRAME}-C`) :: any
	expect(c.y).to_be(a.y)
	expect(d.y).to_be(a.y + a.h + 6)
end)

test("picks: unsent picks stay in the draft over a reload and the page is told them again", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	pick(app, "A2")
	app:command("remake_session")
	settle(app)
	expect(app:tree("#composer-picks")).to_contain(LABELS.A2)
	-- And over a switch to another chat and back.
	app:command("go_s1")
	settle(app)
	expect(app:has("#composer-picks")).to_be(false)
	app:command("go_s2")
	settle(app)
	expect(app:tree("#composer-picks")).to_contain(LABELS.A2)
	expect(last_run(app, FRAME)).to_contain('{"code":"A2","group":"A"}')
end)

test("pages: what a page posts without its pick nonce, from a frame, or asking to send or focus does nothing", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	-- Forged: no nonce, another's nonce, an iframe's.
	post(app, FRAME, { nonce = "", pick = { group = "A", code = "A2", label = "x", groups = GROUPS } })
	expect(app:has("#composer-picks")).to_be(false)
	post(app, FRAME, { nonce = "p:/u/html/s2/pictures.html", pick = { group = "B", code = "B2", label = "x", groups = GROUPS } })
	expect(app:has("#composer-picks")).to_be(false)
	app:set_state("page_message", { id = `html-page-{FRAME}`, main = false, value = { kind = "zeron.page", nonce = "p:/u/html/s2/frame.html", pick = { group = "C", code = "C2", label = "x", groups = GROUPS } } })
	app:command("page_post")
	settle(app)
	expect(app:has("#composer-picks")).to_be(false)
	-- The review's two leak routes (a setter on Object.prototype, a wrapped JSON.stringify) and their
	-- neighbours are refused in the page itself (html_pages `a_pages_own_script_never_reads_the_nonces…`,
	-- under node); what a leak of the height channel would still give, its nonce, places no pick.
	post(app, FRAME, { nonce = "n:/u/html/s2/frame.html", pick = { group = "A", code = "A1", label = "x", groups = GROUPS } })
	expect(app:has("#composer-picks")).to_be(false)
	-- Signed, asking the app to send, focus the composer or click its send.
	post(app, FRAME, { send = true, focus = true, click = "#composer-send", text = "go" })
	post(app, FRAME, { nonce = "p:/u/html/s2/frame.html", send = true, focus = true, click = "#composer-send", text = "go" })
	expect(string.find(app:state("calls"), "send:", 1, true)).to_be(nil)
	expect(app:tree('role:textbox "Message"')).never.to_contain(" focused")
	-- A `ready` that isn't a list, as a page's own toJSON could make it, is no error.
	post(app, FRAME, { ready = { pick = { group = "A", code = "A1" } } })
	expect(app:has("#composer-picks")).to_be(false)
	-- A signed pick only places it.
	pick(app, "A2")
	expect(app:has("#composer-picks")).to_be(true)
	expect(string.find(app:state("calls"), "send:", 1, true)).to_be(nil)
	expect(app:tree('role:textbox "Message"')).never.to_contain(" focused")
end)

test("pages: a page with a script before its bootstrap gets no pick nonce, and its picks place nothing", function()
	local app = mount("chat_fixture.luau")
	app:set_state("page_early_script", true)
	seeded(app)
	pick(app, "A2")
	expect(app:has("#composer-picks")).to_be(false)
	-- Its height still follows.
	post(app, FRAME, { height = 420 })
	local page = `#html-page-{FRAME}`
	expect((box_of(app, page, page) :: any).h).to_be(420)
end)

test("pages: the sheet's page never sizes the card, and gets the ticks, locks and theme the card has", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	local page = `#html-page-{FRAME}`
	post(app, FRAME, { height = 420 })
	expect((box_of(app, page, page) :: any).h).to_be(420)
	app:click(`#html-card-expand-{FRAME}`)
	settle(app)
	-- The sheet's copy, wider, measures shorter.
	post_to(app, "html-sheet-page", FRAME, { height = 250 })
	app:key("Escape")
	settle(app)
	expect((box_of(app, page, page) :: any).h).to_be(420)
	-- A pick on either page shows on both.
	post_to(app, "html-sheet-page", FRAME, { pick = { group = "A", code = "A2", label = LABELS.A2, groups = GROUPS } })
	expect(app:tree("#composer-picks")).to_contain(LABELS.A2)
	expect(last_run(app, `{FRAME}#sheet`)).to_contain('{"code":"A2","group":"A"}')
	expect(last_run(app, FRAME)).to_contain('{"code":"A2","group":"A"}')
	-- The theme switched while the sheet was closed: its page follows too.
	local runs = #(app:state("page_runs") or {})
	app:command("theme_light")
	settle(app)
	local after = app:state("page_runs") or {}
	local sheet_rethemed = false
	for i = runs + 1, #after do
		if after[i] == `html:{FRAME}#sheet|retheme()` then
			sheet_rethemed = true
		end
	end
	expect(sheet_rethemed).to_be(true)
end)

test("pages: a link leaving the page opens in the browser and the card keeps its page; pages load under about:blank", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	local loaded = app:state("page_loaded") or {}
	expect(loaded[1]).to_contain("|about:blank|")
	local before = #loaded
	local function nav(kind: string, url: string)
		app:set_state("page_nav", { id = `html-page-{FRAME}`, kind = kind, url = url })
		app:command("page_navigate")
		settle(app)
	end
	-- Its own document and its anchors stay.
	nav("started", "about:blank")
	nav("changed", "about:blank#top")
	expect(#(app:state("page_loaded") or {})).to_be(before)
	nav("started", "https://example.com/docs")
	expect(app:state("calls")).to_contain("open_url:https://example.com/docs")
	loaded = app:state("page_loaded") or {}
	expect(#loaded).to_be(before + 1)
	expect(loaded[#loaded]).to_be(`html:{FRAME}|about:blank|<p>/u/html/s2/frame.html</p>`)
	expect(app:has(`#html-page-{FRAME}`)).to_be(true)
	-- Its commit after its start is the same leaving.
	nav("committed", "https://example.com/docs")
	expect(#(app:state("page_loaded") or {})).to_be(before + 1)
	-- Not a web address: put back, never opened.
	nav("started", "file:///etc/hosts")
	expect(string.find(app:state("calls"), "open_url:file", 1, true)).to_be(nil)
end)

test("pages: each page's web is made once it is read, with its bootstrap for the app's own world, the sheet's too", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	local boots = app:state("page_boots") or {}
	expect(boots).to_contain(`html:{FRAME}|boot:/u/html/s2/frame.html`)
	expect(boots).to_contain(`html:{PICTURES}|boot:/u/html/s2/pictures.html`)
	app:click(`#html-card-expand-{FRAME}`)
	settle(app)
	expect(app:state("page_boots")).to_contain(`html:{FRAME}#sheet|boot:/u/html/s2/frame.html`)
end)

test("pages: a stored page decides each navigation first: its own document goes, a link opens the browser and stays, a script's goes nowhere", function()
	local app = mount("chat_fixture.luau")
	seeded(app)
	local function ask(kind: string, url: string, main_frame: boolean?): any
		app:set_state("page_ask", { web = `html:{FRAME}`, kind = kind, url = url, main_frame = main_frame })
		app:command("page_decide")
		return app:state("page_decided")
	end
	expect(ask("other", "about:blank")).to_be(true)
	expect(ask("link", "about:blank#top")).to_be(true)
	expect(ask("link", "https://example.com/docs")).to_be(false)
	expect(app:state("calls")).to_contain("open_url:https://example.com/docs")
	-- A script's `location.href`: refused, and never opened.
	expect(ask("other", "https://example.com/elsewhere")).to_be(false)
	expect(string.find(app:state("calls"), "open_url:https://example.com/elsewhere", 1, true)).to_be(nil)
	expect(ask("link", "file:///etc/hosts")).to_be(false)
	expect(string.find(app:state("calls"), "open_url:file", 1, true)).to_be(nil)
	-- A frame inside it may load, but not a file.
	expect(ask("other", "https://example.com/embed", false)).to_be(true)
	expect(ask("other", "file:///etc/hosts", false)).to_be(false)
end)

test("open in: a reply naming .html files shows Website cards: linked ones, found ones, a missing one disabled", function()
	local app = mount("chat_fixture.luau")
	seeded(app, "pages_seed_live")
	local cards = app:tree("#html-open-p3")
	expect(cards).to_contain('"Demo page"')
	expect(cards).to_contain("Website · out/demo.html")
	-- In inline code and not on disk: no card.
	expect(string.find(cards, "other.html", 1, true)).to_be(nil)
	-- Linked and gone: disabled, says so.
	expect(cards).to_contain("Website · no longer on disk")
	expect(app:tree("#html-card-open-p3-3")).to_contain("disabled")
	app:click("#html-card-chevron-p3-1")
	settle(app)
	expect(app:has('role:menuitem "Zeron"')).to_be(true)
	expect(app:has('role:menuitem "Default browser"')).to_be(true)
	expect(app:has('role:menuitem "Reveal in Finder"')).to_be(true)
	expect(app:has('role:menuitem "Copy path"')).to_be(true)
	app:click('role:menuitem "Copy path"')
	settle(app)
	expect(app:clipboard()).to_be("/tmp/atlas/out/demo.html")
	app:click("#html-card-chevron-p3-1")
	settle(app)
	app:key("Escape")
	settle(app)
	expect(app:tree('role:menuitem "Zeron"')).to_contain(" hidden")
end)

test("open in: code spans not on disk don't count toward the three", function()
	local app = mount("chat_fixture.luau")
	seeded(app, "pages_seed_spans")
	expect(app:tree("#html-open-p4")).to_contain('"Demo page"')
end)

test("pages: a chat on another device says where its dev server runs, and its Website cards are disabled", function()
	local app = mount("chat_fixture.luau")
	app:resize(960, 900)
	app:command("go_s2")
	settle(app)
	app:set_state("test_patch", { device = "box" })
	app:command("patch_open")
	settle(app)
	app:command("pages_seed_live")
	settle(app)
	expect(app:tree("#html-card-p3-tc4-html")).to_contain("Running on Build box")
	expect(app:has("#html-page-p3-tc4-html")).to_be(false)
	local cards = app:tree("#html-open-p3")
	expect(cards).to_contain("Website · on another device")
	-- (A code span's file can't be looked for there: only linked ones.)
	expect(string.find(cards, "other.html", 1, true)).to_be(nil)
	expect(app:tree("#html-card-open-p3-1")).to_contain("disabled")
end)
