-- A ```markdown fence in a reply (ds/markdown): drawn as Markdown in its block, its source a toggle away. On the page in ui/md_fence_fixture.luau.

local function open(): any
	local app = mount("md_fence_fixture.luau")
	app:resize(900, 700)
	app:frames(20)
	return app
end

test("a markdown fence shows its document drawn, and its source with the toggle off", function()
	local app = open()
	expect(app:wait('role:checkbox "Render Markdown"', { timeout_ms = 15000 })).to_be(true)
	expect(app:tree('role:checkbox "Render Markdown"')).to_contain("checked")
	expect(app:has('role:heading "Handoff title"')).to_be(true)
	expect(app:has('"# Handoff title"')).to_be(false)
	expect(app:has('role:checkbox "Fit content"')).to_be(false)
	app:wait('"never shown, only waited past"', { timeout_ms = 400 })
	app:click('role:checkbox "Render Markdown"')
	expect(app:wait('role:heading "Handoff title"', { gone = true, timeout_ms = 15000 })).to_be(true)
	expect(app:tree()).to_contain("# Handoff title")
	expect(app:has('role:checkbox "Fit content"')).to_be(true)
end)
