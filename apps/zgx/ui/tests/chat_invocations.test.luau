-- The composer's `/` commands and `$` skills (screens/chat/invocations),
-- on the chat page (ui/chat_fixture.luau, the host's list stubbed): each
-- list comes from the agent's, read as a `/` or `$` opens; it narrows as
-- the name is typed; the agent's settings gate `$` and fold the skills
-- into `/`; a pick lands as its token and goes out as its reference.

local function settle(app: any)
	app:frames(30)
end

local function chat(): any
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	settle(app)
	return app
end

local function reads(app: any): number
	local n = 0
	for _ in string.gmatch(app:state("calls") or "", "invocations:") do
		n += 1
	end
	return n
end

local PREFS = "screens/settings/prefs.luau::settings_skill_completion"
local DISABLED = "screens/chat/invocations.luau::skills_disabled"

test("`/` lists the agent's commands, read as it opens, narrowing as the name is typed", function()
	local app = chat()
	local before = reads(app)
	app:type("role:textbox", "/")
	settle(app)
	expect(app:state("calls")).to_contain("invocations:claude-code:s2")
	expect(app:has('role:option "/review"')).to_be(true)
	expect(app:has('role:option "/compact"')).to_be(true)
	expect(app:has('~"Review the diff · <branch>"')).to_be(true)
	-- (Separate by default: the skills wait for `$`.)
	expect(app:has('role:option "Code Review"')).to_be(false)
	-- (Set, not pressed and typed: a press on the pill puts the menu away.)
	app:set_state("chat.composer.text", "/rev")
	settle(app)
	expect(app:has('role:option "/review"')).to_be(true)
	expect(app:has('role:option "/compact"')).to_be(false)
	-- Typed further at, no new read: one per opening.
	expect(reads(app)).to_be(before + 1)
end)

test("`$` lists the skills; with the agent's `$` off it's plain text, and with `/` not separate, `/` lists them too", function()
	local app = chat()
	app:type("role:textbox", "$")
	settle(app)
	expect(app:has('role:option "Code Review"')).to_be(true)
	expect(app:has('role:option "/review"')).to_be(false)
	-- Turned off on the Skills page (by its path): not offered.
	app:set_state(DISABLED, { ["/s/code-review/SKILL.md"] = true })
	settle(app)
	expect(app:has('role:option "Code Review"')).to_be(false)
	app:set_state(DISABLED, {})
	app:key("Escape")
	app:fill("role:textbox", "")
	settle(app)
	app:set_state(PREFS, { ["claude-code"] = { dollar = false, separate = false } })
	settle(app)
	app:type("role:textbox", "$")
	settle(app)
	expect(app:has('role:option "Code Review"')).to_be(false)
	app:fill("role:textbox", "/")
	settle(app)
	expect(app:has('role:option "/review"')).to_be(true)
	expect(app:has('role:option "Code Review"')).to_be(true)
	app:set_state(PREFS, {})
end)

test("a command picked lands as its token and is sent as its reference; one typed by hand stays words", function()
	local app = chat()
	app:type("role:textbox", "/rev")
	settle(app)
	app:key("Enter")
	settle(app)
	expect(app:text("role:textbox")).to_be(string.rep("\u{a0}", 6) .. "/review" .. string.rep("\u{a0}", 2) .. " ")
	app:type("role:textbox", "the branch, then /compact by hand")
	settle(app)
	-- (No menu over it: Enter sends. Escape here would give the keyboard
	-- back to the window.)
	expect(app:has("role:option")).to_be(false)
	app:key("Enter")
	settle(app)
	-- (It goes to the host as it joins the transcript.)
	expect(app:wait('~"the branch"', { timeout_ms = 2000 })).to_be(true)
	local calls = app:state("calls")
	expect(calls).to_contain("send:s2:[/review](zeron-invoke:7b226b696e64223a22636f6d6d616e64222c226e616d65223a22726576696577227d) the branch, then /compact by hand")
end)

test("a draft whose last word is a trigger nothing matches still sends on Enter", function()
	for _, words in { "look in /tmp", "echo $HOME", "ping @nobody", "/nocommand" } do
		local app = chat()
		app:type("role:textbox", words)
		settle(app)
		-- (Its list shows, with nothing in it; a `$` that is a variable opens
		-- none, mentions.test.)
		expect(app:has("#mention-popover")).to_be(words ~= "echo $HOME")
		expect(app:has("role:option")).to_be(false)
		app:key("Enter")
		settle(app)
		expect(app:text("role:textbox")).to_be("")
		expect(app:state("calls")).to_contain(`send:s2:{words}`)
	end
end)

test("Zeron's own commands end the `/` list, named clear of the agent's, and only a chat's in a chat", function()
	local app = chat()
	app:type("role:textbox", "/")
	settle(app)
	expect(app:has('role:option "/model"')).to_be(true)
	expect(app:has('role:option "/stop"')).to_be(true)
	expect(app:has('~"Zeron: open settings"')).to_be(true)
	-- The agent has a `/new`: it keeps it, and Zeron's is `/zeron:new`.
	expect(app:has('role:option "/new"')).to_be(true)
	expect(app:has('role:option "/zeron:new"')).to_be(true)
	-- A new chat (no session): none of a chat's own.
	app:key("Escape")
	app:fill("role:textbox", "")
	app:command("go_home")
	settle(app)
	app:type("role:textbox", "/")
	settle(app)
	expect(app:has('role:option "/model"')).to_be(true)
	expect(app:has('role:option "/stop"')).to_be(false)
	expect(app:has('role:option "/rename"')).to_be(false)
end)

test("one of Zeron's own picked runs at once, its token gone and the rest kept; a draft of only it runs as it's sent", function()
	local app = chat()
	app:type("role:textbox", "Keep this /sto")
	settle(app)
	app:key("Enter")
	settle(app)
	expect(app:state("calls")).to_contain("interrupt:s2")
	expect(app:text("role:textbox")).to_be("Keep this ")
	expect(app:state("calls")).never.to_contain("send:")
	app:fill("role:textbox", "/stop")
	settle(app)
	app:key("Escape")
	app:key("Enter")
	settle(app)
	local _, n = string.gsub(app:state("calls"), "interrupt:s2", "")
	expect(n).to_be(2)
	expect(app:state("calls")).never.to_contain("send:")
	expect(app:text("role:textbox")).to_be("")
end)

-- (`/rename` opens the sidebar's rename, which reads the host's places:
-- the fixture has none, so it's left to the app.)
test("every one of Zeron's own runs from the chat page without an error", function()
	-- (The fixture's agent has a `/new` of its own, so Zeron's is
	-- `/zeron:new`: `/new` goes to the agent, as it should.)
	for _, name in { "model", "resume", "settings", "files", "terminal", "zeron:new" } do
		local app = chat()
		app:type("role:textbox", `/{name}`)
		settle(app)
		app:key("Escape")
		app:key("Enter")
		settle(app)
		expect(app:error()).to_be_nil()
		expect(app:state("calls")).never.to_contain("send:")
	end
end)

-- The catalog's states and flags, on a plain field (completions_fixture.luau).

local function plain(): any
	local app = mount("completions_fixture.luau")
	app:resize(900, 700)
	app:frames(5)
	return app
end

-- Whether the menu shows `target` (a closed popover keeps its rows, hidden).
local function shown(app: any, target: string): boolean
	if not app:has(target) or not app:has("#mention-popover") then
		return false
	end
	return string.find(string.match(app:tree("#mention-popover"), "^[^\n]*") or "", " hidden", 1, true) == nil
end

-- A new draft typed afresh (the popover follows the caret the field reports).
local function retype(app: any, text: string)
	app:fill("#field", "")
	app:frames(1)
	app:type("#field", text)
end

local function box_of(app: any, target: string): (number, number)
	local x, y = string.match(app:tree(target), "(%-?[%d%.]+),(%-?[%d%.]+) %d")
	assert(x and y, `no box for {target}`)
	return tonumber(x) :: number, tonumber(y) :: number
end

test("a second use of a picked token opens its menu; the picked one stays quiet", function()
	local app = plain()
	app:type("#field", "/compact")
	app:frames(3)
	app:key("Enter")
	app:frames(3)
	expect(app:text("#field")).to_be("/compact ")
	expect(app:has('role:option "/compact"')).to_be(false)
	app:type("#field", "and /compact")
	app:frames(3)
	expect(app:has('role:option "/compact"')).to_be(true)
end)

test("the catalog: rows flagged plain go in as words", function()
	local app = plain()
	app:type("#field", "/compact")
	app:frames(3)
	app:key("Enter")
	app:frames(3)
	expect(app:text("#field")).to_be("/compact ")
	app:command("encode")
	expect(app:state("calls")).to_contain("encoded:/compact ;")
	retype(app, "/rev")
	app:frames(3)
	app:key("Enter")
	app:frames(3)
	app:command("encode")
	expect(app:state("calls")).to_contain("encoded:[/review](zeron-invoke:7b7d) ;")
end)

test("the catalog: Zeron's own rows stay after the agent's", function()
	local app = plain()
	app:type("#field", "/")
	app:frames(3)
	local _, agent_y = box_of(app, 'role:option "/doctor"')
	local _, zeron_y = box_of(app, 'role:option "/model"')
	expect(agent_y < zeron_y).to_be(true)
end)

test("the catalog: rows read before show while it's read again, and an empty list says so by mode", function()
	local app = plain()
	app:set_state("reply", "rows")
	app:set_state("rows", { { kind = "command", name = "review", label = "/review", detail = "", link = "x" } })
	app:set_state("place", "s9")
	retype(app, "/")
	app:frames(3)
	app:fill("#field", "")
	app:frames(2)
	app:set_state("reply", "hold")
	retype(app, "/")
	app:frames(3)
	expect(app:state("calls")).to_contain("invocations:s9;invocations:s9;")
	expect(shown(app, 'role:option "/review"')).to_be(true)
	retype(app, "/zzz")
	app:frames(3)
	expect(shown(app, '~"No matching commands"')).to_be(true)
	retype(app, "$")
	app:frames(3)
	expect(shown(app, '~"No skills available"')).to_be(true)
end)

