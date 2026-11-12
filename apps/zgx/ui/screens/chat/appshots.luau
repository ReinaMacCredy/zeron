-- Appshots, as the shipping app has them (crates/ui appshots.rs): the
-- window in front of another app, captured on a global shortcut by the host
-- (src/appshots: off the window's thread), lands on a composer as a chip.
-- It is never sent on its own; sent, its picture goes as an attachment and
-- the app's name, window title and (with Accessibility) its text go after
-- the words as untrusted observed content (the host writes that block).
--
-- Where a capture lands (Settings → Appshots → Destination):
-- - Automatic (Synara's rule, the shipping app's `resolve_automatic_target`):
--   the session you were in within the last minute, else the one the last
--   Appshot went to within the last minute, else a new chat's composer;
-- - Last session: the open session, or back to the last one an Appshot
--   went to;
-- - New session: a new chat's composer, the drafts there are kept.
-- What's staged is held to 96 MB of Appshots, as the shipping app's.
--
-- The choices are the engine's `ui-settings.json`, where the shipping app
-- keeps them (the host reads and writes them: src/appshots), so both apps
-- read the same ones. The hotkey is a double-modifier chord or none.

local route = require("@app/screens/shell/route")
local shared = require("@app/screens/chat/shared")
local toast = require("@ds/toast")
local host = require("@app/screens/settings/host")

local M = {}

M.state = ui.shared("appshots.state", nil :: any)
local captured = ui.shared("appshots.captured", nil :: any)
local failed = ui.shared("appshots.failed", nil :: any)
local quick = ui.shared("appshots.quick", nil :: any)

M.NO_HOTKEY = "none"
local GLYPHS: { [string]: string } = { ["command-command"] = "⌘", ["option-option"] = "⌥", ["shift-shift"] = "⇧" }
-- The shipping app's defaults (crates/ui settings.rs).
M.DEFAULTS = { enabled = false, sound = true, hotkey = "command-command", quick_capture = "option-option", destination = "automatic" }

M.enabled = ui.state("appshots_enabled", M.DEFAULTS.enabled)
M.sound = ui.state("appshots_sound", M.DEFAULTS.sound)
M.hotkey = ui.state("appshots_hotkey", M.DEFAULTS.hotkey)
M.quick_capture = ui.state("appshots_quick_capture", M.DEFAULTS.quick_capture)
M.destination = ui.state("appshots_destination", M.DEFAULTS.destination)
local KEYS = { "enabled", "sound", "hotkey", "quick_capture", "destination" }
-- The last session an Appshot went to (Last session's).
local last_session = ui.persist("appshots_last_session", "")
-- Automatic's two clues: where you last were, and where the last Appshot went ({ session, at }).
local interaction = ui.state("appshots_interaction", nil :: any)
local last_snap = ui.state("appshots_last_snap", nil :: any)
-- The capture last taken in (kept: a reload takes none twice).
local taken = ui.state("appshots_taken", 0)

-- The staged Appshots' limit (the shipping app's MAX_STAGED_APPSHOT_BYTES).
M.MAX_STAGED = 96 * 1024 * 1024
-- Synara's APPSNAP_RECENT_TARGET_WINDOW_MS.
M.RECENT_MS = 60 * 1000

local function invoke(name: string, arg: any?)
	pcall(host.invoke, name, arg)
end

function M.glyph(combo: string): string?
	return GLYPHS[combo]
end
function M.label(combo: string): string
	local g = GLYPHS[combo]
	return if g then `{g} + {g}` else ui.t("settings.appshots.hotkey.none"):get()
end

-- Whether captures are on (Settings → Appshots' Enable switch is the only gate).
function M.active(): boolean
	return M.enabled:get()
end

ui.effect(function()
	local st = M.state:get()
	if type(st) ~= "table" or st.loaded ~= true then
		return
	end
	for _, k in KEYS do
		if st[k] ~= nil then
			(M :: any)[k]:set(st[k])
		end
	end
end)
ui.effect(function()
	local now = {}
	for _, k in KEYS do
		now[k] = (M :: any)[k]:get()
	end
	local st = M.state:peek()
	if type(st) ~= "table" or st.loaded ~= true then
		return
	end
	local changed, any = {}, false
	for _, k in KEYS do
		if now[k] ~= st[k] then
			changed[k] = now[k]
			any = true
		end
	end
	if any then
		invoke("appshots.configure", changed)
	end
end)

function M.refresh()
	invoke("appshots.refresh")
end
-- Asks macOS for a permission ("capture", "text" or "input"); `open` also
-- opens System Settings at it.
function M.request(which: string, open: boolean?)
	invoke("appshots.request", { which = which, open = open == true })
end
function M.open_settings(which: string)
	invoke("appshots.open_settings", { which = which })
end

function M.resolve_automatic(now: number, seen: any?, snap: any?): string?
	local function fresh(c: any?): any?
		return if c and c.session and c.session ~= "" and now - c.at <= M.RECENT_MS then c else nil
	end
	local i, a = fresh(seen), fresh(snap)
	if i and a then
		return if i.at >= a.at then i.session else a.session
	end
	return if i then i.session elseif a then a.session else nil
end

-- What counts as being somewhere: going to a session or a new chat, and
-- typing in its composer (a new chat is nowhere to send to yet).
local function seen(at: any)
	if at.name == "session" and at.session then
		interaction:set({ session = at.session, at = ui.now() })
	elseif at.name == "home" then
		interaction:set({ session = "", at = ui.now() })
	end
end
ui.effect(function()
	seen(route.current:get())
end)
ui.effect(function()
	shared.text:get()
	seen(route.current:peek())
end)

-- A capture as the composer carries it.
function M.chip(shot: any, id: string): any
	return {
		id = id,
		kind = "appshot",
		name = if shot.title and shot.title ~= "" then shot.title else shot.app,
		meta = `{shot.app} · Appshot`,
		src = shot.path,
		path = shot.path,
		icon = shot.icon,
		bytes = shot.bytes,
		appshot = {
			app = shot.app,
			bundle = shot.bundle,
			title = shot.title,
			text = shot.text,
			truncated = shot.truncated == true,
		},
	}
end

-- What a send carries of its Appshots: one each, its picture's path here
-- and what the host writes of it.
function M.of_cargo(items: { any }): { any }
	local out = {}
	for _, item in items do
		if item.kind == "appshot" and item.path and item.appshot then
			local a = item.appshot
			table.insert(out, { path = item.path, app = a.app, bundle = a.bundle, title = a.title, text = a.text, truncated = a.truncated })
		end
	end
	return out
end

-- How many bytes of Appshots are staged.
local function staged(): number
	local n = 0
	for _, item in shared.cargo:peek() do
		if item.kind == "appshot" then
			n += item.bytes or 0
		end
	end
	return n
end

local chips = 0
-- A capture landing: to its destination's composer, unless that would pass
-- the limit.
function M.land(shot: any)
	if staged() + (shot.bytes or 0) > M.MAX_STAGED then
		toast.show { title = ui.t("chat.appshots.limit"):get(), tone = "warning" }
		return
	end
	local now = route.current:peek()
	local where = M.destination:peek()
	if where == "new-session" then
		route.new_chat()
	elseif where == "last-session" then
		if now.name ~= "session" and last_session:peek() ~= "" then
			route.open_session(last_session:peek())
		elseif now.name ~= "session" and now.name ~= "home" then
			route.new_chat()
		end
	else
		local target = M.resolve_automatic(ui.now(), interaction:peek(), last_snap:peek())
		if target then
			if not (now.name == "session" and now.session == target) then
				route.open_session(target)
			end
		elseif now.name ~= "home" then
			route.new_chat()
		end
	end
	local at = route.current:peek()
	if at.name == "session" and at.session then
		last_session:set(at.session)
	end
	last_snap:set({ session = if at.name == "session" then at.session else "", at = ui.now() })
	chips += 1
	-- (Into the destination's own draft: the page shown may not have taken it out yet.)
	shared.push_cargo(if at.name == "session" and at.session then at.session else "", M.chip(shot, `appshot{chips}-{shot.id}`))
	shared.focus:add(1)
end

-- Each capture the host takes, once.
ui.effect(function()
	local c = captured:get()
	if type(c) ~= "table" or not c.n or c.n <= taken:peek() then
		return
	end
	taken:set(c.n)
	M.land(c.shot)
end)
-- Each that couldn't be taken, said once.
local said = ui.state("appshots_said", 0)
ui.effect(function()
	local f = failed:get()
	if type(f) ~= "table" or not f.n or f.n <= said:peek() then
		return
	end
	said:set(f.n)
	toast.show { title = ui.t("chat.appshots.couldnt_capture"):get(), body = f.message, tone = "danger" }
end)
local quick_seen = ui.state("appshots_quick_seen", 0)
ui.effect(function()
	local q = quick:get()
	if type(q) ~= "number" or q <= quick_seen:peek() then
		return
	end
	quick_seen:set(q)
	require("@app/screens/tasks/capture").toggle()
end)

return M
