-- The composer's review chip and its findings (screens/chat/review), on ui/chat_fixture.luau, its host stood in for (`Review.host`): the state is the host's (src/chat/review.rs).

local function open(): any
	local app = mount("chat_fixture.luau")
	app:resize(1100, 900)
	app:command("go_s2")
	app:frames(20)
	expect(app:error()).to_be(nil)
	return app
end

local function calls(app: any): string
	return tostring(app:state("calls") or "")
end

test("the chip asks the host about its chat, and shows nothing until the host says", function()
	local app = open()
	expect(calls(app)).to_contain("review.watch:s2")
	expect(app:has("#composer-review")).to_be(false)
end)

test("the review button runs a review: Review +N −M starts it, Reviewing while it runs", function()
	local app = open()
	app:command("review_idle")
	app:frames(10)
	expect(app:has('#composer-review >> "Review"')).to_be(true)
	expect(app:has('#composer-review >> "+12"')).to_be(true)
	expect(app:has('#composer-review >> "−3"')).to_be(true)
	app:click("#composer-review")
	app:frames(5)
	expect(calls(app)).to_contain("review.start:s2")
	app:command("review_busy")
	app:frames(10)
	expect(app:has('#composer-review >> "Reviewing"')).to_be(true)
	local before = calls(app)
	app:click("#composer-review")
	app:frames(5)
	expect(calls(app)).to_be(before)
end)

test("its scopes are on the caret: a pick goes to the host, Review now starts it", function()
	local app = open()
	app:command("review_idle")
	app:frames(10)
	app:click("#composer-review-scope")
	app:frames(10)
	expect(app:has('role:menuitem "Latest turn +4 −1"')).to_be(true)
	app:click('role:menuitem "Latest turn +4 −1"')
	app:wait('"never shown, only waited past"', { timeout_ms = 400 })
	expect(calls(app)).to_contain("review.scope:s2:latest_turn")
	app:click("#composer-review-scope")
	app:frames(10)
	app:click('role:menuitem "Review uncommitted"')
	app:wait('"never shown, only waited past"', { timeout_ms = 400 })
	expect(calls(app)).to_contain("review.start:s2")
end)

test("done, the chip opens the findings: each its severity, title, place and outcome; Dismiss and Fix go to the host", function()
	local app = open()
	app:command("review_done")
	app:frames(10)
	expect(app:has('#composer-review >> "1 finding"')).to_be(true)
	expect(app:has("#review-findings")).to_be(false)
	app:click("#composer-review")
	app:frames(10)
	expect(app:has("#review-findings")).to_be(true)
	expect(app:text("#review-finding-F1 >> .review-severity")).to_be("Bug")
	expect(app:has('#review-finding-F1 >> "the frame is drawn twice"')).to_be(true)
	expect(app:has('#review-finding-F1 >> "src/render.rs:42"')).to_be(true)
	expect(app:text("#review-finding-F1 >> .review-state")).to_be("Open")
	expect(app:text("#review-finding-F2 >> .review-state")).to_be("Fixed · abcdef1")
	expect(app:has('#review-findings >> "1 fixed · 1 open"')).to_be(true)
	app:click("#review-dismiss-F2")
	app:frames(5)
	expect(calls(app)).to_contain("review.dismiss:s2:F2")
	app:click("#review-fix-remaining")
	app:frames(10)
	expect(calls(app)).to_contain("review.fix:s2")
	local staged = {}
	for _, item in (app:state("chat.composer.cargo") or {}) :: { any } do
		if item.kind == "comment" then
			table.insert(staged, `{item.file}:{item.line}`)
		end
	end
	expect(table.concat(staged, ",")).to_be("src/render.rs:42")
	app:click("#composer-review")
	app:frames(10)
	expect(app:has("#review-findings")).to_be(false)
end)
