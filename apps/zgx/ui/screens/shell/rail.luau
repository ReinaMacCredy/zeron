local ds = require("@ds")
local Avatar = require("@ds/avatar")
local locale = require("@ds/locale")
local Menu = require("@ds/menu")
local motion = require("@ds/motion")
local PrivateText = require("@ds/private_text")
local Tooltip = require("@ds/tooltip")
local UpdateGlyph = require("@ds/update_glyph")
local agents = require("@account/agents")
local auth = require("@account/auth")
local boundary = require("@app/screens/shell/boundary")
local keys = require("@app/screens/shell/keys")
local route = require("@app/screens/shell/route")
local view = require("@app/screens/shell/view")
local art = require("@app/screens/shell/art")
local tasks_store = require("@app/screens/tasks/store")
local bots_store = require("@app/screens/bots/store")
local updates = require("@app/screens/shell/updates")

local M = {}

M.W = art.RAIL_W
M.over = ui.state("shell_rail_over", false)

local BUTTON, GLYPH, GAP, TOP, PAD = 34, 18, 6, 44, 8
local HOVER_MS, SWAP_MS = 150, 120
local MENU_RADIUS, MENU_ROW = ui.raw(12, "the Bots menus' corner"), ui.raw(31, "the Bots menus' row")
local AVATAR = 24
local HOVER_FADE = "cubicBezier(.4, 0, .2, 1)"
local REPO = "https://github.com/zeronsh/comet"

local PROVIDERS = {
	{ id = "claude-code", name = "Claude Code" },
	{ id = "codex", name = "Codex" },
	{ id = "cursor", name = "Cursor" },
	{ id = "grok", name = "Grok" },
	{ id = "devin", name = "Devin" },
	{ id = "opencode", name = "OpenCode" },
	{ id = "pi", name = "Pi" },
	{ id = "hermes", name = "Hermes" },
	{ id = "muse", name = "Muse Code" },
}

local pages = ui.shared("plugins.pages", {} :: { any })

local function is_pulls(page: any): boolean
	return page.plugin == "github" and page.id == "pulls"
end

local function pulls_page(): any
	for _, page in pages:get() do
		if is_pulls(page) then
			return page
		end
	end
	return nil
end

M.sidebar_pages = ui.computed(function(): { any }
	local out: { any } = {}
	for _, page in pages:get() do
		if not is_pulls(page) then
			table.insert(out, page)
		end
	end
	return out
end)

local function with_key(label: string, id: string): string
	return locale.s("shell.rail.with_key", { label = label, key = ui.shortcut_text(keys.combo(id)) })
end

type ButtonProps = {
	key: string,
	label: () -> string,
	tooltip: (() -> string)?,
	quiet: (() -> boolean)?,
	lit: (() -> boolean)?,
	on_click: (() -> ())?,
	expanded: (() -> boolean)?,
	icon: string?,
	bold: string?,
	badge: Node?,
	[number]: Child,
}

local function fade(target: any, ms: number, ease: string): any
	return ui.tween(target, { duration = math.max(motion.ms(ms), 1), ease = ease })
end

-- ds.IconButton's hover snaps; this eases as gpui's rail does (motion::HOVER_FADE).
local RailButton = ui.component(function(props: ButtonProps): Node
	local over = ui.use_state(false)
	local on = ui.computed(function(): number
		return if props.lit ~= nil and props.lit() then 1 else 0
	end)
	local bright = ui.computed(function(): number
		return if over:get() or on:get() == 1 then 1 else 0
	end)
	local wash = ui.computed(function(): number
		return if over:get() and on:get() == 0 then 1 else 0
	end)
	local tip = if props.tooltip
		then ui.show(function(): boolean
			return props.quiet == nil or not props.quiet()
		end, Tooltip { props.tooltip })
		else nil
	local function fill(bg: string, shown: any): Node
		return ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = "$radius.lg",
			bg = bg,
			opacity = fade(shown, HOVER_MS, HOVER_FADE),
			pointer_events = "none",
		}
	end
	local glyph: Child = props[1]
	if props.icon then
		-- Each variant is mounted only while it shows: an image left at opacity 0 holds the shell's first frame after a route change back.
		local ms = math.max(motion.ms(SWAP_MS), 1)
		local outline, bold = props.icon, props.bold or props.icon
		glyph = ui.div {
			relative = true,
			w = GLYPH,
			h = GLYPH,
			ui.presence(function(): string
				return if on:get() == 1 then bold else outline
			end, function(name: string): Node
				return ds.Icon { name = name :: any, size = GLYPH }
			end, {
				absolute = true,
				left = 0,
				top = 0,
				w = GLYPH,
				h = GLYPH,
				enter = { opacity = { 0, 1 }, duration = ms, ease = "outQuad" },
				exit = { opacity = 0, duration = ms, ease = "outQuad" },
			}),
			props.badge,
		}
	end
	return ui.button {
		id = props.key,
		label = props.label,
		tooltip = tip,
		tooltip_placement = { side = "right", align = "center", offset = 6 },
		expanded = props.expanded,
		relative = true,
		w = BUTTON,
		h = BUTTON,
		shrink = 0,
		p = 0,
		items = "center",
		justify = "center",
		rounded = "$radius.lg",
		bg = "#00000000",
		hover = { bg = "#00000000" },
		active = { bg = "$pressed" },
		cursor = if props.on_click then "pointer" else nil,
		color = ui.tween(bright, { from = "$text.secondary", to = "$text.primary", duration = math.max(motion.ms(HOVER_MS), 1), ease = HOVER_FADE }),
		on_hover = function(inside: boolean)
			over:set(inside)
		end,
		on_click = props.on_click,
		fill("$zeron.selected.flat.bg", on),
		fill("$hover", wash),
		glyph,
	}
