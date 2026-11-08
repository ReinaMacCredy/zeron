-- Computer use's rows in the transcript on ui/chat_fixture.luau
-- (`computer_turn`), from the engine's records as src/chat.rs writes them:
-- gpui's consent and foreground cards and how they're answered, the row a
-- card settles to, and the engine's notes as rows and cards.

local function computer(): any
	local app = mount("chat_fixture.luau")
	app:resize(1100, 2400)
	app:command("go_s2")
	app:frames(20)
	app:command("computer_turn")
	app:frames(40)
	expect(app:error()).to_be(nil)
	return app
end

local function x_of(app: any, sel: string): number
	return tonumber(string.match(app:tree(sel), "^@%d+ [^\n]- (%d+),%d+ %d+x%d+")) or -1
end

test("computer: a consent card is gpui's: its title and what it would do, the risk, the first-use words, Deny, Always allow, then Allow this conversation filled", function()
	local app = computer()
	local card = "#computer-approval-ca1:c1"
	expect(app:has(card)).to_be(true)
	for _, w in {
		"Allow Zeron to use Ghostty?",
		"Open a new tab and run the tests.",
		"Elevated Risk",
		"Zeron could run commands that change files and settings on this Mac.",
		"Computer use lets Zeron use apps on your computer.",
		"Deny",
		"Always allow",
		"Allow this conversation",
	} do
		expect(`{w}: {app:has(`{card} >> ~"{w}"`)}`).to_be(`{w}: true`)
	end
	local deny, always, allow = x_of(app, "#computer-deny-ca1:c1"), x_of(app, "#computer-always-ca1:c1"), x_of(app, "#computer-allow-ca1:c1")
	expect(deny < always and always < allow).to_be(true)
	-- Answered before this window opened: the row it settled to.
	expect(app:has("#computer-approval-ca1:c0")).to_be(false)
	expect(app:has('#transcript >> "Allowed once"')).to_be(true)
end)

test("computer: Allow answers the engine with its label, and the card is its row at once, kept while the rows go and come back", function()
	local app = computer()
	app:set_state("settings_computer", { approvedOnce = false })
	app:frames(5)
	app:click("#computer-allow-ca1:c1")
	app:frames(20)
	expect(app:state("calls") .. ",").to_contain("computer:s2:req-c1:q1:Allow,")
	expect(app:has("#computer-approval-ca1:c1")).to_be(false)
	expect(app:has('#transcript >> "Allowed Ghostty for this task"')).to_be(true)
	expect(app:state("settings_computer").approvedOnce).to_be(true)
	app:command("kinds_rows_away")
	app:frames(10)
	app:command("kinds_rows_back")
	app:frames(30)
	expect(app:has("#computer-approval-ca1:c1")).to_be(false)
	expect(app:has('#transcript >> "Allowed Ghostty for this task"')).to_be(true)
end)

test("computer: Always allow sends the app's own label; the first-use words go once one was allowed", function()
	local app = computer()
	app:set_state("settings_computer", { approvedOnce = true })
	app:frames(10)
	expect(app:has("#computer-disclosure-ca1:c1")).to_be(false)
	app:click("#computer-always-ca1:c1")
	app:frames(20)
	expect(app:state("calls")).to_contain("computer:s2:req-c1:q1:Always allow Ghostty")
	expect(app:has('#transcript >> "Always allowed Ghostty"')).to_be(true)
end)

test("computer: an answer that fails says so on the card, and its buttons stay", function()
	local app = computer()
	app:command("computer_fail_next")
	app:click("#computer-deny-ca1:c1")
	app:frames(20)
	expect(app:has("#computer-approval-ca1:c1")).to_be(true)
	expect(app:tree("#computer-approval-ca1:c1")).to_contain("Answer failed: offline")
	app:click("#computer-deny-ca1:c1")
	app:frames(20)
	expect(app:state("calls")).to_contain("computer:s2:req-c1:q1:Deny")
	expect(app:has('#transcript >> "Declined Ghostty"')).to_be(true)
end)

test("computer: the foreground card answers Allow for this turn once; the engine's answer turns it into its row", function()
	local app = computer()
	local card = "#computer-approval-ca1:f1"
	expect(app:tree(card)).to_contain("Bring Safari to the front?")
	expect(app:tree(card)).to_contain("Zeron needs it in front to type.")
	expect(app:has("#computer-disclosure-ca1:f1")).to_be(false)
	expect(x_of(app, "#computer-no-ca1:f1") < x_of(app, "#computer-allow-ca1:f1")).to_be(true)
	app:click("#computer-allow-ca1:f1")
	app:frames(20)
	app:click("#computer-allow-ca1:f1")
	app:frames(20)
	local _, sent = string.gsub(app:state("calls"), "computer:s2:req%-f1:qf:Allow for this turn", "")
	expect(sent).to_be(1)
	app:command("computer_settle")
	app:frames(20)
	expect(app:has(card)).to_be(false)
	expect(app:has('#transcript >> "You allowed Safari to come to the front"')).to_be(true)
end)

test("computer: the engine's notes are gpui's: Revoked, Paused and Left open as rows; Unavailable, Setup and Computer is off as cards", function()
	local app = computer()
	for _, w in {
		"Desktop access revoked by you (Esc)",
		"Paused while you used Safari",
		"Left 2 Ghostty windows open",
		"Computer is unavailable for this turn",
		"The desktop host is offline.",
		"Computer control needs setup",
		"Computer is off for ordinary turns",
		"Send this request with /computer-use to let the agent use the desktop once.",
	} do
		expect(`{w}: {app:has(`#transcript >> "{w}"`)}`).to_be(`{w}: true`)
	end
	expect(app:has("#computer-card-cs4:row")).to_be(true)
	expect(app:has("#computer-card-cs5:row")).to_be(true)
	expect(app:has("#computer-card-cs2:row")).to_be(false)
	expect(app:has("#computer-action-cs4:row")).to_be(false)
end)

test("computer: Use for this request puts /computer-use before the draft and takes the keyboard there", function()
	local app = computer()
	app:click("#computer-action-ca1:d1")
	app:frames(10)
	expect(app:state("chat.composer.text")).to_be("/computer-use ")
	app:set_state("chat.composer.text", "open notes")
	app:frames(5)
	app:click("#computer-action-ca1:d1")
	app:frames(10)
	expect(app:state("chat.composer.text")).to_be("/computer-use open notes")
	app:click("#computer-action-ca1:d1")
	app:frames(10)
	expect(app:state("chat.composer.text")).to_be("/computer-use open notes")
end)

test("computer: Find counts a card's words and a settled row's, as drawn", function()
	local app = computer()
	local function tally(q: string): string
		app:command("find_open")
		for _ = 1, 4 do
			app:wait("#never-shown", { timeout_ms = 20 })
			app:frames(5)
		end
		app:fill('role:textbox "Find"', q)
		for _ = 1, 4 do
			app:wait("#never-shown", { timeout_ms = 20 })
			app:frames(5)
		end
		return app:tree("#chat-find"):match('text "(%d+ of %d+)"') or app:tree("#chat-find"):match('text "(No results)"') or ""
	end
	expect(tally("Allow Zeron to use")).to_be("1 of 1")
	expect(tally("Allowed once")).to_be("1 of 1")
	expect(tally("run the tests")).to_be("1 of 1")
end)
