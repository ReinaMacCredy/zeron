-- Voice (screens/settings/voice) around dictation (screens/chat/dictation,
-- upstream's), on the shell in ui/shell_fixture.luau: the Voice built-in's
-- switch takes the microphone, the hold shortcut and its Shortcuts row
-- away and brings them back, the page stays listed; Codex's voice
-- conversation rows keep what's chosen. (Dictation itself, its download and
-- its words in the draft: tests/dictation.rs and voice_wave.test.luau.)

local PAGE = "#settings-page"
local MIC = 'role:button "Hold to dictate"'

local function shell(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1200)
	app:frames(30)
	return app
end
local function settings(app: any, page: string)
	app:command("shell.settings")
	app:frames(10)
	app:click(`#settings-nav >> role:tab "{page}"`)
	app:frames(30)
end
local function has(app: any, words: string): boolean
	return app:tree(PAGE):find(words, 1, true) ~= nil
end

test("voice settings: the Voice switch, Codex's conversation rows and Dictation's; the voice style is kept", function()
	local app = shell()
	settings(app, "Voice")
	for _, words in { "Voice conversation", "Codex voice device", "Codex voice style", "Dictation", "Microphone" } do
		expect(if has(app, words) then words else `missing: {words}`).to_be(words)
	end
	expect(app:tree(PAGE):match('switch "Voice"[^\n]*') or "").to_contain("checked")
	app:click(`{PAGE} >> role:button "Codex voice style"`)
	app:frames(10)
	app:click('role:menuitem "Maple"')
	app:frames(10)
	expect(app:state("codex_voice")).to_be("maple")
	expect(app:error()).to_be_nil()
end)

test("Voice off takes the mic away; Settings → Voice stays, its Voice switch brings it back", function()
	local app = shell()
	expect(app:has(MIC)).to_be(true)
	app:command("voice_stub.builtin_off")
	app:frames(10)
	expect(app:has(MIC)).to_be(false)
	settings(app, "Voice")
	expect(app:tree(PAGE):match('switch "Voice"[^\n]*') or "").to_contain("unchecked")
	app:click(`{PAGE} >> role:switch "Voice"`)
	app:frames(20)
	app:command("shell.settings")
	app:frames(10)
	expect(app:has(MIC)).to_be(true)
end)

test("Voice off: the hold shortcut starts nothing; on, with Dictation off, it opens Settings → Voice", function()
	local app = shell()
	app:command("voice_stub.builtin_off")
	app:frames(10)
	app:command("shell.dictate")
	app:frames(10)
	expect(tostring(app:state("shell_route_path"))).never.to_contain("/settings")
	app:command("voice_stub.builtin_on")
	app:frames(10)
	app:command("shell.dictate")
	app:frames(20)
	expect(tostring(app:state("shell_route_path"))).to_contain("/settings/voice")
end)

test("Settings → Shortcuts lists Hold to dictate while Voice is on, and leaves it out while it's off", function()
	local app = shell()
	settings(app, "Shortcuts")
	expect(has(app, "Hold to dictate")).to_be(true)
	app:command("voice_stub.builtin_off")
	app:frames(10)
	expect(has(app, "Hold to dictate")).to_be(false)
	app:command("voice_stub.builtin_on")
	app:frames(10)
	expect(has(app, "Hold to dictate")).to_be(true)
end)
