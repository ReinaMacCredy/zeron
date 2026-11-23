local function open(): any
	local app = mount("md_file_cards_fixture.luau")
	app:resize(700, 1000)
	app:frames(20)
	expect(app:error()).to_be_nil()
	expect(app:wait(".md-file-card", { timeout_ms = 3000 })).to_be(true)
	app:frames(5)
	return app
end

local function cards(app: any): number
	local n = 0
	for _ in string.gmatch(app:tree("#reply"), 'button "[^"]+" ') do
		n += 1
	end
	return n
end

local REPORT = 'role:button "checkout-latency-report.md"'
local CSV = 'role:button "checkout-latency.csv"'

test("file cards: a paragraph or list naming only listed files is cards; prose, or a dotted word the folder doesn't list, stays words", function()
	local app = open()
	expect(cards(app)).to_be(4)
	for _, name in { "checkout-latency-report.md", "checkout-latency.csv", "chart.png", "notes.txt" } do
		expect(app:has(`role:button "{name}"`)).to_be(true)
	end
	expect(app:has(`{REPORT} >> "Markdown, 4 KB"`)).to_be(true)
	expect(app:has('"string.find"')).to_be(true)
	expect(app:has('"notes.txt missing.md"')).to_be(true)
	expect(app:has('"See checkout-latency-report.md for the rest."')).to_be(true)
	expect(app:has('role:link "checkout-latency-report.md"')).to_be(false)
	local lists = 0
	for _, c in app:state("test_calls") do
		if c == "list:" then
			lists += 1
		end
	end
	expect(lists).to_be(1)
end)

test("file cards: a press opens the file's head under the row, another card's takes its place, the open one's closes it", function()
	local app = open()
	app:click(REPORT)
	expect(app:wait('"28 more lines"', { timeout_ms = 3000 })).to_be(true)
	expect(app:tree(".md-file-preview")).to_contain("# Checkout latency, week 47")
	app:click(CSV)
	expect(app:wait('"165 more lines"', { timeout_ms = 3000 })).to_be(true)
	expect(app:has('.md-file-preview >> "p95"')).to_be(true)
	expect(app:has('.md-file-preview >> "2026-11-18"')).to_be(true)
	expect(app:has('"28 more lines"')).to_be(false)
	app:frames(30)
	app:click('.md-file-preview >> role:button "Open in Files panel"')
	app:click('.md-file-preview >> role:button "Reveal in Finder"')
	app:frames(10)
	local calls = app:state("test_calls")
	expect(calls).to_contain("open:checkout-latency.csv")
	expect(calls).to_contain("reveal:s1:/work/atlas:checkout-latency.csv")
	app:click(CSV)
	expect(app:wait(".md-file-preview", { gone = true, timeout_ms = 3000 })).to_be(true)
end)

test("file cards: one whose file went away since it showed says so, dimmed, and opens nothing", function()
	local app = open()
	app:set_state("test_gone", { "checkout-latency.csv" })
	app:click(CSV)
	expect(app:wait(`{CSV} >> "No longer on this machine"`, { timeout_ms = 3000 })).to_be(true)
	app:frames(30)
	expect(app:has(".md-file-preview")).to_be(false)
	expect(cards(app)).to_be(4)
	app:click(CSV)
	app:frames(30)
	expect(app:has(".md-file-preview")).to_be(false)
end)

test("file cards: while the reply is being written its files stay words, and turn to cards once it ends", function()
	local app = open()
	app:set_state("test_live", true)
	app:set_state("test_reply", app:state("test_reply") .. "\n\nnotes.txt checkout-latency.csv")
	app:frames(30)
	expect(app:has('"notes.txt checkout-latency.csv"')).to_be(true)
	expect(cards(app)).to_be(4)
	app:set_state("test_live", false)
	app:frames(30)
	expect(app:wait('"notes.txt checkout-latency.csv"', { gone = true, timeout_ms = 3000 })).to_be(true)
	expect(cards(app)).to_be(6)
end)

test("file cards: a file named before it was written turns to a card once its folder lists it", function()
	local app = open()
	expect(app:has('"later.md"')).to_be(true)
	app:set_state("test_extra", { "later.md" })
	app:wait('"never shown, only waited past"', { timeout_ms = 6000 })
	app:set_state("test_reply", app:state("test_reply") .. "\n\nlater.md checkout-latency.csv")
	expect(app:wait('role:button "later.md"', { timeout_ms = 3000 })).to_be(true)
	app:frames(10)
	local _, later = string.gsub(app:tree("#reply"), 'button "later.md"', "")
	expect(later).to_be(2)
	expect(string.find(app:tree("#reply"), 'text "later.md"', 1, true)).to_be_nil()
end)
