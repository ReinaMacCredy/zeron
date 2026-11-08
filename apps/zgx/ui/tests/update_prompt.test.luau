-- "Check for Updates…" (Zeron's update prompt) on ui/shell_fixture.luau: the host's `about` and its
-- release feed stood in for (ui/settings_stub.luau), so nothing is downloaded or installed.

local DIALOG = "role:dialog"

-- Mounted, then the command: its dialog (and About's module with it) loads, and `about` with it.
local function open(fields: { [string]: any }): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("shell.check_updates")
	app:frames(10)
	app:set_state("about", (function()
		local now = { app_version = "0.2.131", app_build = "1793000000", available = false }
		for k, v in fields do
			now[k] = v
		end
		return now
	end)())
	app:frames(6)
	return app
end

local function about(app: any, fields: { [string]: any })
	local now = { app_version = "0.2.131", app_build = "1793000000", available = false }
	for k, v in fields do
		now[k] = v
	end
	app:set_state("about", now)
	app:frames(6)
end

-- (Put away, it stays in the tree, hidden, until the shell is made again.)
local function shown(app: any): boolean
	return app:has(DIALOG) and string.find(string.match(app:tree(DIALOG), "^[^\n]*") or "", " hidden", 1, true) == nil
end

local function says(app: any, text: string): boolean
	return shown(app) and app:tree(DIALOG):find(text, 1, true) ~= nil
end

local function asked(app: any, entry: string): number
	local n = 0
	for _, c in (app:state("settings_stub_calls") :: any) or {} do
		if c == entry then
			n += 1
		end
	end
	return n
end

local function buttons(app: any): string
	local out = {}
	for name in app:tree("#update-prompt-buttons"):gmatch('button "([^"]+)"') do
		table.insert(out, name)
	end
	return table.concat(out, ",")
end

test("Check for Updates… is a File menu command that loads its dialog only when asked, checks at once, and Cancel puts it away", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	expect(app:state("screens/engine/update.luau::update_prompt_loaded")).to_be(false)
	expect(shown(app)).to_be(false)
	app:set_state("settings_stub_about_next", "pending")
	app:command("shell.check_updates")
	app:frames(10)
	expect(app:state("screens/engine/update.luau::update_prompt_loaded")).to_be(true)
	expect(asked(app, "about.check")).to_be(1)
	expect(says(app, "Checking for updates…")).to_be(true)
	expect(buttons(app)).to_be("Cancel")
	app:click(`#update-prompt-buttons >> role:button "Cancel"`)
	app:frames(20)
	expect(shown(app)).to_be(false)
end)

test("a loaded flag left by a script reload builds nothing: the prompt loads from the command's timer alone, never in a build", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:set_state("screens/engine/update.luau::update_prompt_loaded", true)
	app:frames(10)
	-- (About's module, which the prompt's brings, unloaded: its state not yet made.)
	expect(app:state("screens/settings/about.luau::about_beta")).to_be(nil)
	expect(app:has(DIALOG)).to_be(false)
	app:set_state("settings_stub_about_next", "pending")
	app:command("shell.check_updates")
	app:frames(10)
	expect(says(app, "Checking for updates…")).to_be(true)
	expect(app:state("screens/settings/about.luau::about_beta")).to_be(false)
end)

test("the app's own update: Later or Update, Hide while it downloads, then Restart to update, as About says", function()
	local app = open({ self_updating = true, flow = "idle", latest = "0.2.140", available = true })
	expect(says(app, "Zeron 0.2.140 is available")).to_be(true)
	expect(says(app, "You have 0.2.131.")).to_be(true)
	expect(buttons(app)).to_be("Later,Update")
	app:click(`#update-prompt-buttons >> role:button "Update"`)
	app:frames(6)
	expect(asked(app, "about.apply")).to_be(1)
	about(app, { self_updating = true, flow = "downloading", flow_version = "0.2.140", latest = "0.2.140", available = true })
	expect(says(app, "Downloading Zeron 0.2.140…")).to_be(true)
	expect(buttons(app)).to_be("Hide")
	about(app, { self_updating = true, flow = "ready", flow_version = "0.2.140", latest = "0.2.140", available = true })
	expect(says(app, "Zeron 0.2.140 is ready")).to_be(true)
	expect(buttons(app)).to_be("Later,Restart to update")
	app:click(`#update-prompt-buttons >> role:button "Restart to update"`)
	app:frames(6)
	expect(asked(app, "about.apply")).to_be(2)
	-- A download that failed: Later and Try again, which downloads again.
	about(app, { self_updating = true, flow = "failed", flow_version = "0.2.140", flow_error = "disk full", latest = "0.2.140", available = true })
	expect(says(app, "Couldn't download Zeron 0.2.140")).to_be(true)
	expect(says(app, "disk full")).to_be(true)
	expect(buttons(app)).to_be("Later,Try again")
	-- One it can't install here: why, and OK.
	about(app, { self_updating = true, flow = "idle", held = "Needs macOS 14 or later.", latest = "0.2.150", available = false })
	expect(says(app, "Zeron 0.2.150 can't be installed yet")).to_be(true)
	expect(buttons(app)).to_be("OK")
	app:click(`#update-prompt-buttons >> role:button "OK"`)
	app:frames(20)
	expect(shown(app)).to_be(false)
end)

test("a check that fails offers Close and Try again; up to date, OK alone; Escape puts it away", function()
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:set_state("settings_stub_about_next", "offline")
	app:command("shell.check_updates")
	app:frames(10)
	about(app, { engine_version = "0.2.131" })
	expect(says(app, "Couldn't check for updates")).to_be(true)
	expect(says(app, "This computer seems to be offline.")).to_be(true)
	expect(buttons(app)).to_be("Close,Try again")
	app:set_state("settings_stub_about_next", "ok")
	app:click(`#update-prompt-buttons >> role:button "Try again"`)
	app:frames(10)
	expect(asked(app, "about.check")).to_be(2)
	about(app, { engine_version = "0.2.131", latest = "0.2.131", checked_at = os.time() * 1000 })
	expect(says(app, "Zeron is up to date")).to_be(true)
	expect(buttons(app)).to_be("OK")
	app:key("Escape")
	app:frames(20)
	expect(shown(app)).to_be(false)
end)
