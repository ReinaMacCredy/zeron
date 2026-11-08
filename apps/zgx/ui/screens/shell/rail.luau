local ds = require("@ds")
local AgentMark = require("@ds/agent_mark")
local context_usage = require("@ds/context_usage")
local HoverCard = require("@ds/hover_card")
local locale = require("@ds/locale")
local Menu = require("@ds/menu")
local motion = require("@ds/motion")
local PrivateText = require("@ds/private_text")
local Tooltip = require("@ds/tooltip")
local agents = require("@account/agents")
local boundary = require("@app/screens/shell/boundary")
local keys = require("@app/screens/shell/keys")
local route = require("@app/screens/shell/route")
local art = require("@app/screens/shell/art")
local tasks_store = require("@app/screens/tasks/store")

local M = {}

M.W = art.RAIL_W

local BUTTON, GLYPH, GAP, TOP, PAD = 34, 18, 6, 44, 8
local RING_RADIUS, RING_STROKE, RING_GLYPH = 13, 2.5, 14
local CARD_W = 290
local HOVER_FADE = "cubicBezier(.4, 0, .2, 1)"
local REPO = "https://github.com/zeronsh/comet"

local PROVIDERS = {
	{ id = "claude-code", name = "Claude Code", mark = "claude" },
	{ id = "codex", name = "Codex", mark = "openai" },
	{ id = "cursor", name = "Cursor", mark = "cursor" },
	{ id = "grok", name = "Grok", mark = "grok" },
	{ id = "devin", name = "Devin", mark = "devin" },
	{ id = "opencode", name = "OpenCode", mark = "opencode" },
	{ id = "pi", name = "Pi", mark = "pi" },
	{ id = "hermes", name = "Hermes", mark = "hermes" },
	{ id = "muse", name = "Muse Code", mark = "muse" },
}

local WINDOW_NAMES: { [string]: string } = {
	Session = "shell.rail.usage.window.session",
	Week = "shell.rail.usage.window.week",
	Day = "shell.rail.usage.window.day",
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
	[number]: Child,
}

-- ds.IconButton's hover snaps; this eases as gpui's rail does (motion::HOVER_FADE).
local RailButton = ui.component(function(props: ButtonProps): Node
	local over = ui.use_state(false)
	local bright = ui.computed(function(): number
		return if over:get() or (props.lit ~= nil and props.lit()) then 1 else 0
	end)
	local tip = if props.tooltip
		then ui.show(function(): boolean
			return props.quiet == nil or not props.quiet()
		end, Tooltip { props.tooltip })
		else nil
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
		color = ui.tween(bright, { from = "$text.secondary", to = "$text.primary", duration = motion.ms(150), ease = HOVER_FADE }),
		on_hover = function(inside: boolean)
			over:set(inside)
		end,
		on_click = props.on_click,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = "$radius.lg",
			bg = "$hover",
			opacity = ui.tween(bright, { duration = motion.ms(150), ease = HOVER_FADE }),
			pointer_events = "none",
		},
		props[1],
	}
end)

local function new_chat(): Node
	return RailButton {
		key = "rail-new-chat",
		label = function(): string
			return locale.s("ds.sidebar.new_chat")
		end,
		tooltip = function(): string
			return with_key(locale.s("ds.sidebar.new_chat"), "shell.new_chat")
		end,
		lit = function(): boolean
			return route.current:get().name == "home"
		end,
		on_click = function()
			route.new_chat()
		end,
		ds.Icon { name = "pen-new-square", size = GLYPH },
	}
end

local function pull_requests(): Node
	local function title(): string
		local page = pulls_page()
		return if page then page.title or "" else ""
	end
	return ui.show(function(): boolean
		return pulls_page() ~= nil
	end, RailButton {
		key = "open-pull-requests",
		label = title,
		tooltip = title,
		lit = function(): boolean
			local c = route.current:get()
			return c.name == "plugin" and c.plugin == "github" and c.page == "pulls"
		end,
		on_click = function()
			route.plugin_page("github", "pulls")
		end,
		ds.Icon { name = "pull-request", size = GLYPH },
	})
end

-- Inbox, Tasks and Skills (app_rail.rs), built-in pages on the plugin route; Tasks counts unseen captures.
local function page_entry(page: string, icon: string, word: string): Node
	local count = if page == "tasks" then tasks_store.unseen else nil
	return RailButton {
		key = `rail-{page}`,
		label = function(): string
			return locale.s(word)
		end,
		tooltip = function(): string
			return locale.s(word)
		end,
		lit = function(): boolean
			local c = route.current:get()
			return c.name == "plugin" and c.plugin == "zeron" and c.page == page
		end,
		on_click = function()
			route.plugin_page("zeron", page)
		end,
		ui.div { relative = true, w = GLYPH, h = GLYPH, ds.Icon { name = icon, size = GLYPH }, if count then ui.show(function(): boolean
			return count:get() > 0
		end, ui.text {
			id = "rail-tasks-badge",
			function(): string
				return tostring(count:get())
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
		}) else nil },
	}
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

