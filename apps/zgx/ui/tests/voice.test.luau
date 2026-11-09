-- Voice (screens/settings/voice) and dictation (ds/dictation, the
-- composer's mic), on the shell in ui/shell_fixture.luau with
-- ui/voice_stub.luau's fake microphone standing in for the host's: the
-- model's download and Remove, the microphone kept when unplugged, the
-- Codex voice rows; holding the mic writes what was heard into the draft,
-- a tap says to hold, silence and a refused microphone say so, Escape and a
-- changed draft let go, the shortcut holds as the mic does.

local PAGE = "#settings-page"
local DRAFT = 'role:textbox "Message"'

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

local function shell(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 1200)
	app:frames(30)
	return app
end
local function voice_page(app: any)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> "Voice"')
	app:frames(30)
end
local function calls(app: any): string
	return table.concat(app:state("voice_stub_calls") or {}, ",")
end
local function has(app: any, words: string): boolean
	return app:tree(PAGE):find(words, 1, true) ~= nil
end
local function checked(app: any, label: string): boolean
	for line in app:tree(PAGE):gmatch("[^\n]+") do
		if line:find(`switch "{label}`, 1, true) then
			return line:find("checked", 1, true) ~= nil and line:find("unchecked", 1, true) == nil
		end
	end
	error(`no switch {label}`)
end
-- Held a while, then let go: the mic, as a person holds it.
local function hold(app: any)
	app:command("voice_stub.press")
	expect(eventually(app, function()
		return app:tree("#chat-composer"):find("Listening", 1, true) ~= nil
	end)).to_be(true)
	app:wait('"never shown, only waited past"', { timeout_ms = 400 })
	app:command("voice_stub.release")
end

test("voice settings: gpui's rows; the switch downloads the model first, then turns Dictation on; Remove takes it away", function()
	local app = shell()
	voice_page(app)
	for _, w in { "Voice conversation", "Codex voice device", "This device", "Codex voice style", "Codex default", "Dictation", "670 MB download", "Shortcut", "Hold to talk, release to transcribe" } do
		expect(`{w}: {has(app, w)}`).to_be(`{w}: true`)
	end
	expect(has(app, "Microphone")).to_be(false)
	expect(has(app, "Speech model")).to_be(false)
	expect(checked(app, "Dictation")).to_be(false)
	app:click(`{PAGE} >> role:switch "Dictation"`)
	expect(eventually(app, function()
		return has(app, "Downloading · 168 of 670 MB")
	end)).to_be(true)
	expect(app:has("#voice-progress")).to_be(true)
	expect(checked(app, "Dictation")).to_be(true)
	-- Pressed while downloading: Cancel.
	app:click(`{PAGE} >> role:switch "Dictation"`)
	expect(eventually(app, function()
		return has(app, "Download cancelled.")
	end)).to_be(true)
	expect(calls(app)).to_contain("voice.cancel")
	expect(checked(app, "Dictation")).to_be(false)
	app:click(`{PAGE} >> role:switch "Dictation"`)
	-- (The page left while it downloads: Dictation still turns on as it ends.)
	app:click('#settings-nav >> "General"')
	app:frames(10)
	app:command("voice_stub.download_done")
	app:frames(5)
	app:click('#settings-nav >> "Voice"')
	app:frames(30)
	expect(eventually(app, function()
		return has(app, "Microphone") and has(app, "Parakeet v3 · 670 MB")
	end)).to_be(true)
	expect(checked(app, "Dictation")).to_be(true)
	expect(app:state("dictation_enabled")).to_be(true)
	-- Off keeps the model.
	app:click(`{PAGE} >> role:switch "Dictation"`)
	app:frames(5)
	expect(checked(app, "Dictation")).to_be(false)
	expect(has(app, "Parakeet v3")).to_be(true)
	expect(has(app, "Microphone")).to_be(false)
	app:click(`{PAGE} >> role:switch "Dictation"`)
	app:frames(5)
	expect(checked(app, "Dictation")).to_be(true)
	app:click("#voice-remove")
	expect(eventually(app, function()
		return not has(app, "Parakeet v3") and has(app, "670 MB download")
	end)).to_be(true)
	expect(checked(app, "Dictation")).to_be(false)
	expect(app:state("dictation_enabled")).to_be(false)
end)

test("voice settings: the microphone lists the system's first, a saved one unplugged stays as Disconnected; the voice style is kept", function()
	local app = shell()
	app:command("voice_stub.dictation_on")
	app:set_state("dictation_input", "mock:gone")
	voice_page(app)
	expect(eventually(app, function()
		return has(app, "Disconnected microphone")
	end)).to_be(true)
	app:click(`{PAGE} >> role:button "Microphone"`)
	app:frames(10)
	for _, w in { "System default", "Mock Microphone", "USB Microphone", "Disconnected microphone" } do
		expect(`{w}: {app:has(`role:menuitem "{w}"`)}`).to_be(`{w}: true`)
	end
	app:click('role:menuitem "USB Microphone"')
	app:frames(10)
	expect(app:state("dictation_input")).to_be("mock:usb")
	-- (Opened again, the list is built anew: the unplugged one has gone.)
	app:click(`{PAGE} >> role:button "Microphone"`)
	app:frames(10)
	expect(app:has('role:menuitem "USB Microphone"')).to_be(true)
	expect(app:has('role:menuitem "Disconnected microphone"')).to_be(false)
	app:key("Escape")
	app:frames(10)
	app:click(`{PAGE} >> role:button "Codex voice style"`)
	app:frames(10)
	app:click('role:menuitem "Maple"')
	app:frames(10)
	expect(app:state("codex_voice")).to_be("maple")
end)

test("the composer's mic: only once Dictation is on; held, it listens, and let go writes what was heard into the draft", function()
	local app = shell()
	expect(app:has('role:button "Hold to dictate"')).to_be(false)
	app:command("voice_stub.dictation_on")
	app:frames(10)
	expect(app:has('role:button "Hold to dictate"')).to_be(true)
	app:type(DRAFT, "Note:")
	app:click('role:button "Hold to dictate"')
	app:frames(5)
	-- (A click is a tap: hold to talk, as gpui's.)
	expect(eventually(app, function()
		return app:has("#dictation-status")
	end)).to_be(true)
	expect(app:tree("#dictation-status")).to_contain("Keep holding the microphone or the shortcut while you speak.")
	app:click("#dictation-dismiss")
	expect(eventually(app, function()
		return not app:has("#dictation-status")
	end)).to_be(true)
	hold(app)
	expect(eventually(app, function()
		return app:tree(DRAFT):find("Note: Hello from a fake microphone", 1, true) ~= nil
	end)).to_be(true)
	expect(calls(app)).to_contain("finish:2")
	expect(app:has("#dictation-status")).to_be(false)
	expect(app:has('role:button "Hold to dictate"')).to_be(true)
end)

test("dictation: silence says No speech detected, a refused microphone says so, Escape and a changed draft let go", function()
	local app = shell()
	app:command("voice_stub.dictation_on")
	app:frames(10)
	app:command("voice_stub.silence")
	hold(app)
	expect(eventually(app, function()
		return app:has("#dictation-status")
	end)).to_be(true)
	expect(app:tree("#dictation-status")).to_contain("No speech detected")
	app:command("voice_stub.deny")
	app:command("voice_stub.press")
	expect(eventually(app, function()
		return app:has("#dictation-status") and app:tree("#dictation-status"):find("Dictation unavailable", 1, true) ~= nil
	end)).to_be(true)
	expect(app:tree("#dictation-status")).to_contain("Allow Microphone access")
	app:command("voice_stub.release")
	-- Escape while listening: cancelled, nothing written, nothing said.
	app:command("voice_stub.press")
	expect(eventually(app, function()
		return app:tree("#chat-composer"):find("Listening", 1, true) ~= nil
	end)).to_be(true)
	app:click(DRAFT)
	app:key("Escape")
	expect(eventually(app, function()
		return calls(app):find("cancel:3", 1, true) ~= nil
	end)).to_be(true)
	app:command("voice_stub.release")
	app:frames(5)
	expect(app:has("#dictation-status")).to_be(false)
	-- The draft changed under it: it lets go and writes nothing.
	app:command("voice_stub.press")
	expect(eventually(app, function()
		return app:tree("#chat-composer"):find("Listening", 1, true) ~= nil
	end)).to_be(true)
	app:type(DRAFT, "typed")
	expect(eventually(app, function()
		return calls(app):find("cancel:4", 1, true) ~= nil
	end)).to_be(true)
	app:command("voice_stub.release")
	app:frames(10)
	expect(app:tree(DRAFT)).never.to_contain("fake microphone")
end)

test("dictation: the hold shortcut starts it from the draft, the microphone chosen is the one asked for", function()
	local app = shell()
	app:command("voice_stub.dictation_on")
	app:set_state("dictation_input", "mock:usb")
	app:frames(10)
	app:click(DRAFT)
	app:key("cmd+d")
	expect(eventually(app, function()
		return calls(app):find("start:mock:usb", 1, true) ~= nil
	end)).to_be(true)
	-- (Down and up at once: a tap.)
	expect(eventually(app, function()
		return app:has("#dictation-status")
	end)).to_be(true)
	expect(app:tree("#dictation-status")).to_contain("Hold to dictate")
end)

test("Settings → Shortcuts lists Hold to dictate under Voice, refuses its keys for another, and sends you to Voice to change it", function()
	local app = shell()
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> "Shortcuts"')
	app:frames(30)
	expect(has(app, "Hold to dictate")).to_be(true)
	-- (Its keys as this platform writes them: ⌘D on the Mac, Ctrl D here.)
	local keys = app:tree("#setting-key_dictation")
	expect(keys:find('img "Ctrl D"', 1, true) ~= nil or keys:find('img "Command D"', 1, true) ~= nil).to_be(true)
	app:click(`{PAGE} >> role:button ~"New chat"`)
	app:frames(10)
	app:key("cmd+d")
	app:frames(10)
	expect(app:state("settings_shortcuts_notice")).to_contain(" is already assigned to Hold to dictate in Settings → Voice.")
	app:scroll(`{PAGE} >> role:heading "Shortcuts"`, 3000)
	app:frames(4)
	app:click(`{PAGE} >> role:button "Change Hold to dictate in Voice"`)
	app:frames(30)
	expect(has(app, "Hold to talk, release to transcribe")).to_be(true)
	-- Voice off: the row goes, and its keys are anyone's.
	app:command("voice_stub.builtin_off")
	app:click('#settings-nav >> "Shortcuts"')
	app:frames(30)
	expect(app:has("#setting-key_dictation")).to_be(false)
	app:command("voice_stub.builtin_on")
	app:frames(10)
	expect(app:has("#setting-key_dictation")).to_be(true)
end)

test("Voice off takes the mic and Settings → Voice away; on brings them back", function()
	local app = shell()
	app:command("voice_stub.dictation_on")
	app:frames(10)
	expect(app:has('role:button "Hold to dictate"')).to_be(true)
	app:command("voice_stub.builtin_off")
	app:frames(10)
	expect(app:has('role:button "Hold to dictate"')).to_be(false)
	app:command("shell.settings")
	app:frames(10)
	expect(app:has('#settings-nav >> "Voice"')).to_be(false)
	app:command("voice_stub.builtin_on")
	app:frames(10)
	expect(app:has('#settings-nav >> "Voice"')).to_be(true)
end)