end)

local function more_owns(c: any): boolean
	if c.name ~= "plugin" then
		return false
	end
	if c.plugin == "github" then
		return c.page == "pulls"
	end
	return c.plugin == "zeron" and (c.page == "inbox" or c.page == "tasks" or c.page == "skills")
end

local function home(): Node
	return RailButton {
		key = "rail-new-chat",
		label = function(): string
			return locale.s("shell.rail.home")
		end,
		tooltip = function(): string
			return with_key(locale.s("shell.rail.home"), "shell.new_chat")
		end,
		lit = function(): boolean
			local name = route.current:get().name
			return (name == "home" or name == "session") and not bots_store.mode:get()
		end,
		on_click = function()
			route.new_chat()
		end,
		icon = "home",
		bold = "home-bold",
	}
end

local function bots(): Node
	return RailButton {
		key = "rail-bots",
		label = function(): string
			return locale.s("bots.title")
		end,
		tooltip = function(): string
			return locale.s("bots.title")
		end,
		lit = function(): boolean
			return bots_store.mode:get()
		end,
		on_click = function()
			route.plugin_page("zeron", "bots")
		end,
		icon = "bot",
		bold = "bot-bold",
	}
end

local function search(): Node
	return RailButton {
		key = "rail-search",
		label = function(): string
			return locale.s("shell.rail.search")
		end,
		tooltip = function(): string
			return with_key(locale.s("shell.rail.search"), "shell.palette")
		end,
		on_click = function()
			view.palette:set(not view.palette:peek())
		end,
		icon = "search",
		bold = "search-bold",
	}
end

local function tasks_badge(): Node
	return ui.show(function(): boolean
		return tasks_store.unseen:get() > 0
	end, ui.text {
		id = "rail-tasks-badge",
		function(): string
			return tostring(tasks_store.unseen:get())
		end,
		absolute = true,
		top = -5,
		right = -7,
		min_w = 14,
		h = 14,
		px = ui.raw(3, "gpui badge"),
		rounded = "$radius.full",
		bg = "$accent",
		color = ui.raw("#ffffff", "white on the accent, as gpui"),
		text_size = ui.raw(9, "gpui badge"),
		align = "center",
		line_height = 14,
	})
end

local function more(): Node
	local open = ui.use_state(false)
	local shown = ui.computed(function(): number
		return if open:get() then 1 else 0
	end)
	local function row(id: string, icon: any, label: any, go: () -> ()): Node
		return Menu.Row {
			id = id,
			icon = icon,
			label = label,
			h = MENU_ROW,
			on_click = function()
				open:set(false)
				go()
			end,
		}
	end
	local function page(id: string, icon: string, word: string): Node
		return row(`rail-{id}`, icon, ui.t(word), function()
			route.plugin_page("zeron", id)
		end)
	end
	return ui.popover(Menu.panel {
		open = open,
		anchor = RailButton {
			key = "rail-more",
			label = function(): string
				return locale.s("shell.rail.more")
			end,
			tooltip = function(): string
				return locale.s("shell.rail.more")
			end,
			quiet = function(): boolean
				return open:get()
			end,
			lit = function(): boolean
				return open:get() or more_owns(route.current:get())
			end,
			expanded = function(): boolean
				return open:get()
			end,
			on_click = ui.toggle(open),
			icon = "more-horizontal",
			bold = "more-horizontal-bold",
			badge = tasks_badge(),
		},
		label = function(): string
			return locale.s("shell.rail.more")
		end,
		placement = { side = "right", align = "start", offset = 6 },
		w = "$zeron.menu.width",
		rounded = MENU_RADIUS,
		origin = { 0, 0 },
		opacity = ui.tween(shown, { duration = math.max(motion.ms(140), 1), ease = "cubicBezier(.2, .8, .2, 1)" }),
		scale = ui.tween(shown, { from = 0.96, to = 1, duration = math.max(motion.ms(140), 1), ease = "cubicBezier(.2, .8, .2, 1)" }),
		Menu.Group {
			page("inbox", "inbox", "inbox.title"),
			page("tasks", "tasks", "tasks.title"),
			page("skills", "magic-stick", "skills.title"),
			ui.show(function(): boolean
				return pulls_page() ~= nil
			end, row("open-pull-requests", "pull-request", function(): string
				local p = pulls_page()
				return if p then p.title or "" else ""
			end, function()
				route.plugin_page("github", "pulls")
			end)),
		},
	})