local function tightest(list: { Live }): Live?
	local best: Live? = nil
	for _, live in list do
		if best == nil or most_used(live.account) >= most_used(best.account) then
			best = live
		end
	end
	return best
end

local function window_name(label: string): string
	local key = WINDOW_NAMES[label]
	return if key then locale.s(key) else label
end

local function percent(used: number): string
	return `{math.floor(math.clamp(used, 0, 1) * 100 + 0.5)}%`
end

local function ring_label(): string
	local shown = tightest(live_accounts())
	if not shown then
		local loading = agents.on_device:get()[""] == nil
		return locale.s(if loading then "shell.rail.usage.loading" else "shell.rail.usage.none")
	end
	if most_used(shown.account) >= 1 then
		return locale.s("shell.rail.usage.limit")
	end
	local parts: { string } = {}
	for _, w in shown.account.windows do
		table.insert(parts, `{window_name(w.label)} {percent(w.used)}`)
	end
	return locale.s("shell.rail.usage.line", { name = shown.provider.name, windows = table.concat(parts, ", ") })
end

local function window_row(w: any, binding: boolean): Node
	local hot = w.used >= 1
	local tone = if hot then "$danger" else "$text.secondary"
	return ui.column {
		gap = ui.raw(2, "a window's bar and the line under it, as gpui's card"),
		ui.row {
			items = "center",
			gap = "$space.2",
			ui.text { window_name(w.label), w = ui.raw(62, "gpui's window column"), shrink = 0, truncate = true, color = tone },
			ds.ProgressBar {
				value = function(): number
					return math.clamp(w.used, 0, 1)
				end,
				tone = if w.used >= 0.95 then "danger" elseif w.used >= 0.8 then "warning" else "accent",
				grow = 1,
			},
			ui.text {
				percent(w.used),
				w = ui.raw(44, "gpui's percent column"),
				shrink = 0,
				tabular = true,
				align = "right",
				color = if hot then "$danger" else "$text.primary",
			},
		},
		if hot and binding
			then ui.text {
				ui.t("shell.rail.usage.limit_reached"),
				pl = ui.raw(70, "under the bar: the name's column and its gap"),
				color = tone,
			}
			else nil,
	}
end

local function account_section(live: Live, binding: any): Node
	local a = live.account
	local rows: { any } = {
		gap = "$space.2",
		ui.row {
			items = "center",
			gap = "$zeron.space.tight",
			AgentMark.Mark { label = live.provider.name, mark = live.provider.mark, seed = live.provider.id, size = 13, bare = true },
			ui.text { live.provider.name, shrink = 0, weight = "$font.weight.medium", text_size = "$font.size.sm" },
			if a.plan ~= "" then ui.text { a.plan, shrink = 0, color = "$text.secondary" } else nil,
			ui.div { grow = 1 },
			PrivateText { a.email, reveal = false, truncate = true, shrink = 1, min_w = 0, color = "$text.secondary", text_size = "$font.size.xs" },
		},
	}
	if a.error then
		table.insert(rows, ui.text { a.error, wrap = true, color = "$warning" })
	end
	for _, w in a.windows do
		table.insert(rows, window_row(w, w == binding))
	end
	return ui.column(rows)
end

local function hover_card(): Node
	return HoverCard.Card {
		id = "rail-usage-hover-card",
		w = function(): any
			return if #live_accounts() > 0 then CARD_W else "auto"
		end,
		p = 0,
		gap = 0,
		text_size = "$font.size.xs",
		ui.switch(function(): string
			return if #live_accounts() > 0 then "accounts" else "none"
		end, {
			accounts = ui.column {
				w_full = true,
				gap = 0,
				ui.column {
					w_full = true,
					p = "$space.2",
					gap = "$space.3",
					ui.row {
						w_full = true,
						items = "baseline",
						justify = "between",
						ui.text { ui.t("shell.rail.usage.title"), weight = "$font.weight.medium", text_size = "$font.size.sm" },
						ui.text { ui.t("shell.rail.usage.used"), color = "$text.secondary" },
					},
					ui.column {
						w_full = true,
						gap = "$space.3",
						ui.each(ui.computed(function(): { any }
							local list = live_accounts()
							local best = tightest(list)
							local binding: any = nil
							if best then
								for _, w in best.account.windows do
									if binding == nil or w.used > binding.used then
										binding = w
									end
								end
							end
							local out: { any } = {}
							for _, live in list do
								table.insert(out, { live = live, binding = binding })
							end
							return out
						end), function(item: any): Node
							return account_section(item.live, item.binding)
						end, {
							key = function(item: any): string
								return `{item.live.provider.id}:{item.live.account.id}`
							end,
							gap = "$space.3",
						}),
					},
				},
				Menu.Rule {},
				ui.row {
					w_full = true,
					px = "$space.2",
					py = "$space.2",
					items = "center",
					justify = "between",
					color = "$text.secondary",
					ui.text { ui.t("shell.rail.usage.open_page") },
					ds.Icon { name = "alt-arrow-right", size = 12 },
				},
			},
			none = ui.text { ring_label, px = "$space.2", py = "$space.1", color = "$text.primary", wrap = true },
		}),
	}
