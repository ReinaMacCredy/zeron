-- A tool row opens to what it did (ds/tool_detail through ds/activity;
-- ui/tool_detail_fixture.luau): an edit to its whole diff, fetched as it
-- opens, and while the work is the reply's live tail, open by itself; a command to its output, its first ten lines and
-- "Show all N lines" (its whole output fetched as it opens: a failure says
-- so, and a tap tries again); a short one whole; a failed one with its exit
-- code, escapes out; a huge one's end only; a running one's output growing
-- in place; a read to its result; an older doc's edit to its diff at once;
-- a command the chat kept the end of to those ten lines, then all of them
-- from the device that ran it (a quiet "Loading…" while they come, a plain
-- line when that device is offline, too old, or didn't keep them).
-- What was fetched is kept: opened again, or the work made again as a
-- reload makes it, nothing is fetched twice.

local function work(): any
	local app = mount("tool_detail_fixture.luau")
	app:resize(900, 1600)
	app:frames(10)
	expect(app:error()).to_be_nil()
	-- (Settled work folds to its line: open it.)
	app:click('role:button "Read 1 file · ran 6 commands · edited 2 files · 1 failed"')
	app:frames(30)
	return app
end

local function fetched(app: any): number
	local n = 0
	for _ in string.gmatch(app:state("test_tool_fetches") or "", "[^\n]+") do
		n += 1
	end
	return n
end

-- The height of the first detail under the row whose line has `label`.
local function detail_h(app: any, label: string): number
	local seen = false
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(label, 1, true) then
			seen = true
		elseif seen and line:match("^%s*@%d+ scroll ") then
			return tonumber(line:match(" %-?%d+,%-?%d+ %d+x(%d+)")) or -1
		end
	end
	return -1
end

local function open_row(app: any, label: string)
	app:click(`role:button "{label}"`)
	app:frames(20)
end

test("an edit opens to its whole diff, fetched as it opens", function()
	local app = work()
	expect(fetched(app)).to_be(0)
	open_row(app, "Edited app.rs")
	expect(app:has('.tool-diff >> ~"new_call();"')).to_be(true)
	expect(app:has('.tool-diff >> ~"old_call();"')).to_be(true)
	expect(app:has('role:button "Show full diff"')).to_be(false)
	expect(fetched(app)).to_be(1)
end)

test("while the work is the reply's live tail, an edit opens by itself to its diff, and folds when it settles", function()
	local app = work()
	expect(app:has('.tool-diff >> ~"new_call();"')).to_be(false)
	app:set_state("test_tool_live", true)
	app:frames(20)
	expect(app:has('.tool-diff >> ~"new_call();"')).to_be(true)
	expect(app:has('.tool-diff >> ~"after"')).to_be(true)
	-- (A long diff, at most 240 points of it, scrolling.)
	expect(detail_h(app, "Edited app.rs")).to_be(240)
	expect(app:has('.tool-output >> ~"The first line."')).to_be(false)
	app:set_state("test_tool_live", false)
	app:frames(30)
	expect(app:has('.tool-diff >> ~"new_call();"')).to_be(false)
	open_row(app, "Edited app.rs")
	expect(app:has('.tool-diff >> ~"new_call();"')).to_be(true)
	expect(detail_h(app, "Edited app.rs") > 240).to_be(true)
	expect(fetched(app)).to_be(1)
end)

-- A node's identity in the tree, by a pattern on its line: `@id`.
local function node(app: any, pattern: string): string?
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(pattern, 1, true) then
			return line:match("@(%d+)")
		end
	end
	return nil
end

test("a command opens to its output's first ten lines and all of them when asked; its whole output comes as it opens, and a failure says so and tries again", function()
	local app = work()
	app:set_state("test_tool_fail", true)
	open_row(app, "Ran cargo test")
	-- (The summary, while the whole output failed to come.)
	expect(fetched(app)).to_be(1)
	expect(app:has('role:button "Couldn\'t load full output — tap to retry"')).to_be(true)
	expect(app:has('.tool-output >> ~"line 10"')).to_be(true)
	expect(app:has('.tool-output >> ~"line 11"')).to_be(false)
	expect(app:has('role:button "Show all 30 lines"')).to_be(true)
	app:click('role:button "Couldn\'t load full output — tap to retry"')
	app:frames(20)
	expect(fetched(app)).to_be(2)
	expect(app:has('~"Couldn\'t load full output"')).to_be(false)
	-- (The fold is the tool's: the whole output comes folded too.)
	expect(app:has('.tool-output >> ~"line 11"')).to_be(false)
	local folded = detail_h(app, "Ran cargo test")
	app:click('role:button "Show all 30 lines"')
	app:frames(20)
	expect(app:has('.tool-output >> ~"line 30"')).to_be(true)
	expect(detail_h(app, "Ran cargo test") > folded + 200).to_be(true)
	expect(app:has('~"more lines"')).to_be(false)
	app:click('role:button "Show fewer lines"')
	app:frames(20)
	expect(app:has('.tool-output >> ~"line 11"')).to_be(false)
	expect(app:has('role:button "Show all 30 lines"')).to_be(true)
end)

test("a short output shows whole, with no button", function()
	local app = work()
	open_row(app, "Ran echo ok")
	expect(app:has('.tool-output-text >> ~"ok"')).to_be(true)
	expect(app:has(".tool-output-fold")).to_be(false)
	expect(app:has(".tool-exit-code")).to_be(false)
end)

test("a failed command says its exit code, its escapes and overwritten progress gone", function()
	local app = work()
	open_row(app, "Ran make")
	expect(app:has('.tool-exit-code >> ~"Exit code 2"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"error: missing target"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"fetch 100%"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"fetch 10%"')).to_be(false)
	expect(app:has('.tool-output-text >> ~"Exit code"')).to_be(false)
	expect(app:has('.tool-output-text >> ~"[0m"')).to_be(false)
	expect(app:has('.tool-output-text >> ~"title"')).to_be(false)
end)

test("a huge output keeps its end, saying earlier output was trimmed", function()
	local app = work()
	open_row(app, "Ran seq 5000")
	expect(app:has('.tool-output >> ~"… earlier output trimmed"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"row 4601"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"row 4600"')).to_be(false)
	expect(app:has('role:button "Show all 400 lines"')).to_be(true)
	app:click('role:button "Show all 400 lines"')
	app:frames(20)
	expect(app:has('.tool-output-text >> ~"row 5000"')).to_be(true)
end)

test("a running command's output grows in place", function()
	local app = work()
	open_row(app, "Ran cargo build")
	expect(app:has('.tool-output-text >> ~"Compiling zeron"')).to_be(true)
	local before = node(app, "Compiling zeron")
	expect(before).never.to_be_nil()
	app:set_state("test_tool_stream", "Compiling zeron\nFinished dev\n")
	app:frames(10)
	expect(app:has('.tool-output-text >> ~"Finished dev"')).to_be(true)
	expect(node(app, "Compiling zeron")).to_be(before)
end)

test("a read opens to its result; an older doc's edit to its diff at once", function()
	local app = work()
	open_row(app, "Read notes.md")
	expect(app:has('.tool-output >> ~"The first line."')).to_be(true)
	open_row(app, "Edited old.txt")
	expect(app:has('.tool-diff >> ~"after"')).to_be(true)
	expect(fetched(app)).to_be(0)
end)

test("what was fetched is kept: the work made again (as a reload makes it) fetches nothing twice", function()
	local app = work()
	open_row(app, "Edited app.rs")
	expect(fetched(app)).to_be(1)
	app:set_state("test_tool_generation", 2)
	app:frames(30)
	-- (Opened rows and the open group are kept by id too.)
	expect(app:has('.tool-diff >> ~"new_call();"')).to_be(true)
	expect(fetched(app)).to_be(1)
end)

local function wholes(app: any): number
	local n = 0
	for _ in string.gmatch(app:state("test_tool_fetches") or "", "whole:[^\n]+") do
		n += 1
	end
	return n
end

test("a command the chat kept the end of shows those ten lines, then all of them from its host", function()
	local app = work()
	open_row(app, "Ran cargo test --workspace")
	expect(app:has('.tool-output-text >> ~"case 291 ok"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"case 300 ok"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"case 290 ok"')).to_be(false)
	expect(app:has(".tool-exit-code")).to_be(false)
	expect(wholes(app)).to_be(0)
	-- (Asked for: a quiet line while it comes, the ten lines still there.)
	app:set_state("test_tool_whole", "hold")
	app:click('role:button "Show all 300 lines"')
	app:frames(10)
	expect(wholes(app)).to_be(1)
	expect(app:has('role:button "Loading all 300 lines…"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"case 300 ok"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"case 1 ok"')).to_be(false)
	app:set_state("test_tool_release", 1)
	app:frames(20)
	expect(app:has('.tool-output-text >> ~"case 1 ok"')).to_be(true)
	expect(app:has('.tool-output-text >> ~"case 300 ok"')).to_be(true)
	expect(app:has('role:button "Show fewer lines"')).to_be(true)
	expect(app:has(".tool-output-trouble")).to_be(false)
	-- Folded again: the ten; opened again: the whole at once, not read twice.
	-- (Three hundred lines: down to the button under them.)
	app:scroll('role:button "Ran cargo test --workspace"', 6000)
	app:frames(10)
	app:click('role:button "Show fewer lines"')
	app:frames(20)
	expect(app:has('.tool-output-text >> ~"case 290 ok"')).to_be(false)
	expect(app:has('role:button "Show all 300 lines"')).to_be(true)
	app:click('role:button "Show all 300 lines"')
	app:frames(20)
	expect(app:has('.tool-output-text >> ~"case 1 ok"')).to_be(true)
	expect(wholes(app)).to_be(1)
end)

test("a whole output that can't come says why, keeps the ten lines, and is asked for again", function()
	local app = work()
	open_row(app, "Ran cargo test --workspace")
	for _, case in {
		{ word = "offline", says = "The device that ran this command is offline. Try again when it's back." },
		{ word = "unavailable", says = "The rest of this output wasn't kept." },
		{ word = "update", says = "Update Zeron on the device that ran this command to see all of its output." },
	} do
		app:set_state("test_tool_whole", case.word)
		app:click('role:button "Show all 300 lines"')
		app:frames(20)
		expect(app:has(`.tool-output-trouble >> ~"{case.says}"`)).to_be(true)
		expect(app:has('.tool-output-text >> ~"case 300 ok"')).to_be(true)
		expect(app:has('.tool-output-text >> ~"case 1 ok"')).to_be(false)
		expect(app:has('role:button "Show all 300 lines"')).to_be(true)
	end
	expect(wholes(app)).to_be(3)
	-- Back: asked again, it comes, and the line goes.
	app:set_state("test_tool_whole", "ok")
	app:click('role:button "Show all 300 lines"')
	app:frames(20)
	expect(wholes(app)).to_be(4)
	expect(app:has('.tool-output-text >> ~"case 1 ok"')).to_be(true)
	expect(app:has(".tool-output-trouble")).to_be(false)
end)