end

type Live = { provider: any, account: any }

-- One a provider: its account in use, while that reports windows.
local function live_accounts(): { Live }
	local on = agents.on_device:get()[""]
	local out: { Live } = {}
	for _, provider in PROVIDERS do
		for _, account in (on and on[provider.id]) or {} do
			if account.active and #account.windows > 0 then
				table.insert(out, { provider = provider, account = account })
				break
			end
		end
	end
	return out
end

local function most_used(account: any): number
	local most = 0
	for _, w in account.windows do
		most = math.max(most, math.clamp(w.used, 0, 1))
	end
	return most
end
M.live_accounts = live_accounts
M.most_used = most_used

local function help_row(id: string, icon: any, label: string, on_click: () -> ()): Node
	return Menu.Row { id = id, icon = icon, label = ui.t(label), on_click = on_click }
end

local function help(): Node
	local open = ui.use_state(false)
	local function go(action: () -> ()): () -> ()
		return function()
			open:set(false)
			action()
		end
	end
	return ui.popover(Menu.panel {
		open = open,
		anchor = RailButton {
			key = "rail-help",
			label = function(): string
				return locale.s("shell.rail.help")
			end,
			tooltip = function(): string
				return locale.s("shell.rail.help")
			end,
			quiet = function(): boolean
				return open:get()
			end,
			lit = function(): boolean
				return open:get()
			end,
			expanded = function(): boolean
				return open:get()
			end,
			on_click = ui.toggle(open),
			icon = "lucide-circle-help",
			bold = "question-circle-bold",
		},
		label = function(): string
			return locale.s("shell.rail.help")
		end,
		placement = { side = "right", align = "end", offset = 6 },
		w = "$zeron.menu.width",
		Menu.Group {
			help_row("help-keybindings", "keyboard", "shell.rail.help_shortcuts", go(function()
				route.settings("shortcuts")
			end)),
			help_row("help-star", "star-bold", "shell.rail.help_star", go(function()
				ui.invoke("open_url", REPO)
			end)),
		},
	})
end

local about = ui.shared("about", { app_version = "", available = false })

local function app_update(): any?
	local here = updates.here()
	if here then
		return here
	end
	local a: any = about:get()
	if a and a.blocker ~= nil and a.available == true then
		return { phase = "available", action = "explain", latest = a.latest }
	end
	return nil
end

local function app_update_label(): string
	local r = app_update()
	if not r then
		return ""
	end
	if r.phase == "downloading" then
		return locale.s("shell.updates.downloading")
	elseif r.phase == "ready" then
		return locale.s("shell.updates.ready", { version = r.latest or "" })
	elseif r.phase == "failed" then
		return locale.s("shell.updates.failed")
	end
	return locale.s("shell.updates.version", { version = r.latest or "" })
end

local function explain()
	local prompt: any = boundary.load("@app/screens/shell/update_prompt")
	local watch: any = boundary.load("@app/screens/engine/update")
	if prompt and watch then
		prompt.open()
		watch.show_prompt(prompt)
	end
end

local function app_update_button(): Node
	return RailButton {
		key = "update-rail-button",
		label = app_update_label,
		tooltip = app_update_label,
		on_click = function()
			local r = app_update()
			if not r or not r.action then
				return
			end
			if r.action == "explain" then
				explain()
			else
				updates.apply_here(r)
			end
		end,
		UpdateGlyph {
			phase = function(): any
				local r = app_update()
				return if r then r.phase else "available"
			end,
		},
	}
end