end

local function usage_ring(): Node
	ui.effect(function()
		agents.watch_device("")
	end)
	local over = ui.use_state(false)
	local clicked = ui.use_state(false)
	local function shown(): Live?
		return tightest(live_accounts())
	end
	local function hot(): boolean
		local s = shown()
		return s ~= nil and most_used(s.account) >= 1
	end
	local function lit(): boolean
		local c = route.current:get()
		return c.name == "settings" and c.page == "usage"
	end
	local bright = ui.computed(function(): number
		return if over:get() or lit() then 1 else 0
	end)
	local card = ui.show(function(): boolean
		return not clicked:get()
	end, hover_card())
	return ui.button {
		id = "rail-usage",
		label = ring_label,
		tooltip = card,
		tooltip_placement = { side = "right", align = "end", offset = 6 },
		tooltip_interactive = true,
		tooltip_delay = 120,
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
		cursor = "pointer",
		color = "$text.primary",
		on_hover = function(inside: boolean)
			over:set(inside)
			if inside then
				agents.refresh_device("")
			else
				clicked:set(false)
			end
		end,
		on_click = function()
			clicked:set(true)
			route.settings("usage")
		end,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = "$radius.lg",
			bg = "$hover",
			opacity = ui.tween(bright, { duration = motion.ms(150), ease = HOVER_FADE }),
			pointer_events = "none",
		},
		ui.canvas {
			absolute = true,
			left = 0,
			top = 0,
			w = BUTTON,
			h = BUTTON,
			pointer_events = "none",
			draw = function(c, w, h)
				local s = shown()
				local f = if hot() then 1 elseif s then most_used(s.account) else 0
				c:stroke(ui.path():arc(w / 2, h / 2, RING_RADIUS, 0, 360), "$border", RING_STROKE)
				if f > 0 then
					local ink = if hot() then "$danger" else context_usage.usage_ink(f)
					c:stroke(ui.path():arc(w / 2, h / 2, RING_RADIUS, -90, 360 * f), ink, RING_STROKE, { cap = "round" })
				end
			end,
		},
		ui.presence(function(): string
			local s = shown()
			return if s then s.provider.id else ""
		end, function(id: string): Node
			if id == "" then
				return ui.div { opacity = 0.5, ds.Icon { name = "usage-chart", size = RING_GLYPH } }
			end
			for _, provider in PROVIDERS do
				if provider.id == id then
					return AgentMark.Mark { label = provider.name, mark = provider.mark, seed = id, size = RING_GLYPH, bare = true }
				end
			end
			return ui.div {}
		end),
		ui.show(hot, ui.row {
			id = "rail-usage-limit",
			absolute = true,
			top = ui.raw(1, "gpui's badge on the ring's corner"),
			right = 0,
			w = 12,
			h = 12,
			items = "center",
			justify = "center",
			rounded = "$radius.full",
			bg = "$danger",
			pointer_events = "none",
			ui.text { "!", text_size = "$font.size.xs", weight = "$font.weight.bold", color = "$accent.text" },
		}),
	}
end

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
			ds.Icon { name = "lucide-circle-help", size = GLYPH },
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

local function update_entry(): Node
	local function module(): any
		return boundary.load("@app/screens/engine/update")
	end
	local function kind(): string
		local update = module()
		local now = if update then update.which() else ""
		return if now == "ours" or now == "theirs" then now else ""
	end
	return ui.show(function(): boolean
		return kind() ~= ""
	end, ui.presence(kind, function(now: string): Node
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
		ds.Icon { name = "settings", size = GLYPH },
	}
end

M.Rail = ui.component(function(): Node
	-- Voice joins above Help.
	local entries: { any } = {
		new_chat(),
		page_entry("inbox", "inbox", "inbox.title"),
		page_entry("tasks", "checklist", "tasks.title"),
		page_entry("skills", "magic-stick", "skills.title"),
		pull_requests(),
		ui.div { grow = 1 },
		usage_ring(),
		help(),
		update_entry(),
		settings_entry(),
	}
	local rail: { [any]: any } = {
		id = "app-rail",
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
