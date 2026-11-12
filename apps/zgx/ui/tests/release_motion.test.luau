-- The first launch after moving from the other interface (screens/shell/arrival) and What's New
-- (screens/shell/whats_new) on ui/shell_fixture.luau: the host's `migration.carried` and
-- `whats_new` stood in for here, its `whats_new.seen` answered by ui/settings_stub.luau.

local DIALOG = "role:dialog"
local PILL = "#arrival-pill"
local CARD = "#whats-new-card"

local ENTRY = {
	version = "0.3.0",
	date = "Nov 20",
	features = {
		{ id = "zgx", title = "A faster interface", description = "Zeron runs on zgx now." },
		{ id = "settings", title = "Settings, tidied", description = "Usage and Appshots have their own pages." },
	},
	improvements = { "Chimes no longer stall the Mac" },
}

local function data(card: boolean): any
	return {
		version = "0.3.0",
		card = if card then ENTRY else nil,
		highlights = ENTRY,
		releases = { ENTRY, { version = "0.2.132", date = "Nov 1", features = { { id = "tabs", title = "Tabs stay put", description = "" } }, improvements = {} } },
	}
end

local function launch(carried: any?, card: boolean): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	if carried then
		app:set_state("migration.carried", carried)
	end
	app:set_state("whats_new", data(card))
	return app
end

local function shown(app: any): boolean
	return app:has(DIALOG) and string.find(string.match(app:tree(DIALOG), "^[^\n]*") or "", " hidden", 1, true) == nil
end

local function seen(app: any): number
	local n = 0
	for _, c in (app:state("settings_stub_calls") :: any) or {} do
		if c == "whats_new.seen" then
			n += 1
		end
	end
	return n
end

test("a run that carried settings and drafts: the pill counts to them, says what moved, and leaves after 5.2 s; the card waits for it", function()
	local app = launch({ settings = 9, drafts = 2 }, true)
	app:frames(20)
	expect(app:has(PILL)).to_be(false)
	app:frames(15)
	expect(app:text(PILL)).to_contain("Moving your settings…")
	expect(app:wait(PILL, { text = "Moved 9 settings and 2 drafts" })).to_be(true)
	expect(app:has(`{PILL} >> "What's new"`)).to_be(true)
	expect(app:has(CARD)).to_be(false)
	app:frames(300)
	expect(app:has(PILL)).to_be(true)
	expect(app:has(CARD)).to_be(false)
	expect(app:wait(PILL, { gone = true, timeout_ms = 1000 })).to_be(true)
	expect(app:has(CARD)).to_be(false)
	expect(app:wait(CARD, { timeout_ms = 1500 })).to_be(true)
	expect(app:has(`{CARD} >> "0.3.0"`)).to_be(true)
	expect(app:has(`{CARD} >> "A faster interface"`)).to_be(true)
end)

test("what moved leaves out a part that's zero, and the pill never comes twice", function()
	local app = launch({ settings = 3, drafts = 0 }, false)
	expect(app:wait(PILL, { text = "Moved 3 settings" })).to_be(true)
	expect(app:text(PILL)).never.to_contain(" and ")
	app:frames(400)
	expect(app:has(PILL)).to_be(false)
	app:set_state("migration.carried", { settings = 1, drafts = 1 })
	app:frames(60)
	expect(app:has(PILL)).to_be(false)
	local drafts = launch({ settings = 0, drafts = 1 }, false)
	expect(drafts:wait(PILL, { text = "Moved 1 draft" })).to_be(true)
end)

test("typing sends the pill away", function()
	local app = launch({ settings = 4, drafts = 1 }, false)
	expect(app:wait(PILL, { text = "Moved 4 settings and 1 draft" })).to_be(true)
	app:set_state("chat.composer.text", "h")
	expect(app:wait(PILL, { gone = true, timeout_ms = 500 })).to_be(true)
end)

test("the pill's What's new opens the dialog on its highlights; Changelog lists the releases; Escape closes it and it's seen", function()
	local app = launch({ settings = 2, drafts = 0 }, true)
	expect(app:wait(PILL, { text = "Moved 2 settings" })).to_be(true)
	app:click(`{PILL} >> "What's new"`)
	app:frames(30)
	expect(app:has(PILL)).to_be(false)
	expect(shown(app)).to_be(true)
	expect(app:text(DIALOG)).to_contain("What's new in 0.3.0")
	expect(app:has(`#whats-new-highlights >> "Settings, tidied"`)).to_be(true)
	expect(app:has(`#whats-new-highlights >> "Chimes no longer stall the Mac"`)).to_be(true)
	app:click(`{DIALOG} >> "Changelog"`)
	app:frames(10)
	expect(app:has(`#whats-new-changelog >> "0.2.132"`)).to_be(true)
	expect(app:has(`#whats-new-changelog >> "Tabs stay put"`)).to_be(true)
	app:key("Escape")
	app:frames(20)
	expect(shown(app)).to_be(false)
	expect(seen(app)).to_be(1)
	expect(app:has(CARD)).to_be(false)
end)

test("no pill on a run that carried nothing; the card springs at 500 ms, × puts it away as seen, and Help → What's New… opens the dialog", function()
	local app = launch(nil, true)
	app:frames(20)
	expect(app:has(PILL)).to_be(false)
	expect(app:has(CARD)).to_be(false)
	expect(app:wait(CARD, { timeout_ms = 600 })).to_be(true)
	app:frames(40)
	app:click("#whats-new-card-close")
	app:frames(10)
	expect(app:has(CARD)).to_be(false)
	expect(seen(app)).to_be(1)
	local help = nil
	for _, c in app:commands() do
		if c.id == "shell.whats_new" then
			help = c
		end
	end
	expect(help and help.menu).to_be("Help")
	app:command("shell.whats_new")
	app:frames(30)
	expect(shown(app)).to_be(true)
	expect(app:has(`#whats-new-highlights >> "A faster interface"`)).to_be(true)
end)

test("the card opens the dialog; without a card to show, none springs", function()
	local app = launch(nil, true)
	expect(app:wait(CARD, { timeout_ms = 600 })).to_be(true)
	app:frames(40)
	app:click(CARD)
	app:frames(30)
	expect(shown(app)).to_be(true)
	expect(app:has(CARD)).to_be(false)
	local none = launch(nil, false)
	none:frames(90)
	expect(none:has(CARD)).to_be(false)
end)
