-- Files carried over the chat from outside dim it under "Drop to attach"
-- until they drop or leave. A leave lost on the way (the platform's drag
-- ended elsewhere) must not leave it dimmed for good: Escape lets go of
-- them, as do the pointer back over the chat and a while with no word of
-- them. A sidebar session carried (the window's own drag) is kept.

local function open(app: any)
	app:resize(900, 700)
	app:command("go_s1")
	app:frames(30)
end

local function carried(app: any, what: any)
	app:set_state("chat.dropping", { session = what })
	app:frames(2)
end

test("Escape lets go of files carried over the chat", function()
	local app = mount("chat_fixture.luau")
	open(app)
	-- (The keys in the composer: the shell, not in this fixture, hears a
	-- bare Escape with nothing focused the same way.)
	app:click('role:textbox "Message"')
	app:frames(30)
	carried(app, true)
	expect(app:has("#chat-drop")).to_be(true)
	app:key("escape")
	app:frames(2)
	expect(app:has("#chat-drop")).to_be(false)
end)

test("the pointer back over the chat lets go of files carried over it", function()
	local app = mount("chat_fixture.luau")
	open(app)
	carried(app, true)
	expect(app:has("#chat-drop")).to_be(true)
	-- (A while after they came: a move from before can be heard late.)
	app:frames(30)
	app:hover("#chat-session")
	app:frames(2)
	expect(app:has("#chat-drop")).to_be(false)
end)

test("files carried with no word of them for a while are let go of", function()
	local app = mount("chat_fixture.luau")
	open(app)
	carried(app, true)
	app:frames(120)
	expect(app:has("#chat-drop")).to_be(true)
	app:frames(600)
	expect(app:has("#chat-drop")).to_be(false)
end)

test("a sidebar session carried over the chat is kept", function()
	local app = mount("chat_fixture.luau")
	open(app)
	carried(app, "session")
	expect(app:has("#chat-drop-mention")).to_be(true)
	app:frames(30)
	app:hover("#chat-session")
	app:frames(600)
	expect(app:has("#chat-drop-mention")).to_be(true)
end)

-- What a stage asks the host first (ui/paste_fixture.luau answers): over 24
-- MiB linked, a folder or an unreadable file refused in one line, a picture
-- numbered for the draft; a host that can't say stages as before.
local function stage(app: any, stats: { any }, files: { any })
	app:set_state("test_stats", stats)
	app:set_state("chat.composer.refused", nil)
	app:set_state("test_files", files)
	app:command("attach_test_files")
	app:frames(2)
end
local function cargo_of(app: any): { any }
	return app:state("chat.composer.cargo") or {}
end
local function page(): any
	local app = mount("paste_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	app:frames(30)
	return app
end

test("a file over 24 MiB is linked, not copied", function()
	local app = page()
	stage(app, { { path = "/x/big.mov", size = 40 * 1024 * 1024, is_file = true, readable = true } }, { { name = "big.mov", path = "/x/big.mov" } })
	local item = cargo_of(app)[1]
	expect(item.on_host).to_be(true)
	expect(item.linked).to_be(true)
	expect(item.label).to_be("big.mov · 40 MB")
	expect(item.kind).to_be("file")
	app:command("size_now")
	expect(app:state("test_size")).to_be("40 MB|1.5 GB")
	-- Exactly the limit is copied.
	stage(app, { { path = "/x/edge.bin", size = 24 * 1024 * 1024, is_file = true, readable = true } }, { { name = "edge.bin", path = "/x/edge.bin" } })
	expect(cargo_of(app)[2].on_host).to_be_nil()
	expect(app:error()).to_be_nil()
end)

test("a folder and an unreadable file are refused in one line, the good one staged", function()
	local app = page()
	stage(app, {
		{ path = "/x/dir", size = 0, is_file = false, readable = true },
		{ path = "/x/gone.txt", size = 0, is_file = false, readable = false },
		{ path = "/x/ok.txt", size = 3, is_file = true, readable = true },
	}, { { name = "dir", path = "/x/dir" }, { name = "gone.txt", path = "/x/gone.txt" }, { name = "ok.txt", path = "/x/ok.txt" } })
	expect(app:state("chat.composer.refused")).to_be("dir is not a file. gone.txt could not be read.")
	expect(#cargo_of(app)).to_be(1)
	expect(cargo_of(app)[1].name).to_be("ok.txt")
	-- The next attach clears the line.
	stage(app, { { path = "/x/b.txt", size = 3, is_file = true, readable = true } }, { { name = "b.txt", path = "/x/b.txt" } })
	expect(app:state("chat.composer.refused")).to_be_nil()
end)

test("every picture is named Image N, as gpui's staged images and its chip's link; a file keeps its name; a host too old stages as before", function()
	local app = page()
	stage(app, {}, { { name = "image.png", path = "/x/a.png", clipboard = true }, { name = "Shot.JPG", path = "/x/b.jpg" }, { name = "notes.md", path = "/x/n.md" } })
	local items = cargo_of(app)
	expect(items[1].name).to_be("Image 1.png")
	expect(items[1].number).to_be(1)
	expect(items[2].name).to_be("Image 2.jpg")
	expect(items[3].name).to_be("notes.md")
	expect(items[3].number).to_be(3)
	app:set_state("test_host", "old")
	stage(app, {}, { { name = "image.png", path = "/x/c.png", clipboard = true } })
	expect(cargo_of(app)[4].name).to_be("Image 4.png")
	expect(app:error()).to_be_nil()
end)

test("a stage answered late lands on the draft it was attached to", function()
	local app = page()
	app:set_state("test_host", "late")
	app:set_state("test_files", { { name = "a.txt", path = "/x/a.txt" } })
	app:command("attach_test_files")
	app:command("go_s1")
	app:frames(30)
	app:command("answer_late")
	app:frames(2)
	expect(#cargo_of(app)).to_be(0)
	app:command("draft_of_s2")
	expect(app:state("test_chip")).to_be(string.rep("\u{a0}", 6) .. "a.txt\u{a0}\u{a0} |a.txt")
end)

test("a second attach before the host answers the first takes the next number", function()
	local app = page()
	app:set_state("test_host", "late")
	app:set_state("test_files", { { name = "a.png", path = "/x/a.txt" } })
	app:command("attach_test_files")
	app:set_state("test_host", "new")
	stage(app, {}, { { name = "b.png", path = "/x/b.png" } })
	app:command("answer_late")
	app:frames(2)
	local items = cargo_of(app)
	expect(items[1].name).to_be("Image 2.png")
	expect(items[1].number).to_be(2)
	expect(items[2].name).to_be("Image 1.png")
	expect(items[2].number).to_be(1)
end)
