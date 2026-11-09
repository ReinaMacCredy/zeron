-- The Review built-in (screens/chat/review, its switch screens/plugins/builtins): a
-- Review button in the chat's header, there only with something to review
-- and the built-in on, running the review and saying so while it runs; the
-- findings a block of the transcript where the review was asked, each with
-- its outcome, Fix on an open one and Fix all open in its head. On
-- ui/review_plugin_fixture.luau: the chat fixture (its review host stood
-- in for) under the window's title bar.

local function open(): any
	local app = mount("review_plugin_fixture.luau")
	app:resize(1100, 760)
	app:command("go_s2")
	app:frames(30)
	return app
end

local function calls(app: any): string
	return app:state("calls") or ""
end

local function times(app: any, what: string): number
	local n = 0
	for call in calls(app):gmatch("[^,]+") do
		if call == what then
			n += 1
		end
	end
	return n
end

local function top_of(app: any, pattern: string): number?
	for line in app:tree("#transcript"):gmatch("[^\n]+") do
		if line:find(pattern, 1, true) then
			return tonumber(line:match(" %-?%d+,(%-?%d+) %d+x%d+"))
		end
	end
	return nil
end

local function landed(): any
	local app = open()
	app:command("review_lands")
	app:frames(30)
	return app
end

test("the chat's Review stands in its header only with something to review and the built-in on", function()
	local app = open()
	app:command("review_none")
	app:frames(20)
	expect(app:has("#chat-review")).to_be(false)
	app:command("review_idle")
	app:frames(20)
	expect(app:has('#titlebar-actions >> #chat-review >> "Review"')).to_be(true)
	expect(calls(app)).to_contain("review.watch:s2")
	-- Off in Settings → Plugins: gone at once; on, back.
	app:command("review_builtin_off")
	app:frames(20)
	expect(app:has("#chat-review")).to_be(false)
	app:command("review_builtin_on")
	app:frames(20)
	expect(app:has("#chat-review")).to_be(true)
	-- (No composer chip: the header's is the one door.)
	expect(app:has("#composer-review")).to_be(false)
end)

test("Review runs the review, says Reviewing and takes no second press; its findings land in the transcript where it was asked", function()
	local app = open()
	app:command("review_idle")
	app:frames(20)
	app:click("#chat-review")
	app:frames(20)
	expect(times(app, "review.start:s2")).to_be(1)
	expect(app:has('#chat-review >> "Reviewing"')).to_be(true)
	app:click("#chat-review")
	app:frames(20)
	expect(times(app, "review.start:s2")).to_be(1)
	expect(app:has("#review-block")).to_be(false)
	app:command("review_lands")
	app:frames(30)
	expect(app:has('#transcript >> #review-block >> "3 findings · Claude Code · haiku"')).to_be(true)
	expect(app:count("#review-block >> .review-finding")).to_be(3)
	-- Under the reply it was asked after (the fixture places the row; the host's placement is Rust `the_findings_block_stands_in_the_transcript_where_the_review_was_asked`).
	local reply, block = top_of(app, "Hi! What shall we work on?"), top_of(app, "#review-block")
	expect(reply ~= nil and block ~= nil and (block :: number) > (reply :: number)).to_be(true)
	-- Each with its severity, title, place and outcome: Fixed, Fix while open, Skipped.
	expect(app:has('#review-finding-F1 >> "Bug"')).to_be(true)
	expect(app:has('#review-finding-F1 >> "A token can reach the log when a request fails"')).to_be(true)
	expect(app:has('#review-finding-F1 >> "crates/engine/src/rpc.rs:212"')).to_be(true)
	expect(app:has('#review-finding-F1 >> "Fixed · abcdef1"')).to_be(true)
	expect(app:has("#review-fix-F1")).to_be(false)
	expect(app:has("#review-finding-F2 >> #review-fix-F2")).to_be(true)
	expect(app:has('#review-finding-F3 >> "Skipped"')).to_be(true)
	expect(app:has('#review-finding-F3 >> "Linux runs it in CI."')).to_be(true)
	expect(app:has("#review-fix-F3")).to_be(false)
end)

test("Fix stages that finding as one composer comment; Fix all open stages the open ones only", function()
	local app = landed()
	app:click("#review-fix-F2")
	app:frames(30)
	expect(calls(app)).to_contain("review.fix:s2:F2")
	expect(app:has('#chat-composer >> role:button ~"1 comment"')).to_be(true)
	-- Staged, it's fixable no more: its Fix and Fix all go, the finding still open.
	expect(app:has("#review-fix-F2")).to_be(false)
	expect(app:has('#review-finding-F2 >> "Open"')).to_be(true)
	expect(app:has("#review-fix-all")).to_be(false)

	local again = landed()
	again:click("#review-fix-all")
	again:frames(30)
	expect(times(again, "review.fix:s2")).to_be(1)
	expect(again:has('#chat-composer >> role:button ~"1 comment"')).to_be(true)
	expect(again:has('#chat-composer >> role:button ~"3 comments"')).to_be(false)
end)

test("a finding's place opens its file at its line", function()
	local app = landed()
	app:click('#review-finding-F2 >> role:button "crates/client/src/conn.rs:88"')
	app:frames(20)
	expect(calls(app)).to_contain("open_file:s2:crates/client/src/conn.rs@88")
end)

test("the block stays through a reload and a chat switch; with the built-in off it stays readable, its actions gone", function()
	local app = landed()
	app:command("remake_session")
	app:frames(30)
	expect(app:has('#review-block >> "3 findings · Claude Code · haiku"')).to_be(true)
	app:command("go_s1")
	app:frames(30)
	expect(app:has("#review-block")).to_be(false)
	app:command("go_s2")
	app:frames(30)
	expect(app:has('#review-block >> "3 findings · Claude Code · haiku"')).to_be(true)
	expect(app:has("#review-fix-F2")).to_be(true)
	app:command("review_builtin_off")
	app:frames(20)
	expect(app:has('#review-block >> "Fixed · abcdef1"')).to_be(true)
	expect(app:has("#review-fix-F2")).to_be(false)
	expect(app:has("#review-fix-all")).to_be(false)
	expect(app:has("#chat-review")).to_be(false)
end)

test("pressed again, the landed block stays with its outcomes and Fix while the new run works", function()
	local app = landed()
	app:command("review_again")
	app:frames(20)
	expect(app:has('#chat-review >> "Reviewing"')).to_be(true)
	expect(app:has('#review-block >> "3 findings · Claude Code · haiku"')).to_be(true)
	expect(app:has('#review-finding-F1 >> "Fixed · abcdef1"')).to_be(true)
	app:click("#review-fix-F2")
	app:frames(30)
	expect(calls(app)).to_contain("review.fix:s2:F2")
	expect(app:has('#chat-composer >> role:button ~"1 comment"')).to_be(true)
end)

test("a finding whose lines went away says outdated, as gpui's meta, and the head counts it", function()
	local app = landed()
	app:command("review_outdated")
	app:frames(20)
	expect(app:has('#review-finding-F2 >> "· outdated"')).to_be(true)
	expect(app:has('#review-finding-F2 >> "Open"')).to_be(true)
	expect(app:has("#review-fix-F2")).to_be(false)
	expect(app:has('#review-finding-F1 >> "· outdated"')).to_be(false)
	expect(app:has('#review-block >> "3 findings · Claude Code · haiku · 1 outdated"')).to_be(true)
	expect(app:has("#review-fix-all")).to_be(false)
end)