local function update_entry(): Node
	local function module(): any
		return boundary.load("@app/screens/engine/update")
	end
	-- The app's own update wins (gpui shows only it): it restarts the engine too, and an idle stale engine restarts itself.
	local function kind(): string
		if app_update() then
			return "app"
		end
		local update = module()
		local now = if update then update.which() else ""
		return if now == "ours" or now == "theirs" then now else ""
	end
	return ui.show(function(): boolean
		return kind() ~= ""
	end, ui.presence(kind, function(now: string): Node
		if now == "app" then
			return app_update_button()
		end
		local mine = now == "ours"
		return RailButton {
			key = "update-rail-button",
			label = function(): string
				return locale.s(if mine then "engine.update.restart" else "engine.update.out_of_date")
			end,
			tooltip = function(): string
				return locale.s(if mine then "engine.update.restart_tip" else "engine.update.theirs_tip")
			end,
			on_click = if mine
				then function()
					local update = module()
					if update then
						update.restart()
					end
				end
				else nil,
			ui.row {
				w = 20,
				h = 20,
				items = "center",
				justify = "center",
				rounded = "$radius.full",
				bg = "$accent",
				color = "$accent.text",
				ds.Icon { name = if mine then "arrow-up" else "info-circle", size = 14 },
			},
		}
	end, {
		enter = motion.transition({ opacity = { 0, 1 }, duration = 140 }),
		exit = motion.transition({ opacity = 0, duration = 100 }),
	}))
end

-- You: your name, else your email; signed out, the workspace's ("Local",
-- as Zeron's: kept on this device); nothing until the engine has said which.
type User = { id: string?, name: string, avatar: string? }
local function user(): User
	local u = auth.user:get()
	if u then
		local name = if u.first_name then (u.first_name :: string):match("^%s*(.-)%s*$") else nil
		-- (No first name: the email, unless addresses are hidden on screen.)
		local fallback = if PrivateText.hidden:get() then locale.s("shell.sidebar.you") else u.email
		-- (Your photo: Settings → Profile's, kept from the last time it was heard.)
		local settings_store: any = boundary.load("@app/screens/settings/store")
		local avatar = if settings_store then settings_store.photo_file(settings_store.photo()) else ""
		return { id = u.id, name = if name and name ~= "" then name else fallback, avatar = if avatar ~= "" then avatar else nil }
	end
	if auth.state:get() == "restoring" then
		return { name = "" }
	end
	return { name = locale.s("shell.sidebar.local") }
end

M.user = user
M.account = {
	on_user = function()
		route.settings("profile")
	end,
	user_open = function(): boolean
		local c = route.current:get()
		return c.name == "settings" and c.page == "profile"
	end,
}

local function account_entry(): Node
	local function name(): string
		return user().name
	end
	return ui.show(function(): boolean
		return name() ~= ""
	end, RailButton {
		key = "rail-account",
		label = name,
		tooltip = name,
		lit = M.account.user_open,
		on_click = M.account.on_user,
		ui.presence(function(): string
			local u = user()
			return u.id or u.name
		end, function(seed: string): Node
			return Avatar {
				seed = seed,
				size = AVATAR,
				label = name(),
				src = function(): string?
					return user().avatar
				end,
			}
		end),
	})
end

local function settings_entry(): Node
	local function open(): boolean
		return route.current:get().name == "settings"
	end
	local function name(): string
		return locale.s(if open() then "shell.rail.close_settings" else "ds.sidebar.settings")
	end
	return RailButton {
		key = "settings-trigger",
		label = name,
		tooltip = function(): string
			return with_key(name(), "shell.settings")
		end,
		lit = open,
		on_click = function()
			if route.current:peek().name == "settings" then
				route.leave_settings()
			else
				route.settings()
			end
		end,
		icon = "settings",
		bold = "settings-bold",
	}
end

M.Rail = ui.component(function(): Node
	-- Voice joins above Help.
	local entries: { any } = {
		home(),
		bots(),
		search(),
		more(),
		ui.div { grow = 1 },
		help(),
		settings_entry(),
		update_entry(),
		account_entry(),
	}
	local rail: { [any]: any } = {
		id = "app-rail",
		on_hover = function(inside: boolean)
			M.over:set(inside)
		end,
		role = "toolbar",
		label = function(): string
			return locale.s("shell.rail.label")
		end,
		w = M.W,
		h_full = true,
		shrink = 0,
		pt = ui.raw(TOP + PAD, "under the title bar, as gpui's rail starts"),
		pb = ui.raw(PAD, "gpui's rail padding"),
		gap = ui.raw(GAP, "gpui's rail gap"),
		items = "center",
		window_drag = true,
		relative = true,
		-- (On the picture its icons need every bit of contrast they can get: 3:1 at its brightest.)
		theme = function(): any
			return if art.under_rail:get() then { ["text.secondary"] = "$text.primary" } else nil
		end,
	}
	for _, entry in entries do
		table.insert(rail, entry)
	end
	return ui.column(rail)
end)

return M
