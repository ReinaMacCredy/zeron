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
	local app = mount("pdf_fixture.luau")
	app:resize(700, 600)
	expect(app:wait('role:img "Page 1 of 300"', { timeout_ms = 5000 })).to_be(true)
	expect(eventually(app, function(): boolean
		return app:state("test_pdf_calls") > 0
	end)).to_be(true)
	return app
end

test("pdf viewer: a PDF opens as its pages, fitted to the width, only those in view drawn", function()
	local app = open()
	expect(app:has('"Page 1 of 300  ·  PDF  ·  121 KB"')).to_be(true)
	expect(app:has('"84%"')).to_be(true)
	app:wait('"never shown, only waited past"', { timeout_ms = 400 })
	expect(app:state("test_pdf_farthest") <= 4).to_be(true)
	expect(app:count('role:img "Page 299 of 300"')).to_be(0)
end)

test("pdf viewer: the foot names the page in view as it scrolls", function()
	local app = open()
	app:scroll("#pdf-pages", 5000)
	expect(eventually(app, function(): boolean
		return app:has('"Page 8 of 300  ·  PDF  ·  121 KB"')
	end)).to_be(true)
	expect(eventually(app, function(): boolean
		return string.find(app:state("test_pdf_asked"), ";8@0.84;", 1, true) ~= nil
	end)).to_be(true)
	-- (Page 6 is built above the view, but too far from it to be drawn.)
	expect(string.find(app:state("test_pdf_asked"), ";6@", 1, true)).to_be(nil)
end)

test("pdf viewer: zoom steps keep the page in view and draw it again at the new zoom", function()
	local app = open()
	app:scroll("#pdf-pages", 5000)
	expect(eventually(app, function(): boolean
		return app:has('"Page 8 of 300  ·  PDF  ·  121 KB"')
	end)).to_be(true)
	app:click('role:button "Zoom In"')
	expect(eventually(app, function(): boolean
		return app:has('"100%"')
	end)).to_be(true)
	expect(app:has('"Page 8 of 300  ·  PDF  ·  121 KB"')).to_be(true)
	expect(eventually(app, function(): boolean
		return string.find(app:state("test_pdf_asked"), ";8@1;", 1, true) ~= nil
	end)).to_be(true)
	app:click('role:button "Fit or actual size"')
	expect(eventually(app, function(): boolean
		return app:has('"84%"')
	end)).to_be(true)
	expect(app:has('"Page 8 of 300  ·  PDF  ·  121 KB"')).to_be(true)
	-- (Out to half: a page shorter than the view, so only the zoom's own anchor keeps it.)
	for _ = 1, 3 do
		app:click('role:button "Zoom Out"')
	end
	expect(eventually(app, function(): boolean
		return app:has('"50%"')
	end)).to_be(true)
	expect(app:has('"Page 8 of 300  ·  PDF  ·  121 KB"')).to_be(true)
end)

test("pdf viewer: one that can't be shown says why, and opens elsewhere", function()
	local app = open()
	app:set_state("test_pdf_doc", { path = "docs/locked.pdf", absolute = "/work/docs/locked.pdf", state = "ready", pdf = true, size = 2048 })
	expect(app:wait('"Couldn\'t open locked.pdf"', { timeout_ms = 5000 })).to_be(true)
	expect(app:has('"It\'s locked with a password."')).to_be(true)
	expect(app:has('role:button "Zoom In"')).to_be(false)
	expect(app:has('"Page 1 of 300  ·  PDF  ·  2 KB"')).to_be(false)
	app:click('role:button "Open in Default App"')
	expect(app:state("test_pdf_opened")).to_be("/work/docs/locked.pdf")
end)

test("pdf viewer: its menu opens it in the default app and reveals it, at any width", function()
	local app = open()
	app:click('role:button "More"')
	expect(app:wait('role:menuitem "Open in Default App"', { timeout_ms = 2000 })).to_be(true)
	expect(app:has('role:menuitem "Reveal in Finder"')).to_be(true)
	expect(app:has('role:menuitem "Open in Editor"')).to_be(false)
	app:click('role:menuitem "Open in Default App"')
	expect(app:state("test_pdf_opened")).to_be("/work/docs/sample.pdf")
end)

test("pdf viewer: the files pane opens a PDF here as its pages on a Mac, elsewhere as binary", function()
	local app = mount("files_fixture.luau")
	app:resize(900, 600)
	app:set_state("platform", "macos")
	app:command("disk_pdf")
	expect(app:wait('role:treeitem "manual.pdf"', { timeout_ms = 15000 })).to_be(true)
	app:click('role:treeitem "manual.pdf"')
	expect(app:wait('role:img "Page 2 of 2"', { timeout_ms = 5000 })).to_be(true)
	app:set_state("platform", "linux")
	app:click('role:treeitem "zeta.txt"')
	expect(app:wait('"z"', { timeout_ms = 5000 })).to_be(true)
	app:click('role:treeitem "manual.pdf"')
	expect(app:wait('"manual.pdf isn\'t text"', { timeout_ms = 5000 })).to_be(true)
end)

test("pdf viewer: the image view's keys zoom it once a page is clicked", function()
	local app = open()
	app:click('role:img "Page 1 of 300"')
	app:key("cmd+=")
	expect(eventually(app, function(): boolean
		return app:has('"100%"')
	end)).to_be(true)
	app:key("cmd+-")
	expect(eventually(app, function(): boolean
		return app:has('"75%"')
	end)).to_be(true)
	app:key("cmd+1")
	expect(eventually(app, function(): boolean
		return app:has('"100%"')
	end)).to_be(true)
	app:key("cmd+0")
	expect(eventually(app, function(): boolean
		return app:has('"84%"')
	end)).to_be(true)
end)
