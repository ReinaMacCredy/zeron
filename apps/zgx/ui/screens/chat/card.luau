--!strict
local Icon = require("@ds/icon")
local SidebarIcon = require("@ds/sidebar_icon")
local Tooltip = require("@ds/tooltip")
local control = require("@ds/control")
local glass = require("@ds/glass")
local locale = require("@ds/locale")
local motion = require("@ds/motion")
local sessions = require("@account/sessions")
local Actions = require("@app/screens/panel/actions")
local panel_state = require("@app/screens/panel/state")
local files = require("@app/screens/files/api")
local boundary = require("@app/screens/shell/boundary")
local fit = require("@app/screens/shell/fit")
local shell_view = require("@app/screens/shell/view")
local route = require("@app/screens/shell/route")
local BotMenu = require("@app/screens/bots/menu")
local BotChat = require("@app/screens/bots/chat")

local M = {}

M.W = 272
M.SHIFT = 290
local ROOM = 760
-- Where the card starts under its toggles; a Bot's header ends lower.
local HEAD_CLEAR = 44
local OPEN_MS, CLOSE_MS, SHIFT_MS = 160, 100, 200
local EASE = "cubicBezier(.2, .8, .2, 1)"
local WHY = "the mockup's sidecar card"

M.open = ui.persist("chat_card_open", false)
M.room = ui.shared("chat.card.room", 0)

M.shift = motion.tween(
	ui.computed(function(): number
		return if M.open:get() and route.session:get() ~= nil and M.room:get() >= ROOM then M.SHIFT else 0
	end),
	{ duration = SHIFT_MS, ease = EASE }
)

local function chat_of(id: string?): sessions.Chat?
	for _, c in sessions.now:get().chats do
		if c.id == id then
			return c
		end
	end
	return nil
end

local function device_of(id: string?): sessions.Device?
	for _, d in sessions.now:get().devices do
		if d.id == id then
			return d
		end
	end
	return nil
end

local function git_on(c: sessions.Chat?): boolean
	if not c then
		return false
	end
	for _, p in sessions.now:get().projects do
		if p.id == c.project then
			return p.git == true
		end
	end
	return false
end

local function device_icon(d: sessions.Device?): string
	if d and d.platform == "cloud" then
		return "cloud"
	elseif d and d.platform == "linux" and not d.here then
		return "remote-server"
	end
	return "laptop"
end

local function panel(): any
	return (boundary.load("@app/screens/panel"))
end

type RowProps = {
	id: string,
	icon: string | () -> string,
	label: () -> string,
	trail: Node?,
	menu: boolean?,
	muted: boolean?,
	on_click: (() -> ())?,
}

local function row(props: RowProps): Node
	local act = props.on_click
	return ui.row {
		id = props.id,
		w_full = true,
		h = ui.raw(34, WHY),
		px = ui.raw(10, WHY),
		gap = ui.raw(11, WHY),
		items = "center",
		rounded = ui.raw(9, WHY),
		role = if act then "button" else nil,
		label = props.label,
		focusable = act ~= nil,
		cursor = if act then "pointer" else "default",
		hover = if act then { bg = "$hover" } else nil,
		focus = if act then control.ring else nil,
		on_click = act,
		on_key = function(e: any): boolean?
			if act and (e.key == "Enter" or e.key == " ") and not (e.cmd or e.ctrl or e.alt) then
				act()
				return true
			end
			return nil
		end,
		ui.row {
			shrink = 0,
			color = if props.muted then "$text.secondary" else "$text.primary",
			ui.presence(function(): string
				local icon = props.icon
				return if type(icon) == "function" then icon() else icon
			end, function(name: string): Node
				return Icon { name = name :: any, size = ui.raw(17, WHY) }
			end),
		},
		ui.text {
			props.label,
			literal = true,
			truncate = true,
			grow = 1,
			min_w = 0,
			text_size = ui.raw(13.5, WHY),
			color = if props.muted then "$text.secondary" else "$text.primary",
		},
		props.trail,
		if props.menu
			then ui.row { shrink = 0, color = "$text.secondary", Icon { name = "lucide-chevron-down", size = 14 } }
			else nil,
	}
end

