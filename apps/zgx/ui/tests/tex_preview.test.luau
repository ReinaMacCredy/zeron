local function eventually(app: any, ok: () -> boolean, ms: number?): boolean
	local tries = math.ceil((ms or 5000) / 50)
	for _ = 1, tries do
		if ok() then
			return true
		end
		app:wait('"never shown, only waited past"', { timeout_ms = 50 })
	end
	return ok()
end

local function open(): any
	local app = mount("tex_fixture.luau")
	app:resize(700, 600)
	expect(app:wait('role:img "Page 1 of 1"', { timeout_ms = 5000 })).to_be(true)
	return app
end

test("tex preview: the compiled PDF shows under its errors, and one clicked opens Source at its line", function()
	local app = open()
	expect(app:state("test_tex_compiles")).to_be(1)
	expect(app:has('"1 error in the last compile"')).to_be(true)
	expect(app:has('"Undefined control sequence."')).to_be(true)
	app:click('role:button "Show line 3 in Source"')
	expect(eventually(app, function(): boolean
		return app:state("test_tex_revealed") == 3
	end)).to_be(true)
	expect(eventually(app, function(): boolean
		return not app:has('role:img "Page 1 of 1"')
	end)).to_be(true)
end)

test("tex preview: a saved change compiles again, once, after the typing settles", function()
	local app = open()
	local doc = app:state("test_tex_doc")
	for i = 1, 3 do
		local next_doc = table.clone(doc)
		next_doc.text = `{doc.text}% {i}\n`
		app:set_state("test_tex_doc", next_doc)
		app:frames(2)
	end
	expect(app:state("test_tex_compiles")).to_be(1)
	expect(eventually(app, function(): boolean
		return app:state("test_tex_compiles") == 2
	end)).to_be(true)
	app:wait('"never shown, only waited past"', { timeout_ms = 600 })
	expect(app:state("test_tex_compiles")).to_be(2)
	expect(app:has('role:img "Page 1 of 1"')).to_be(true)
end)

test("tex preview: a click on a page annotates the source lines it came from", function()
	local app = open()
	app:click('role:img "Page 1 of 1"')
	expect(eventually(app, function(): boolean
		return app:state("test_tex_synced") ~= ""
	end)).to_be(true)
	expect(string.sub(app:state("test_tex_synced"), 1, 2)).to_be("1:")
	expect(app:wait('#annotation-pill >> role:button "Add to chat"', { timeout_ms = 3000 })).to_be(true)
	app:frames(30)
	app:click("#annotation-add-to-chat")
	app:frames(30)
	app:type("#annotation-comment", "tighten this")
	app:key("Enter")
	app:frames(30)
	local notes = app:state("test_tex_notes")
	expect(#notes).to_be(1)
	expect(notes[1].source).to_be("/work/cv.tex:5-6")
	expect(notes[1].text).to_be("Hello world\nagain")
	expect(notes[1].comment).to_be("tighten this")
	expect(notes[1].anchor.key).to_be("page.1")
	expect(app:has("#annotation-badge-1")).to_be(true)
	app:click('role:img "Page 1 of 1"')
	expect(app:wait('#annotation-pill >> role:button "Add to chat"', { timeout_ms = 3000 })).to_be(true)
	app:scroll("#pdf-pages", 200)
	expect(eventually(app, function(): boolean
		return not app:has("#annotation-pill")
	end)).to_be(true)
end)

test("tex preview: in the files pane, a page's annotation is staged on the chat as the .tex file and line", function()
	local app = mount("files_fixture.luau")
	app:resize(900, 600)
	app:frames(30)
	app:set_state("test_files_away", true)
	app:command("disk_tex")
	expect(app:wait('role:treeitem "docs"', { timeout_ms = 15000 })).to_be(true)
	app:click('role:treeitem "docs"')
	expect(app:wait('role:treeitem "cv.tex"', { timeout_ms = 15000 })).to_be(true)
	app:frames(30)
	app:set_state("platform", "macos")
	app:click('role:treeitem "cv.tex"')
	expect(app:wait('role:img "Page 1 of 2"', { timeout_ms = 15000 })).to_be(true)
	app:frames(10)
	app:click('role:img "Page 1 of 2"')
	expect(app:wait('#annotation-pill >> role:button "Add to chat"', { timeout_ms = 3000 })).to_be(true)
	app:frames(30)
	app:click("#annotation-add-to-chat")
	app:frames(30)
	app:type("#annotation-comment", "bolder")
	app:key("Enter")
	app:frames(30)
	local staged = nil :: any
	for key, d in app:state("chat.composer.drafts") do
		if key == "chat-1" and d.cargo then
			staged = d.cargo[1]
		end
	end
	expect(staged ~= nil).to_be(true)
	expect(staged.source).to_be("docs/cv.tex:3")
	expect(staged.text).to_be("Write it")
	expect(staged.comment).to_be("bolder")
	app:click('role:img "Page 1 of 2"')
	expect(app:wait('#annotation-pill >> role:button "Mention"', { timeout_ms = 3000 })).to_be(true)
	app:frames(30)
	app:click("#annotation-mention")
	app:frames(30)
	local kept = app:state("chat.composer.drafts")["chat-1"]
	local text = if app:state("chat.composer.here") == "chat-1" then app:state("chat.composer.text") else kept and kept.text
	expect(text).to_contain("> docs/cv.tex:3: Write it")
end)
