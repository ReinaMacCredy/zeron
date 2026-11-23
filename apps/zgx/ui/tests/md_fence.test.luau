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

-- Shell blocks run in the chat's terminal (ui/md_run_fixture.luau: its runner keeps what it's given).
local function shells(): any
	local app = mount("md_run_fixture.luau")
	app:resize(900, 1400)
	app:frames(20)
	expect(app:error()).to_be_nil()
	return app
end

local RUN = 'role:button "Run in terminal"'

test("Run shows on a shell's and a session's block with a prompt, not on text, unlabeled, rust or json", function()
	local app = shells()
	local runs = 0
	for _ in string.gmatch(app:tree("#reply"), 'button "Run in terminal"') do
		runs += 1
	end
	expect(runs).to_be(3)
	for _, lang in { "bash", "sh" } do
		expect(app:has(`.lang-{lang} >> {RUN}`)).to_be(true)
	end
	for _, lang in { "text", "rust", "json" } do
		expect(app:has(`.lang-{lang} >> {RUN}`)).to_be(false)
	end
	expect(app:has(`.lang-rust >> role:button "Copy code"`)).to_be(true)
end)

test("Run hands over the block's exact text, and of a session only the commands after its prompts", function()
	local app = shells()
	app:click(`.lang-bash >> {RUN}`)
	app:frames(5)
	app:click(`.lang-console >> {RUN}`)
	app:frames(5)
	local ran = app:state("test_ran")
	expect(#ran).to_be(2)
	expect(ran[1]).to_be("cargo test -p zeron-engine\tquiet")
	expect(ran[2]).to_be("cd /tmp\nmake all\n./run\nprintf '%s\\n' \\\nhello")
end)

test("Copy of a long command wrapped to fit gives its line as written", function()
	local app = shells()
	app:click('.lang-sh >> role:checkbox "Fit content"')
	app:frames(10)
	app:click('.lang-sh >> role:button "Copy code"')
	app:frames(5)
	expect(app:clipboard()).to_be("curl -fsSL https://example.com/install.sh | env PREFIX=$HOME/.local VERSION=1.2.3 CHANNEL=stable sh -s -- --yes --no-modify-path --quiet")
	app:click('.lang-bash >> role:button "Copy code"')
	app:frames(5)
	expect(app:clipboard()).to_be("cargo test -p zeron-engine\tquiet")
end)

local PY_RUN = '.lang-python >> role:button "Run Python"'
local PY_STOP = '.lang-python >> role:button "Stop Python"'
local PY_CLEAR = '.lang-python >> role:button "Clear result"'

test("a python block runs only on a press, shows running then its result, stops, and clears", function()
	local app = shells()
	app:resize(900, 2000)
	app:frames(10)
	expect(#app:state("test_py_asked")).to_be(0)
	expect(app:has(PY_RUN)).to_be(true)
	expect(app:has(PY_CLEAR)).to_be(false)
	expect(app:has('.lang-py >> "restored"')).to_be(true)
	expect(app:has('.lang-py >> role:button "Clear result"')).to_be(true)
	app:click(PY_RUN)
	app:frames(5)
	expect(app:state("test_py_asked")[1]).to_be("run:print('hit')")
	expect(app:has(PY_STOP)).to_be(true)
	expect(app:has('.lang-python >> "Running…"')).to_be(true)
	expect(app:has(PY_CLEAR)).to_be(false)
	local runs = table.clone(app:state("test_py_runs"))
	runs["print('hit')"] = { state = "done", output = "hit=57.4%\n", ms = 1840, exit = 0, cut = true }
	app:set_state("test_py_runs", runs)
	app:frames(5)
	expect(app:has('.lang-python >> "hit=57.4%\n(Output cut at 64 KB.)"')).to_be(true)
	expect(app:has('.lang-python >> "Done in 1.8 s, exit 0"')).to_be(true)
	expect(app:has(PY_RUN)).to_be(true)
	app:click(PY_RUN)
	app:frames(5)
	app:click(PY_STOP)
	app:frames(5)
	expect(app:state("test_py_asked")[3]).to_be("stop:print('hit')")
	expect(app:has('.lang-python >> "Stopped"')).to_be(true)
	expect(app:has('.lang-python >> "partial"')).to_be(true)
	runs = table.clone(app:state("test_py_runs"))
	runs["print('hit')"] = { state = "timed_out", output = "", ms = 10000, exit = 137 }
	app:set_state("test_py_runs", runs)
	app:frames(5)
	expect(app:has('.lang-python >> "Timed out after 10 s"')).to_be(true)
	app:click(PY_CLEAR)
	app:frames(5)
	expect(app:has('.lang-python >> "Timed out after 10 s"')).to_be(false)
	expect(app:has(PY_CLEAR)).to_be(false)
	expect(app:has(PY_RUN)).to_be(true)
end)

test("a python block has no Run where the host can't run python, its Copy and kept result stay", function()
	local app = shells()
	app:resize(900, 2000)
	app:set_state("test_py_available", false)
	app:frames(10)
	expect(app:has(PY_RUN)).to_be(false)
	expect(app:has('.lang-py >> role:button "Run Python"')).to_be(false)
	expect(app:has('.lang-python >> role:button "Copy code"')).to_be(true)
	expect(app:has('.lang-py >> "restored"')).to_be(true)
	expect(#app:state("test_py_asked")).to_be(0)
end)