M.Card = ui.component(function(props: { session: () -> string? }): Node
	local function chat(): sessions.Chat?
		return chat_of(props.session())
	end
	local function target(): any?
		local c = chat()
		local root = if c then c.work_path or c.cwd else nil
		return if c and root then { chat = c.id, device = c.device, root = root } else nil
	end
	local function local_folder(): boolean
		local c = chat()
		local d = if c then device_of(c.device) else nil
		return target() ~= nil and d ~= nil and d.here == true
	end
	local function open_changes()
		local sid = props.session()
		local p = panel()
		if sid and p and p.open_plugin_tab then
			p.open_plugin_tab({ id = sid }, {
				plugin = "git",
				kind = "git-changes",
				title = locale.s("chat.card.changes"),
				icon = "file-diff",
				data = { session = sid },
			})
			shell_view.set_files(true)
		end
	end

	local bound = Actions.bind(props.session, function(title: string, run: { [string]: any })
		local sid = props.session()
		if sid then
			panel_state.open(`chat:{sid}`, "terminal", { title = title, run = run })
			shell_view.set_files(true)
		end
	end)
	local actions_open = ui.use_state(false)
	local function action_items(): { BotMenu.Entry }
		local out: { BotMenu.Entry } = {}
		for _, a in bound.list:get() do
			table.insert(out, {
				id = `chat-card-action-{a.id}`,
				label = if a.setup then locale.s("panel.home.setup_suffix", { name = a.name }) else a.name,
				icon = Actions.icon(a.icon),
				disabled = not bound.can_run(),
				on_select = function()
					bound.run(a)
				end,
			})
		end
		for _, d in bound.imports:get() do
			table.insert(out, {
				id = `chat-card-import-{d.name}`,
				label = d.name,
				icon = Actions.icon(d.icon),
				value = function(): string
					return locale.s("panel.home.import")
				end,
				on_select = function()
					bound.edit { name = d.name, command = d.command, icon = d.icon, setup = d.setup }
				end,
			})
		end
		if #out > 0 then
			table.insert(out, { separator = true })
		end
		table.insert(out, {
			id = "chat-card-add-action",
			label = locale.s("panel.home.add_action"),
			icon = "plus",
			on_select = function()
				bound.edit(nil)
			end,
		})
		return out
	end

	local slot_session = ui.use_state(nil :: any)
	ui.effect(function()
		local c = chat()
		local now = if c and (c.live_branch or c.branch or "") ~= "" then c.id else nil
		local was = slot_session:peek()
		if (if was then was.session else nil) ~= now then
			slot_session:set(if now then { session = now } else nil)
		end
	end)

	return ui.column(glass.style({
		id = "chat-card",
		w = M.W,
		p = ui.raw(6, WHY),
		rounded = ui.raw(16, WHY),
		origin = { 1, 0 },
		pointer_events = "auto",
		row {
			id = "chat-card-device",
			icon = function(): string
				local c = chat()
				return device_icon(if c then device_of(c.device) else nil)
			end,
			label = function(): string
				local c = chat()
				local d = if c then device_of(c.device) else nil
				return if d then d.name else locale.s("sidecar.local")
			end,
			trail = ui.show(
				local_folder,
				ui.row {
					id = "chat-card-folder",
					role = "button",
					label = locale.t("chat.card.reveal"),
					tooltip = Tooltip { locale.t("chat.card.reveal") },
					focusable = true,
					cursor = "pointer",
					shrink = 0,
					w = 24,
					h = 24,
					items = "center",
					justify = "center",
					rounded = "$radius.md",
					color = "$text.secondary",
					hover = { bg = "$hover", color = "$text.primary" },
					focus = control.ring,
					on_click = function()
						local t = target()
						if t then
							files.reveal(t, ""):after(function() end, function() end)
						end
					end,
					Icon { name = "folder", size = 15 },
				}
			),
		},
		ui.show(
			local_folder,
			row {
				id = "chat-card-open-in",
				icon = "lucide-box",
				label = function(): string
					return locale.s("chat.card.open_in", { app = locale.s("chat.card.finder") })
				end,
				on_click = function()
					local t = target()
					if t then
						files.open_external(t, ""):after(function() end, function() end)
					end
				end,
			}
		),
		ui.show(
			function(): boolean
				return bound.place() ~= nil
			end,
			BotMenu.Popup {
				open = actions_open,
				label = locale.t("sidecar.actions"),
				width = M.W - 12,
				placement = { side = "bottom", align = "start", offset = 4 },
				items = action_items,
				anchor = row {
					id = "chat-card-actions",
					icon = "lucide-wrench",
					label = function(): string
						return locale.s("sidecar.actions")
					end,
					menu = true,
					on_click = function()
						actions_open:set(not actions_open:peek())
					end,
				},
			}
		),
		ui.show(
			function(): boolean
				return git_on(chat())
			end,
			ui.column {
				w_full = true,
				ui.div { h = 1, mx = ui.raw(4, WHY), my = ui.raw(6, WHY), bg = "$border" },
				row {
					id = "chat-card-branch",
					icon = "git-branch",
					label = function(): string
						local c = chat()
						local b = if c then c.live_branch or c.branch else nil
						return if b and b ~= "" then b else locale.s("sidecar.no_branch")
					end,
				},
				row {
					id = "chat-card-commit",
					icon = "pull-request",
					label = function(): string
						return locale.s("chat.card.commit")
					end,
					on_click = open_changes,
				},
				row {
					id = "chat-card-changes",
					icon = "file-diff",
					muted = true,
					label = function(): string
						return locale.s("chat.card.changes")
					end,
					on_click = open_changes,
					trail = ui.row {
						id = "chat-card-changes-slot",
						shrink = 0,
						items = "center",
						provide = { panel_branch = slot_session },
						ui.slot("zeron.panel.branch", { direction = "row", items = "center" }),
					},
				},
			}
		),
		bound.dialogs,
	}, "popup"))
end)

