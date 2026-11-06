--!strict
-- A project's run actions as a session sees them (crates/ui shell/actions_ui.rs).

local ds = require("@ds")
local locale = require("@ds/locale")
local Icon = require("@ds/icon")
local sessions = require("@account/sessions")
local api = require("@app/screens/panel/home_api")

local M = {}

local actions_by = ui.state("panel_home_actions", {} :: { [string]: any })

M.ICONS = { "play", "test", "lint", "configure", "build", "debug" }
function M.icon(icon: string?): Icon.Name
	for _, n in M.ICONS do
		if n == icon then
			return `action-{n}` :: any
		end
	end
	return "action-play"
end

local function trim(s: string?): string
	return string.match(s or "", "^%s*(.-)%s*$") or ""
end

local function chat_of(id: string?): sessions.Chat?
	if not id then
		return nil
	end
	for _, c in sessions.now:get().chats do
		if c.id == id then
			return c
		end
	end
	return nil
end

local function project_of(id: string?): sessions.Project?
	if not id then
		return nil
	end
	for _, p in sessions.now:get().projects do
		if p.id == id then
			return p
		end
	end
	return nil
end

function M.bind(session: () -> string?, open_action: (title: string, run: { [string]: any }) -> ()): { [string]: any }
	local function context(): { space: string, device: string, slot: string, chat: string }?
		local sid = session()
		local c = chat_of(sid)
		if not c or not sid then
			return nil
		end
		local p = project_of(c.project)
		if not p or p.device ~= c.device then
			return nil
		end
		return { space = p.id, device = c.device, slot = `{c.device}|{p.id}`, chat = sid }
	end
	local ctx = ui.computed(function(): string?
		local c = context()
		return if c then c.slot else nil
	end)
	local function now(): any
		local slot = ctx:get()
		return if slot then actions_by:get()[slot] else nil
	end
	local asked_here: { [string]: boolean } = {}
	local function load(c: { space: string, device: string, slot: string, chat: string })
		asked_here[c.slot] = true
		local was = actions_by:peek()[c.slot]
		actions_by:set_key(c.slot, { state = "loading", snapshot = if was then was.snapshot else nil })
		api.actions(c.space, c.device):after(function(snapshot: any)
			actions_by:set_key(c.slot, { state = "ready", snapshot = snapshot })
		end, function(err: any)
			local m = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
			local before = actions_by:peek()[c.slot]
			actions_by:set_key(c.slot, {
				state = if m == "unsupported" then "unsupported" else "failed",
				message = m,
				snapshot = if before then before.snapshot else nil,
			})
		end)
	end
	ui.effect(function()
		local slot = ctx:get()
		local was = if slot then actions_by:peek()[slot] else nil
		-- (A column that went while it asked took its answer with it: "loading" with no one left to hear.)
		if not slot or (was and (was.state ~= "loading" or asked_here[slot])) then
			return
		end
		local c = context()
		if c then
			load(c)
		end
	end)

	local editing = ui.use_state(false)
	local deleting = ui.use_state(false)
	local edit_id = ui.use_state(nil :: string?)
	local name = ui.use_state("")
	local command = ui.use_state("")
	local icon = ui.use_state("play")
	local setup = ui.use_state(false)
	local problem = ui.use_state(nil :: string?)
	local saving = ui.use_state(false)
	local function edit(a: any?)
		edit_id:set(if a then a.id else nil)
		name:set(if a then a.name or "" else "")
		command:set(if a then a.command or "" else "")
		icon:set(if a and a.icon then a.icon else "play")
		setup:set(a ~= nil and a.setup == true)
		problem:set(nil)
		saving:set(false)
		editing:set(true)
	end
	local function save()
		local c = context()
		if not c or saving:peek() then
			return
		end
		local n, cmd = trim(name:peek()), trim(command:peek())
		local why: string? = nil
		if n == "" then
			why = "name"
		elseif utf8.len(n) and (utf8.len(n) :: number) > 80 then
			why = "name_long"
		elseif cmd == "" then
			why = "command"
		elseif #cmd > 16 * 1024 then
			why = "command_long"
		end
		if why then
			problem:set(locale.s(`panel.home.editor.err_{why}`))
			return
		end
		saving:set(true)
		problem:set(nil)
		api.save_action(c.space, c.device, { id = edit_id:peek(), name = n, command = cmd, icon = icon:peek(), setup = setup:peek() }):after(function(snapshot: any)
			actions_by:set_key(c.slot, { state = "ready", snapshot = snapshot })
			saving:set(false)
			editing:set(false)
		end, function(err: any)
			saving:set(false)
			problem:set(if type(err) == "table" and err.message then tostring(err.message) else tostring(err))
		end)
	end
	local function delete()
		local c = context()
		local id = edit_id:peek()
		if not c or not id then
			return
		end
		saving:set(true)
		api.delete_action(c.space, c.device, id):after(function(snapshot: any)
			actions_by:set_key(c.slot, { state = "ready", snapshot = snapshot })
			saving:set(false)
			editing:set(false)
		end, function(err: any)
			saving:set(false)
			problem:set(if type(err) == "table" and err.message then tostring(err.message) else tostring(err))
		end)
	end
	local function run(a: any)
		local c = context()
		local s = now()
		if not c or not s or s.state ~= "ready" then
			return
		end
		open_action(a.name, { space = c.space, device = c.device, chat = c.chat, action = a.id })
	end

	local list = ui.computed(function(): { any }
		local s = now()
		local snap = if s then s.snapshot else nil
		return if snap then snap.actions or {} else {}
	end)
	local imports = ui.computed(function(): { any }
		local s = now()
		local snap = if s then s.snapshot else nil
		return if snap then snap.importable or {} else {}
	end)
	local function can_run(): boolean
		local s = now()
		return s ~= nil and s.state == "ready"
	end

	local function icon_choice(n: string): Node
		local function on(): boolean
			return icon:get() == n
		end
		return ui.button {
			class = "panel-home-icon-choice",
			role = "radio",
			label = locale.t(`panel.home.editor.icon_{n}`),
			tooltip = locale.s(`panel.home.editor.icon_{n}`),
			checked = on,
			w = 34,
			h = 34,
			items = "center",
			justify = "center",
			rounded = "$radius.lg",
			cursor = "pointer",
			bg = function(): string
				return if on() then "$zeron.selected.flat.bg" else "#00000000"
			end,
			color = function(): string
				return if on() then "$text.primary" else "$text.secondary"
			end,
			hover = { bg = "$hover", color = "$text.primary" },
			focus = require("@ds/control").ring,
			on_click = function()
				icon:set(n)
			end,
			Icon { name = `action-{n}` :: any, size = 16 },
		}
	end
	local icons: { Node } = {}
	for _, n in M.ICONS do
		table.insert(icons, icon_choice(n))
	end

	local editor = ds.Dialog {
		open = editing,
		size = "md",
		title = function(): string
			return locale.s(if edit_id:get() then "panel.home.editor.edit_title" else "panel.home.editor.add_title")
		end,
		confirm = {
			label = function(): string
				return locale.s(if saving:get() then "panel.home.editor.saving" else "panel.home.editor.save")
			end,
			keep_open = true,
			on_click = save,
		},
		ds.TextField { value = name, label = locale.t("panel.home.editor.name"), w_full = true, autofocus = true },
		ds.TextArea { value = command, label = locale.t("panel.home.editor.command"), rows = 3, max_rows = 8, w_full = true, font = "$font.mono" },
		ui.column {
			w_full = true,
			gap = "$space.2",
			ui.text { locale.t("panel.home.editor.icon"), text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.secondary" },
			ui.row { id = "panel-home-icons", role = "radiogroup", label = locale.t("panel.home.editor.icon"), gap = "$space.1", table.unpack(icons) },
		},
		ds.Checkbox { value = setup, label = locale.t("panel.home.editor.setup") },
		ui.show(function(): boolean
			return problem:get() ~= nil
		end, ui.text {
			function(): string
				return problem:get() or ""
			end,
			id = "panel-home-editor-problem",
			literal = true,
			wrap = true,
			text_size = "$font.size.sm",
			color = "$danger",
		}),
		ui.show(function(): boolean
			return edit_id:get() ~= nil
		end, ui.row {
			w_full = true,
			ds.Button {
				locale.t("panel.home.editor.delete"),
				tone = "ghost",
				size = "sm",
				color = "$danger",
				on_click = function()
					deleting:set(true)
				end,
			},
		}),
	}
	local confirm_delete = ds.Dialog {
		open = deleting,
		tone = "danger",
		title = locale.t("panel.home.editor.delete_title") :: any,
		body = function(): string
			return locale.s("panel.home.editor.delete_body", { name = trim(name:get()) })
		end,
		confirm = { label = locale.t("panel.home.editor.delete_confirm") :: any, on_click = delete },
	}

	return {
		now = now,
		place = function(): string?
			return ctx:get()
		end,
		list = list,
		imports = imports,
		can_run = can_run,
		run = run,
		edit = edit,
		retry = function()
			local c = context()
			if c then
				load(c)
			end
		end,
		dialogs = ui.div { hook = false, editor, confirm_delete },
	}
end

return M
