-- Settings' pages: General, Appearance, Notifications and Profile (yours,
-- your account included); Providers (this Mac's agents); Members (your
-- organization); Cloud and Billing (the account's Cloud, while it has one).
-- Each is a module of its own with `ID`, `TITLE` and `view`, added to the
-- shell's settings (screens/shell/settings: the list, the page beside it),
-- ahead of Plugins and the shell's Archived. Also the search atop the list (⌘F,
-- search.luau), and the sessions' chimes (chimes.luau).
--
-- The shell preloads this module (screens/shell/init.luau's list), so a
-- broken save here shows as that failure, not a blank window.

local settings = require("@app/screens/shell/settings")
local route = require("@app/screens/shell/route")
local auth = require("@account/auth")
local search = require("@app/screens/settings/search")
local store = require("@app/screens/settings/store")
local Builtins = require("@app/screens/plugins/builtins")
require("@app/screens/settings/chimes")

-- (The window's language, which General picks: followed from the start.)
require("@app/screens/language")
local cloud_page = require("@app/screens/settings/cloud")
local billing_page = require("@app/screens/settings/billing")
local cloud = require("@account/cloud")

-- Ahead of the pages others add (Plugins, which joins Organization after
-- Members; the shell's Archived): whichever of them is listed first.
local function ahead(): string
	for _, id in settings.pages:peek() do
		if id == "plugins" or id == "archived" then
			return id
		end
	end
	return "archived"
end

-- (Each page by its module's `ID` and `TITLE`: the module loads as it's first shown or searched.)
-- `builtin`: the built-in plugin whose page it is, listed while that's on.
local function add(id: string, title: string, module: string, icon: string, group: string, builtin: string?)
	settings.add {
		id = id,
		title = title,
		icon = icon,
		group = group,
		-- (The shell centres each page in the room beside the list.)
		module = `@app/screens/settings/{module}`,
		before = ahead(),
		shown = if builtin
			then function(): boolean
				return Builtins.on(builtin)
			end
			else nil,
	}
end

add("general", "General", "general", "settings-minimalistic", "Personal")
add("chat", "Chat", "chat", "chat-round-line", "Personal")
add("appearance", "Appearance", "appearance", "tuning", "Personal")
add("notifications", "Notifications", "notifications", "bell", "Personal")
add("files", "Files", "files", "folder", "Personal")
add("shortcuts", "Shortcuts", "shortcuts", "keyboard", "Personal")
add("appshots", "Appshots", "appshots", "gallery", "Personal", "appshots")
-- (Zeron's This Mac > Computer use, beside Appshots as there.)
add("computer", "Computer use", "computer", "computer-cursor", "Personal")
-- (Zeron's This Mac > Voice, beside Computer use as there.)
add("voice", "Voice", "voice", "microphone", "Personal", "voice")
add("profile", "Account", "profile", "user-rounded", "Personal")
add("usage", "Usage", "usage", "wallet", "Personal", "usage")
-- The panel's browser, under a heading of its own (room for what it grows:
-- site data, permissions, downloads).
add("browser", "Browsing", "browser", "global", "Browser")
add("browser_history", "History", "browser_history", "clock-circle", "Browser")
add("providers", "Providers", "providers", "widget", "Devices")
-- (Zeron's Agents heading has its Profiles beside Providers; zgx keeps the
-- agents under Devices.)
add("agent-profiles", "Agent profiles", "agent_profiles", "bot", "Devices")
-- (Zeron's Agents > Compaction, beside the agents here too.)
add("compaction", "Compaction", "compaction", "collapse-arrows", "Devices")
-- (Zeron's Agents > Code review, beside Compaction too.)
add("code_review", "Code review", "code_review", "file-diff", "Devices")
-- (Zeron's Agents > Tools, beside them too.)
add("tools", "Tools", "tools", "action-configure", "Devices")
-- (Zeron's Agents > Routing, beside them too.)
add("routing", "Routing", "routing", "map-arrow-right", "Devices")
add("devices", "Devices", "devices", "laptop", "Devices")
-- (Zeron's Devices & updates > Device hub, beside Devices here.)
add("device_hub", "Device hub", "device_hub", "smartphone", "Devices", "device")
add("members", "Members", "members", "users-group-rounded", "Organization")
add("experimental", "Experimental", "lab", "action-test", "Lab")
-- Last, under Zeron's own heading (the shipping app's About and its
-- updates, in its app menu): after everything, Archived too.
settings.add {
	id = "about",
	title = "About & updates",
	icon = "info-square",
	group = "Zeron",
	module = "@app/screens/settings/about",
}

-- Cloud and Billing, only while the account has Cloud (the shipping app's
-- `cloud_offered`): listed ahead of Zeron's own, and found by the search
-- only then.
settings.extra("cloud", function(): { any }
	if not cloud.offered() then
		return {}
	end
	return {
		{ id = cloud_page.ID, title = cloud_page.TITLE, icon = "cloud", group = "Cloud", view = cloud_page.view, private = true },
		{ id = billing_page.ID, title = billing_page.TITLE, icon = "wallet", group = "Cloud", view = billing_page.view, private = true },
	}
end)
ui.effect(function()
	if cloud.offered() then
		search.index(cloud_page.ID, ui.t("settings.cloud.title"), cloud_page.TERMS, cloud_page.ROWS)
		search.index(billing_page.ID, ui.t("settings.billing.title"), billing_page.TERMS, billing_page.ROWS)
	else
		search.unindex(cloud_page.ID)
		search.unindex(billing_page.ID)
	end
end)
-- Cloud's status as the account signs in (whether it has Cloud lists the
-- pages, and the new chat's Work in offers it), and nothing of it kept
-- once signed out.
local cloud_for = ui.state("settings_cloud_for", "")
ui.effect(function()
	local u = auth.user:get()
	if auth.state:get() == "signed_in" and u then
		-- (Again after a reload that dropped a read on its way.)
		if cloud_for:peek() ~= u.id or cloud.now:peek().phase == "unknown" then
			cloud_for:set(u.id)
			cloud.load()
		end
	elseif cloud_for:peek() ~= "" then
		cloud_for:set("")
		cloud.forget()
	end
end)

-- Searched, every page is: each lists its rows as it loads.
ui.effect(function()
	if search.query:get() ~= "" then
		settings.load_all()
	end
end)

-- The search: its field heads the list, and while something is typed its
-- results stand in the pages' place.
settings.head(search.Field, {
	replaces = function(): boolean
		return search.query:get() ~= ""
	end,
})

-- Account is Profile's now: a way in to `settings/account` (a link, a
-- history entry, a saved route) lands there, on the account's rows.
ui.effect(function()
	local r = route.current:get()
	if r.name == "settings" and r.page == "account" then
		search.open({ page = "profile", key = "email", title = "", where = "", score = 0 })
	end
end)

-- Your profile once a run while signed in (the sidebar shows your photo
-- from it): kept over a reload, so a reload asks nothing again.
local asked_me = ui.state("settings_me_asked_for", "")
ui.effect(function()
	local u = auth.user:get()
	if u and asked_me:peek() ~= u.id then
		asked_me:set(u.id)
		store.load_me()
	end
end)

-- Signed out, nothing of the account stays for the next one.
ui.effect(function()
	if auth.state:get() == "signed_out" then
		store.forget()
	end
end)

return {}
