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
	improvements = { { text = "Chimes no longer stall the Mac" } },
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

local VISUALS = {
	version = "0.2.132",
	date = "Nov 1",
	hero_visual = "answer-pill-hero",
	features = {
		{ id = "answer-pill", title = "Answer from the composer", description = "", visual = "answer-pill" },
		{ id = "keep-awake", title = "Stays awake", description = "", visual = "keep-awake" },
		{ id = "plain", title = "No visual of ours", description = "", visual = "not-ported" },
		{ id = "text-annotations", title = "Comment on text", description = "", visual = "text-annotation" },
		{ id = "skills-page", title = "Skills", description = "", visual = "skills-page" },
		{ id = "worktree-follow", title = "Worktrees", description = "", visual = "worktree-follow" },
		{ id = "task-references", title = "Depends on", description = "", visual = "task-depends" },
		{ id = "live-preview", title = "Live preview", description = "", visual = "live-preview" },
		{ id = "window-frame", title = "Window frame", description = "", visual = "window-frame" },
		{ id = "project-picker", title = "Project picker", description = "", visual = "project-picker" },
		{ id = "zgx-app", title = "The new interface", description = "" },
		{ id = "markdown-fence-render", title = "Rendered Markdown", description = "" },
		{ id = "mission-threads", title = "Threads tab", description = "" },
		{ id = "mission-plan", title = "Plan card", description = "" },
		{ id = "not-a-visual", title = "Nothing by its id", description = "" },
	},
	improvements = {
		{ text = "Compaction morphs", visual = "compaction-morph" },
		{ text = "Swipes", visual = "swipe-dot" },
		{ text = "Tabs drag", visual = "tab-drag" },
		{ text = "Plain" },
	},
}

local function clock(app: any, which: string): number
	return app:state(`whats_new_{which}_clock`) or 0
end

test("every visual the entry names draws in the dialog and the card's hero; a clock runs only while its visuals show", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:set_state("whats_new", { version = "0.2.132", card = VISUALS, highlights = VISUALS, releases = { VISUALS } })
	expect(app:wait(CARD, { timeout_ms = 600 })).to_be(true)
	expect(app:has(`{CARD} >> #whats-new-visual-answer-pill-hero`)).to_be(true)
	app:frames(30)
	expect(clock(app, "card") > 0).to_be(true)
	expect(clock(app, "dialog")).to_be(0)
	app:command("shell.toggle_sidebar")
	app:frames(30)
	local folded_at = clock(app, "card")
	app:frames(30)
	expect(clock(app, "card")).to_be(folded_at)
	app:command("shell.toggle_sidebar")
	app:frames(30)
	local unfolded_at = clock(app, "card")
	app:frames(10)
	expect(clock(app, "card") > unfolded_at).to_be(true)
	app:click(CARD)
	app:frames(30)
	for _, id in {
		"answer-pill",
		"keep-awake",
		"compaction-morph",
		"swipe-dot",
		"tab-drag",
		"text-annotation",
		"skills-page",
		"worktree-follow",
		"task-depends",
		"live-preview",
		"window-frame",
		"project-picker",
		"zgx-app",
		"markdown-fence-render",
		"mission-threads",
		"mission-plan",
	} do
		expect(app:has(`#whats-new-highlights >> #whats-new-visual-{id}`)).to_be(true)
	end
	expect(app:has("#whats-new-visual-not-ported")).to_be(false)
	expect(app:has("#whats-new-visual-plain")).to_be(false)
	expect(app:has("#whats-new-visual-not-a-visual")).to_be(false)
	local card_at = clock(app, "card")
	local open_at = clock(app, "dialog")
	app:frames(20)
	expect(clock(app, "card")).to_be(card_at)
	expect(clock(app, "dialog") > open_at).to_be(true)
	app:key("Escape")
	app:frames(20)
	local closed_at = clock(app, "dialog")
	app:frames(30)
	expect(clock(app, "dialog")).to_be(closed_at)
end)

test("with motion reduced no clock runs and each visual holds its still frame", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:set_state("motion_reduce", true)
	app:set_state("whats_new", { version = "0.2.132", card = nil, highlights = VISUALS, releases = { VISUALS } })
	app:command("shell.whats_new")
	app:frames(40)
	expect(app:has("#whats-new-visual-keep-awake")).to_be(true)
	expect(clock(app, "dialog")).to_be(0)
	expect(app:has(`#whats-new-visual-keep-awake >> "Working 43s"`)).to_be(true)
	expect(app:has(`#whats-new-visual-answer-pill >> "Which provider first?"`)).to_be(true)
	expect(app:has(`#whats-new-visual-zgx-app >> "Moved 12 chats, 2 drafts and 9 settings"`)).to_be(true)
	expect(app:has(`#whats-new-visual-task-depends >> "Blocked by"`)).to_be(true)
	expect(app:has(`#whats-new-visual-mission-threads >> "Waiting on you"`)).to_be(true)
	expect(app:has(`#whats-new-visual-mission-plan >> "Approved"`)).to_be(true)
end)