local function toggle(id: string, label: string, on: () -> boolean, glyph: Node, on_click: () -> ()): Node
	return ui.row {
		id = id,
		role = "button",
		label = locale.t(label),
		tooltip = Tooltip { locale.t(label) },
		selected = on,
		focusable = true,
		cursor = "pointer",
		w = ui.raw(28, WHY),
		h = ui.raw(26, WHY),
		items = "center",
		justify = "center",
		rounded = ui.raw(7, WHY),
		bg = function(): string
			return if on() then "$zeron.secondary.bg" else "#00000000"
		end,
		color = function(): string
			return if on() then "$text.primary" else "$text.secondary"
		end,
		hover = { bg = "$hover", color = "$text.primary" },
		focus = control.ring,
		on_click = on_click,
		on_key = function(e: any): boolean?
			if (e.key == "Enter" or e.key == " ") and not (e.cmd or e.ctrl or e.alt) then
				on_click()
				return true
			end
			return nil
		end,
		glyph,
	}
end

M.Corner = ui.component(function(props: { session: () -> string? }): Node
	local reduced = motion.reduced_now()
	local close_ms = math.max(motion.ms(CLOSE_MS), 1)
	local held = ui.use_state(M.open:peek())
	local at = ui.computed(function(): number
		return if M.open:get() then 1 else 0
	end)
	local under_head = motion.tween(
		ui.computed(function(): number
			local room = M.room:get()
			return if room > 0 and room < ROOM and BotChat.of(props.session()) ~= nil then BotChat.HEAD_H - HEAD_CLEAR else 0
		end),
		{ duration = SHIFT_MS, ease = EASE }
	)
	ui.effect(function()
		if M.open:get() then
			held:set(true)
		else
			ui.after(close_ms, function()
				if not M.open:peek() then
					held:set(false)
				end
			end)
		end
	end)
	return ui.div {
		id = "chat-corner",
		absolute = true,
		z = 1,
		right = ui.raw(10, WHY),
		top = ui.raw(8, WHY),
		pointer_events = "none",
		ui.show(
			function(): boolean
				return props.session() ~= nil
			end,
			ui.column {
				items = "end",
				gap = ui.raw(10, WHY),
				pointer_events = "none",
				ui.row {
					id = "chat-toggles",
					role = "toolbar",
					label = locale.t("chat.card.toggles"),
					p = ui.raw(2, WHY),
					gap = ui.raw(2, WHY),
					rounded = ui.raw(9, WHY),
					border = 1,
					border_color = "$border",
					bg = "$surface.base",
					pointer_events = "auto",
					toggle("chat-toggle-card", "chat.card.toggle_card", function(): boolean
						return M.open:get()
					end, Icon { name = "card-lines" :: any, size = 16 }, function()
						M.open:set(not M.open:peek())
					end),
					toggle("chat-toggle-terminal", "chat.card.toggle_terminal", function(): boolean
						local p = panel()
						return p ~= nil and p.terminal_shown ~= nil and p.terminal_shown()
					end, Icon { name = "dock-bottom" :: any, size = 16 }, function()
						local p = panel()
						if p and p.toggle_terminal then
							p.toggle_terminal()
						end
					end),
					toggle(
						"chat-toggle-panel",
						"chat.card.toggle_panel",
						function(): boolean
							return shell_view.files:get() == true
						end,
						SidebarIcon {
							open = shell_view.files :: any,
							side = "right",
							size = 16,
							ink = (function(): string
								return if shell_view.files:get() == true then "$text.primary" else "$text.secondary"
							end) :: any,
						},
						fit.toggle_panel
					),
				},
				ui.div {
					origin = { 1, 0 },
					mt = under_head,
					opacity = ui.tween(at, { duration = close_ms, ease = "outQuad" }),
					scale = ui.tween(at, { from = if reduced then 1 else 0.96, to = 1, duration = close_ms, ease = "outQuad" }),
					ui.presence(function(): string
						return if held:get() then "open" else "shut"
					end, function(which: string): Node
						if which == "shut" then
							return ui.div {}
						end
						return M.Card { session = props.session }
					end, {
						enter = if reduced
							then { opacity = { 0, 1 }, duration = 120 }
							else { opacity = { 0, 1 }, scale = { 0.96, 1 }, y = { -4, 0 }, duration = OPEN_MS, ease = EASE },
					}),
				},
			}
		),
	}
end)

return M
