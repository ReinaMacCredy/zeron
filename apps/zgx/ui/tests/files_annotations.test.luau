-- Text selected in a file's Markdown preview: the same pill (Mention, Add to
-- chat, Ask in side chat) and comment as the conversation's, the annotation
-- naming its file and staged on the chat the files are its, its badge on the
-- preview's line; Mention quotes it into that chat's draft under its path.

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

test("preview annotations: a selection in a rendered file is annotated with its path on the files' chat", function()
	local app = mount("files_fixture.luau")
	app:resize(900, 600)
	app:frames(30)
	app:set_state("test_files_away", true)
	app:command("disk_md_links")
	expect(app:wait('role:treeitem "docs"', { timeout_ms = 15000 })).to_be(true)
	app:click('role:treeitem "docs"')
	expect(app:wait('role:treeitem "guide.md"', { timeout_ms = 15000 })).to_be(true)
	app:frames(30)
	app:click('role:treeitem "guide.md"')
	expect(app:wait('"Write it"', { timeout_ms = 15000 })).to_be(true)
	app:wait('"never shown, only waited past"', { timeout_ms = 400 })
	app:frames(10)
	local b = box_of(app, "#file-viewer", '"Write it"') :: any
	local line = { x = b.x, y = b.y, w = b.w, h = b.h }
	app:set_state("fake_selection", { text = "Write it", top = line, bottom = line })
	app:click('#file-viewer >> "Write it"')
	app:frames(30)
	expect(app:has('#annotation-pill >> role:button "Add to chat"')).to_be(true)
	app:click("#annotation-add-to-chat")
	app:frames(30)
	app:type("#annotation-comment", "tick this")
	app:key("Enter")
	app:frames(30)
	local staged = nil :: any
	for key, d in app:state("chat.composer.drafts") do
		if key == "chat-1" and d.cargo then
			staged = d.cargo[1]
		end
	end
	expect(staged ~= nil).to_be(true)
	expect(staged.source).to_be("docs/guide.md")
	expect(staged.comment).to_be("tick this")
	local badge = box_of(app, "#file-viewer", "#annotation-badge-1") :: any
	expect(badge ~= nil).to_be(true)
	expect(math.abs(badge.y - (b.y - 28.5)) <= 1).to_be(true)
	app:set_state("fake_selection", { text = "Write it", top = line, bottom = line })
	app:click('#file-viewer >> "Write it"')
	app:frames(30)
	local names: { string } = {}
	for l in app:tree("#annotation-pill"):gmatch("[^\n]+") do
		local name = l:match(' button "([^"]+)"')
		if name then
			table.insert(names, name)
		end
	end
	expect(table.concat(names, "|")).to_be("Mention|Add to chat|Ask in side chat")
	app:click("#annotation-mention")
	app:frames(30)
	local kept = app:state("chat.composer.drafts")["chat-1"]
	local text = if app:state("chat.composer.here") == "chat-1" then app:state("chat.composer.text") else kept and kept.text
	expect(text).to_contain("> docs/guide.md: Write it")
end)
