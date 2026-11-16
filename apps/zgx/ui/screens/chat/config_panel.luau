local Icon = require("@ds/icon")
local material = require("@ds/material")
local status = require("@ds/status")
local motion = require("@ds/motion")
local prefs = require("@app/screens/settings/prefs")

local M = {}

ui.theme.tokens {
	["zeron.config.hairline"] = "alpha({text.primary}, 0.06)",
	["zeron.config.selected"] = "alpha({accent}, 0.16)",
	["zeron.config.chip"] = "alpha({accent}, 0.14)",
	["zeron.config.ink"] = "alpha({text.primary}, 0.06)",
	["zeron.config.well"] = "alpha({text.primary}, 0.03)",
	["zeron.config.found"] = "alpha({warning}, 0.3)",
	["zeron.config.switch"] = "alpha({text.primary}, 0.18)",
	["zeron.config.heat1"] = "alpha({accent}, 0.25)",
	["zeron.config.heat2"] = "alpha({accent}, 0.45)",
	["zeron.config.heat3"] = "alpha({accent}, 0.7)",
}

local function g(n: number): any
	return ui.raw(n, "gpui's /config panel")
end

local TABS_CLAUDE = { "status", "config", "stats", "zeron" }
local TABS_OTHER = { "zeron" }
local FILTERS = { "all", "zeron", "terminal" }
local HEAT = { "$zeron.config.ink", "$zeron.config.heat1", "$zeron.config.heat2", "$zeron.config.heat3", "$accent" }
local ROW_H = 26
local STATUS_KEYS: { [string]: string } = {
	Version = "version",
	Session = "session",
	Folder = "folder",
	Model = "model",
	["Permission mode"] = "permission_mode",
	["Output style"] = "output_style",
	["MCP servers"] = "mcp_servers",
	Memory = "memory",
	Plugins = "plugins",
}
local EASE_IN, EASE_OUT = "cubicBezier(.32,.72,0,1)", "cubicBezier(.25,.1,.25,1)"

-- What the host answers (src/claude_panel.rs): each call one place, so a fixture can stand in for it.
M.view = ui.shared("claude_panel", nil :: any)
M.host = {
	open = function(args: any)
		pcall(ui.invoke, "claude_panel.open", args)
	end,
	stats = function()
		pcall(ui.invoke, "claude_panel.stats", {})
	end,
	close = function()
		pcall(ui.invoke, "claude_panel.close", {})
	end,
	rows = function(args: any): any
		local ok, v = pcall(ui.call_sync, "claude_panel.rows", args)
		return if ok then v else nil
	end,
	status = function(args: any): any
		local ok, v = pcall(ui.call_sync, "claude_panel.status", args)
		return if ok then v else nil
	end,
	set = function(args: any): any
		local ok, v = pcall(ui.call_sync, "claude_panel.set", args)
		return if ok then v else nil
	end,
}

M.shown = ui.state("chat_config_panel", false)
M.chat = nil :: string?
M.claude = ui.state("chat_config_panel_claude", false)
local tab = ui.state("chat_config_tab", "config")
local filter = ui.state("chat_config_filter", "all")
local query = ui.state("chat_config_query", "")
local selected = ui.state("chat_config_selected", 1)
local expanded = ui.state("chat_config_expanded", nil :: string?)
local option_sel = ui.state("chat_config_option", 1)
local editing = ui.state("chat_config_editing", nil :: string?)
local edit_text = ui.state("chat_config_edit_text", "")
local zeron_sel = ui.state("chat_config_zeron_selected", 1)
local zeron_was = ui.state("chat_config_zeron_was", {} :: { [string]: any })
local focus_ask = ui.state("chat_config_focus", 0)
M.reopen_ask = ui.state("chat_config_reopen", nil :: { chat: string }?)

local function defaults(): any
	return prefs.mission_defaults:get()
end
local function set_default(key: string, value: any)
	prefs.mission_defaults:set_key({ key }, value)
end
local STEPS = { small = 5, medium = 10, large = 50 }
M.ZERON_ROWS = {
	{
		id = "mission.size",
		label = "chat.config_panel.zeron.size",
		kind = "enum",
		options = { "small", "medium", "large", "unlimited" },
		option_label = function(o: string): string
			return ui.t(`ds.composer.mission.size_{o}`):get()
		end,
		option_detail = function(o: string): string
			local n = STEPS[o]
			return if n then ui.t("ds.composer.mission.size_steps", { n = n }):get() else ui.t("ds.composer.mission.size_no_limit"):get()
		end,
		get = function(): string
			return defaults().size or "medium"
		end,
		set = function(v: string)
			set_default("size", v)
		end,
	},
	{
		id = "mission.agents",
		label = "chat.config_panel.zeron.agents",
		kind = "enum",
		options = { "by_role", "same" },
		option_label = function(o: string): string
			return ui.t(if o == "same" then "ds.composer.mission.agents_same" else "ds.composer.mission.agents_by_role"):get()
		end,
		get = function(): string
			return if defaults().same_agents then "same" else "by_role"
		end,
		set = function(v: string)
			set_default("same_agents", v == "same")
		end,
	},
	{
		id = "mission.approve",
		label = "chat.config_panel.zeron.approve",
		kind = "bool",
		options = { "true", "false" },
		get = function(): string
			return if defaults().approve == false then "false" else "true"
		end,
		set = function(v: string)
			set_default("approve", v == "true")
		end,
	},
}

local function tabs(): { string }
	return if M.claude:get() then TABS_CLAUDE else TABS_OTHER
end

function M.open(args: { chat: string?, device: string?, space: string?, claude: boolean })
	if M.shown:peek() then
		focus_ask:set(focus_ask:peek() + 1)
		return
	end
	local was: { [string]: any } = {}
	for _, r in M.ZERON_ROWS do
		was[r.id] = r.get()
	end
	M.chat = args.chat
	ui.batch(function()
		M.claude:set(args.claude)
		tab:set(if args.claude then "config" else "zeron")
		filter:set("all")
		query:set("")
		selected:set(1)
		zeron_sel:set(1)
		expanded:set(nil)
		editing:set(nil)
		zeron_was:set(was)
		M.shown:set(true)
	end)
	M.host.open(args)
	focus_ask:set(focus_ask:peek() + 1)
end

function M.retarget(args: { chat: string?, device: string?, space: string?, claude: boolean })
	if not M.shown:peek() or M.claude:peek() == args.claude then
		return
	end
	ui.batch(function()
		M.claude:set(args.claude)
		if not args.claude then
			tab:set("zeron")
		end
		expanded:set(nil)
		editing:set(nil)
	end)
	M.host.open(args)
end

function M.close()
	if not M.shown:peek() then
		return
	end
	ui.batch(function()
		expanded:set(nil)
		editing:set(nil)
		M.shown:set(false)
	end)
	M.host.close()
end

function M.reopen(chat: string)
	M.reopen_ask:set({ chat = chat })
end

local function chips_of(props: any): any
	return if props.chips then props.chips() else nil
end

export type Props = {
	chips: () -> any,
	on_chip: (chip: string, value: string) -> (),
	height: () -> number,
	on_closed: () -> (),
}

local function kbd(label: string): Node
	return ui.div {
		shrink = 0,
		px = g(4),
		rounded = g(4),
		border = 1,
		border_color = "$border",
		font = "$font.mono",
		text_size = g(10.5),
		color = "$text.secondary",
		ui.text { label, wrap = false },
	}
end

local function highlight(text: string, q: string, style: { [string]: any }): Node
	local lower, needle = string.lower(text), string.lower(q)
	local at = if needle ~= "" then string.find(lower, needle, 1, true) else nil
	local function run(words: string, found: boolean): Node
		local t: { [any]: any } = { words, wrap = false }
		for k, v in style do
			t[k] = v
		end
		if found then
			return ui.div { rounded = g(2), bg = "$zeron.config.found", ui.text(t) }
		end
		return ui.text(t)
	end
	if not at then
		return run(text, false)
	end
	local stop = at + #needle - 1
	return ui.row {
		shrink = 0,
		run(string.sub(text, 1, at - 1), false),
		run(string.sub(text, at, stop), true),
		run(string.sub(text, stop + 1), false),
	}
end

local function note(text: string): Node
	return ui.div {
		p = g(30),
		w_full = true,
		items = "center",
		ui.text { text, text_size = g(12.5), color = "$text.tertiary", wrap = true },
	}
end

local function waiting(text: any): Node
	local began = ui.now()
	local tick = ui.use_state(0)
	local alive = true
	local function step()
		if not alive then
			return
		end
		tick:set(math.floor((ui.now() - began) / 1000))
		ui.after(1000, step)
	end
	ui.after(1000, step)
	ui.on_cleanup(function()
		alive = false
	end)
	return ui.row {
		p = g(16),
		w_full = true,
		gap = g(8),
		justify = "center",
		items = "center",
		text_size = g(12.5),
		color = "$text.tertiary",
		status.StatusIcon { kind = "working", size = 12 },
		ui.text { text, wrap = false },
		ui.text {
			function(): string
				return `{tick:get()}s`
			end,
			tabular = true,
			wrap = false,
		},
	}
end

local function switch(on: boolean): Node
	return ui.div {
		w = g(26),
		h = g(16),
		shrink = 0,
		rounded = "$radius.full",
		bg = if on then "$accent" else "$zeron.config.switch",
		relative = true,
		ui.div {
			absolute = true,
			top = g(2),
			left = if on then g(12) else g(2),
			w = g(12),
			h = g(12),
			rounded = "$radius.full",
			bg = "#ffffff",
		},
	}
end

local function line(r: any, on: boolean, q: string, on_click: () -> (), on_value: () -> ()): Node
	local value: Node
	if r.kind == "bool" then
		value = ui.row {
			gap = g(6),
			items = "center",
			switch(r.value == "true"),
			ui.text { r.shown or r.value, color = if r.value == "true" then "$text.primary" else "$text.secondary", wrap = false },
		}
	elseif r.kind == "text" and r.value == "" then
		value = ui.text { ui.t("chat.config_panel.default"), color = "$text.tertiary", wrap = false }
	else
		value = ui.row {
			gap = g(6),
			items = "center",
			min_w = 0,
			highlight(r.shown or r.value, q, {}),
			if #r.options > 2 and r.writable then ui.text { "›", color = "$text.tertiary", wrap = false } else nil,
		}
	end
	local dim = if r.writable then 1 else 0.5
	return ui.row {
		class = "claude-config-row",
		id = `claude-config-row-{r.id}`,
		role = "listitem",
		label = r.label,
		selected = on,
		w_full = true,
		gap = g(10),
		px = g(8),
		py = g(5),
		items = "center",
		rounded = g(7),
		text_size = g(12.5),
		bg = if on then "$zeron.config.selected" else nil,
		hover = if on then nil else { bg = "$hover" },
		on_click = on_click,
		ui.text { "❯", w = g(10), shrink = 0, color = "$accent", opacity = if on then 1 else 0, wrap = false },
		ui.row {
			w = "46%",
			shrink = 0,
			min_w = 0,
			gap = g(6),
			items = "baseline",
			overflow = "hidden",
			opacity = dim,
			ui.div { shrink = 0, highlight(r.label, q, { color = "$text.primary" }) },
			ui.div { min_w = 0, overflow = "hidden", highlight(r.id, q, { font = "$font.mono", text_size = g(10.5), color = "$text.tertiary" }) },
		},
		ui.row {
			id = `claude-config-value-{r.id}`,
			grow = 1,
			min_w = 0,
			gap = g(6),
			items = "center",
			font = "$font.mono",
			text_size = g(12),
			opacity = dim,
			cursor = if r.writable then "pointer" else nil,
			on_click = on_value,
			value,
			if r.changed then ui.div { w = g(6), h = g(6), shrink = 0, rounded = "$radius.full", bg = "$accent" } else nil,
		},
		if r.tag
			then ui.div {
				shrink = 0,
				px = g(6),
				rounded = g(4),
				text_size = g(10),
				bg = if r.chip then "$zeron.config.chip" else "$zeron.config.ink",
				color = if r.chip then "$accent" else "$text.secondary",
				ui.text { r.tag, wrap = false },
			}
			else nil,
	}
end

local function options_list(r: any, current: string, hot: number, on_pick: (i: number) -> ()): Node
	local items: { any } = {
		ml = g(28),
		mr = g(8),
		mt = g(2),
		mb = g(6),
		p = g(4),
		rounded = g(8),
		border = 1,
		border_color = "$border",
		bg = "$zeron.config.well",
	}
	for i, o in r.options do
		local cur = o == current
		table.insert(items, ui.row {
			id = `claude-config-option-{i}`,
			role = "menuitem",
			selected = cur,
			gap = g(8),
			px = g(8),
			py = g(4),
			items = "center",
			rounded = g(6),
			cursor = "pointer",
			font = "$font.mono",
			text_size = g(12),
			bg = if i == hot then "$zeron.config.selected" else nil,
			hover = if i == hot then nil else { bg = "$hover" },
			on_click = function()
				on_pick(i)
			end,
			ui.div {
				w = g(7),
				h = g(7),
				shrink = 0,
				rounded = "$radius.full",
				border = 1,
				border_color = if cur then "$accent" else "$text.secondary",
				bg = if cur then "$accent" else nil,
			},
			ui.text { if r.option_label then r.option_label(o) else o, wrap = false },
			if r.option_detail then ui.text { r.option_detail(o), color = "$text.tertiary", wrap = false } else nil,
		})
	end
	return ui.column(items)
end

function M.Panel(props: Props): Node
	local root = ui.ref()
	local search = ui.ref()
	local text_field = ui.ref()
	local in_search = ui.state("chat_config_in_search", false)
	local in_text = ui.state("chat_config_in_text", false)
	local offset = ui.state("chat_config_offset", 0)
	local list_h = ui.state("chat_config_list_h", 0)

	ui.effect(function()
		local _ = focus_ask:get()
		local el = root:get()
		if el and M.shown:peek() then
			ui.after(0, function()
				local now = root:peek()
				if now then
					now:focus()
				end
			end)
		end
	end)
	ui.effect(function()
		if tab:get() == "stats" and M.shown:peek() then
			M.host.stats()
		end
	end)
	local query_was = query:peek()
	ui.effect(function()
		local q = query:get()
		if q ~= query_was then
			query_was = q
			selected:set(1)
			expanded:set(nil)
		end
	end)

	local function view(): any
		return M.view:get() or {}
	end
	local listed = ui.computed(function(): any
		local _ = view()
		local got = M.host.rows { chips = chips_of(props), filter = filter:get(), query = query:get() }
		return if type(got) == "table" then got else { rows = {}, counts = {} }
	end)
	local function rows(): { any }
		return listed:get().rows or {}
	end
	local zeron_rows = ui.computed(function(): { any }
		local out = {}
		local was = zeron_was:get()
		local sel = zeron_sel:get()
		local open, hot = expanded:get(), option_sel:get()
		for i, z in M.ZERON_ROWS do
			local v = z.get()
			table.insert(out, {
				id = z.id,
				label = ui.t(z.label):get(),
				value = v,
				shown = if z.option_label then z.option_label(v) else v,
				options = z.options,
				kind = z.kind,
				writable = true,
				changed = was[z.id] ~= nil and was[z.id] ~= v,
				i = i,
				on = i == sel,
				open = open == z.id,
				hot = if open == z.id then hot else 0,
			})
		end
		return out
	end)
	local function sel_index(): number
		return math.clamp(selected:get(), 1, math.max(1, #rows()))
	end
	local shown_rows = ui.computed(function(): { any }
		local out = {}
		local sel, open, edit, hot = sel_index(), expanded:get(), editing:get(), option_sel:get()
		local q = string.match(query:get(), "^%s*(.-)%s*$") or ""
		for i, r in rows() do
			local row = table.clone(r)
			row.i, row.on, row.q = i, i == sel, q
			row.open = open == r.id
			row.hot = if row.open then hot else 0
			row.editing = edit == r.id
			table.insert(out, row)
		end
		return out
	end)

	local function refocus()
		ui.after(0, function()
			local el = root:peek()
			if el then
				el:focus()
			end
		end)
	end
	local function reveal(i: number)
		local h = list_h:peek()
		if h <= 0 then
			return
		end
		local top, bottom = (i - 1) * ROW_H, i * ROW_H
		if top < offset:peek() then
			offset:set(top)
		elseif bottom > offset:peek() + h then
			offset:set(bottom - h)
		end
	end

	local function set_value(r: any, value: string)
		local got = M.host.set { id = r.id, value = value, chips = chips_of(props) }
		if type(got) == "table" and got.change == "chip" and got.value and got.value ~= "" then
			props.on_chip(got.chip, got.value)
		end
	end
	local function activate(i: number)
		local r = rows()[i]
		if not r then
			return
		end
		selected:set(i)
		if not r.writable then
			return
		end
		if r.kind == "bool" then
			set_value(r, if r.value == "true" then "false" else "true")
		elseif r.kind == "text" then
			expanded:set(nil)
			if editing:peek() == r.id then
				editing:set(nil)
				refocus()
			else
				edit_text:set(r.value)
				editing:set(r.id)
				ui.after(0, function()
					local el = text_field:peek()
					if el then
						el:focus()
					end
				end)
			end
		elseif #r.options == 2 then
			local at = table.find(r.options, r.value) or 1
			set_value(r, r.options[at % 2 + 1])
		else
			editing:set(nil)
			if expanded:peek() == r.id then
				expanded:set(nil)
			else
				expanded:set(r.id)
				option_sel:set(table.find(r.options, r.value) or 1)
				reveal(i)
			end
		end
	end
	local function pick_option(o: number)
		local id = expanded:peek()
		for _, r in rows() do
			if r.id == id and r.options[o] then
				set_value(r, r.options[o])
			end
		end
		expanded:set(nil)
		refocus()
	end
	local function save_text()
		local id = editing:peek()
		if not id then
			return
		end
		editing:set(nil)
		refocus()
		local value = string.match(edit_text:peek(), "^%s*(.-)%s*$") or ""
		for _, r in rows() do
			if r.id == id then
				set_value(r, value)
			end
		end
	end
	local function activate_zeron(i: number)
		local z = M.ZERON_ROWS[i]
		if not z then
			return
		end
		zeron_sel:set(i)
		local v = z.get()
		local at = table.find(z.options, v) or 1
		if #z.options <= 2 then
			expanded:set(nil)
			z.set(z.options[at % #z.options + 1])
		elseif expanded:peek() == z.id then
			expanded:set(nil)
		else
			expanded:set(z.id)
			option_sel:set(at)
		end
	end
	local function pick_zeron(o: number)
		for _, z in M.ZERON_ROWS do
			if z.id == expanded:peek() and z.options[o] then
				z.set(z.options[o])
			end
		end
		expanded:set(nil)
		refocus()
	end
	local function select_tab(t: string)
		ui.batch(function()
			tab:set(t)
			expanded:set(nil)
			editing:set(nil)
		end)
		refocus()
	end

	local function keys(e: any): boolean
		if e.cmd or e.meta or e.ctrl or e.alt then
			return false
		end
		local k = e.key
		local t = tab:peek()
		if k == "Tab" then
			local list = tabs()
			local at = table.find(list, t) or 1
			local n = #list
			select_tab(list[if e.shift then (at - 2) % n + 1 else at % n + 1])
			return true
		elseif k == "Escape" then
			if expanded:peek() then
				expanded:set(nil)
			elseif editing:peek() then
				editing:set(nil)
				refocus()
			elseif query:peek() ~= "" then
				query:set("")
				selected:set(1)
				refocus()
			else
				M.close()
				props.on_closed()
			end
			return true
		elseif k == "Enter" and in_text:peek() then
			save_text()
			return true
		elseif in_text:peek() then
			return false
		elseif t == "zeron" then
			local n = #M.ZERON_ROWS
			local open = expanded:peek()
			if k == "ArrowDown" or k == "ArrowUp" then
				local by = if k == "ArrowDown" then 1 else -1
				local z = M.ZERON_ROWS[zeron_sel:peek()]
				if open and z and z.id == open then
					option_sel:set(math.clamp(option_sel:peek() + by, 1, #z.options))
				else
					zeron_sel:set(math.clamp(zeron_sel:peek() + by, 1, n))
				end
				return true
			elseif k == "Enter" or k == " " then
				if open then
					pick_zeron(option_sel:peek())
				else
					activate_zeron(zeron_sel:peek())
				end
				return true
			end
			return false
		elseif t ~= "config" then
			return false
		end
		if k == "ArrowDown" or k == "ArrowUp" then
			local down = k == "ArrowDown"
			local id = expanded:peek()
			if id then
				for _, r in rows() do
					if r.id == id then
						option_sel:set(math.clamp(option_sel:peek() + (if down then 1 else -1), 1, math.max(1, #r.options)))
					end
				end
			elseif #rows() > 0 then
				local i = math.clamp(sel_index() + (if down then 1 else -1), 1, #rows())
				selected:set(i)
				reveal(i)
			end
			return true
		elseif (k == "Enter" or k == " ") and not (k == " " and in_search:peek()) then
			if expanded:peek() then
				pick_option(option_sel:peek())
			else
				activate(sel_index())
			end
			return true
		elseif k == "/" and not in_search:peek() then
			local el = search:peek()
			if el then
				el:focus()
			end
			return true
		elseif not in_search:peek() and not e.editing and utf8.len(k or "") == 1 and string.find(k, "%S") then
			ui.batch(function()
				query:append(k)
				selected:set(1)
				expanded:set(nil)
			end)
			local el = search:peek()
			if el then
				el:focus()
				pcall(function()
					local n = utf8.len(query:peek()) or 0
					el:set_selection(n, n)
				end)
			end
			return true
		end
		return false
	end

	local function header(): Node
		local items: { any } = {
			w_full = true,
			shrink = 0,
			items = "end",
			gap = g(2),
			px = g(12),
			pt = g(8),
			border_sides = { 0, 0, 1, 0 },
			border_color = "$zeron.config.hairline",
			ui.text { ui.t("chat.config_panel.title"), mr = g(8), mb = g(9), text_size = g(11.5), color = "$text.tertiary", wrap = false },
		}
		for _, t in TABS_CLAUDE do
			table.insert(items, ui.show(function(): boolean
				return table.find(tabs(), t) ~= nil
			end, ui.div {
				id = `claude-config-tab-{t}`,
				role = "tab",
				label = ui.t(`chat.config_panel.tab.{t}`),
				selected = function(): boolean
					return tab:get() == t
				end,
				relative = true,
				px = g(12),
				pt = g(6),
				pb = g(9),
				cursor = "pointer",
				text_size = g(12.5),
				color = function(): string
					return if tab:get() == t then "$text.primary" else "$text.secondary"
				end,
				hover = { color = "$text.primary" },
				on_click = function()
					select_tab(t)
				end,
				ui.text { ui.t(`chat.config_panel.tab.{t}`), wrap = false },
				ui.show(function(): boolean
					return tab:get() == t
				end, ui.div { absolute = true, left = g(10), right = g(10), bottom = -1, h = 2, rounded = g(2), bg = "$accent" }),
			}))
		end
		table.insert(items, ui.div { grow = 1 })
		table.insert(items, ui.button {
			id = "claude-config-close",
			label = ui.t("chat.config_panel.close"),
			w = g(26),
			h = g(26),
			p = 0,
			mb = g(4),
			items = "center",
			justify = "center",
			rounded = g(6),
			bg = "#00000000",
			hover = { bg = "$hover" },
			color = "$text.secondary",
			focus_on_press = false,
			on_click = function()
				M.close()
				props.on_closed()
			end,
			Icon { name = "close", size = 12 },
		})
		return ui.row(items)
	end

	local function config_pane(): Node
		local filters: { any } = { w_full = true, shrink = 0, gap = g(6), px = g(12), pb = g(6), items = "center" }
		for _, f in FILTERS do
			table.insert(filters, ui.button {
				id = `claude-config-filter-{f}`,
				selected = function(): boolean
					return filter:get() == f
				end,
				h = g(20),
				px = g(9),
				py = 0,
				items = "center",
				rounded = "$radius.full",
				border = 1,
				focus_on_press = false,
				text_size = g(11.5),
				weight = "$font.weight.regular",
				border_color = function(): string
					return if filter:get() == f then "$accent" else "$border"
				end,
				color = function(): string
					return if filter:get() == f then "$accent" else "$text.secondary"
				end,
				bg = function(): string
					return if filter:get() == f then "$zeron.config.chip" else "#00000000"
				end,
				on_click = function()
					ui.batch(function()
						filter:set(f)
						selected:set(1)
						expanded:set(nil)
					end)
					refocus()
				end,
				ui.text {
					function(): string
						return ui.t(`chat.config_panel.filter.{f}`, { n = (listed:get().counts or {})[f] or 0 }):get()
					end,
					wrap = false,
				},
			})
		end
		table.insert(filters, ui.text {
			function(): string
				return ui.t("chat.config_panel.shown", { n = #rows() }):get()
			end,
			ml = "auto",
			text_size = g(11.5),
			color = "$text.tertiary",
			wrap = false,
		})
		local list = ui.switch(function(): string
			local v = view().values
			if v == "ready" and #rows() == 0 then
				return "none"
			end
			return v or "loading"
		end, {
			loading = waiting(ui.t("chat.config_panel.reading")),
			failed = ui.text {
				function(): string
					return ui.t("chat.config_panel.read_failed", { error = view().error or "" }):get()
				end,
				p = g(30),
				w_full = true,
				text_size = g(12.5),
				color = "$text.tertiary",
				wrap = true,
			},
			none = ui.text {
				function(): string
					return ui.t("chat.config_panel.no_match", { query = query:get() }):get()
				end,
				p = g(30),
				w_full = true,
				text_size = g(12.5),
				color = "$text.tertiary",
				wrap = true,
			},
			ready = ui.scroll {
				id = "claude-config-list",
				grow = 1,
				min_h = 0,
				w_full = true,
				px = g(6),
				pb = g(8),
				offset = offset,
				on_resize = function(size: { w: number, h: number })
					list_h:set(size.h)
				end,
				ui.each(shown_rows, function(r: any): Node
					return ui.column {
						w_full = true,
						line(r, r.on, r.q, function()
							if sel_index() == r.i then
								activate(r.i)
							else
								selected:set(r.i)
								expanded:set(nil)
								refocus()
							end
						end, function()
							activate(r.i)
						end),
						if r.note and (r.on or r.noted) then ui.text { r.note, pl = g(28), pb = g(4), text_size = g(11.5), color = "$text.tertiary", wrap = true } else nil,
						if r.open then options_list(r, r.value, r.hot, pick_option) else nil,
						if r.editing
							then ui.row {
								ml = g(28),
								mr = g(8),
								mt = g(2),
								mb = g(6),
								gap = g(6),
								items = "center",
								ui.div {
									grow = 1,
									h = g(26),
									px = g(8),
									items = "center",
									rounded = g(6),
									border = 1,
									border_color = "$accent",
									ui.input {
										id = "claude-config-text",
										ref = text_field,
										value = edit_text,
										placeholder = ui.t("chat.config_panel.text_placeholder"),
										w_full = true,
										border = 0,
										bg = "#00000000",
										focus = { border = 0, outline = 0 },
										font = "$font.mono",
										text_size = g(12),
										on_key = keys,
										on_focus = function()
											in_text:set(true)
										end,
										on_blur = function()
											in_text:set(false)
										end,
									},
								},
								ui.button {
									id = "claude-config-save-text",
									h = g(26),
									px = g(10),
									py = 0,
									rounded = g(7),
									border = 1,
									border_color = "$border",
									bg = "#00000000",
									hover = { bg = "$hover" },
									text_size = g(12),
									on_click = save_text,
									ui.text { ui.t("chat.config_panel.save"), wrap = false },
								},
							}
							else nil,
					}
				end, { key = function(r: any): string
					return r.id
				end }),
			},
		})
		return ui.column {
			grow = 1,
			min_h = 0,
			w_full = true,
			ui.row {
				mx = g(12),
				mt = g(10),
				mb = g(6),
				shrink = 0,
				h = g(32),
				px = g(10),
				gap = g(8),
				items = "center",
				rounded = g(8),
				border = 1,
				border_color = function(): string
					return if in_search:get() then "$accent" else "$border"
				end,
				ui.div { shrink = 0, color = "$text.secondary", Icon { name = "search", size = 13 } },
				ui.input {
					id = "claude-config-search",
					ref = search,
					value = query,
					label = ui.t("chat.config_panel.search"),
					placeholder = ui.t("chat.config_panel.search"),
					grow = 1,
					min_w = 0,
					h = g(24),
					px = 0,
					border = 0,
					bg = "#00000000",
					focus = { border = 0, outline = 0 },
					text_size = g(12.5),
					on_key = keys,
					on_focus = function()
						in_search:set(true)
					end,
					on_blur = function()
						in_search:set(false)
					end,
				},
				kbd("/"),
			},
			ui.row(filters),
			list,
		}
	end

	local function kv(key: string, value: string, chip: boolean, rule: boolean): Node
		local lines: { any } = { min_w = 0, overflow = "hidden" }
		for l in string.gmatch(value, "[^\n]+") do
			table.insert(lines, ui.text { l, wrap = false })
		end
		local own = STATUS_KEYS[key]
		return ui.row {
			w_full = true,
			gap = g(12),
			px = g(14),
			py = g(7),
			border_sides = { if rule then 1 else 0, 0, 0, 0 },
			border_color = "$zeron.config.hairline",
			text_size = g(12.5),
			ui.text { if own then ui.t(`chat.config_panel.status.{own}`) else key, w = "34%", shrink = 0, color = "$text.secondary", wrap = true },
			ui.row {
				grow = 1,
				min_w = 0,
				gap = g(6),
				items = "center",
				font = "$font.mono",
				text_size = g(12),
				color = "$text.primary",
				ui.column(lines),
				if chip
					then ui.div {
						px = g(6),
						rounded = g(4),
						font = "$font.sans",
						text_size = g(10),
						bg = "$zeron.config.chip",
						color = "$accent",
						ui.text { "= chip", wrap = false },
					}
					else nil,
			},
		}
	end

	local function status_pane(): Node
		return ui.scroll {
			id = "claude-config-status",
			grow = 1,
			min_h = 0,
			w_full = true,
			p = g(6),
			ui.show(function(): boolean
				return view().status == "loading"
			end, waiting(ui.t("chat.config_panel.asking"))),
			ui.each(
				ui.computed(function(): { any }
					local _ = view()
					local got = M.host.status { chips = chips_of(props) }
					local out = {}
					for i, r in (if type(got) == "table" then got else {}) do
						table.insert(out, { key = r.key, value = r.value, chip = r.chip == true, rule = i > 1 })
					end
					return out
				end),
				function(r: any): Node
					return kv(r.key, r.value, r.chip, r.rule)
				end
			),
			ui.text { ui.t("chat.config_panel.status_footnote"), mx = g(14), mt = g(10), text_size = g(11.5), color = "$text.tertiary", wrap = true },
		}
	end

	local function stats_pane(): Node
		local body = ui.presence(function(): string
			local s = view().stats or {}
			return `{s.state or "idle"}:{view().generation or 0}`
		end, function(): Node
			local s = (M.view:peek() or {}).stats or {}
			if s.state == nil or s.state == "idle" or s.state == "loading" then
				return waiting(ui.t("chat.config_panel.counting"))
			elseif s.state == "failed" then
				return note(s.error or "")
			elseif s.state == "empty" then
				return note(ui.t("chat.config_panel.stats_empty"):get())
			end
			local weeks: { any } = { mx = g(14), my = g(8), gap = g(3) }
			for _, week in s.weeks or {} do
				local cells: { any } = { gap = g(3) }
				for _, level in week do
					table.insert(cells, ui.div { w = g(11), h = g(11), rounded = g(2), bg = HEAT[(level or 0) + 1] or HEAT[1] })
				end
				table.insert(weeks, ui.column(cells))
			end
			local tiles = {
				{ s.favorite or ui.t("chat.config_panel.no_data"):get(), "chat.config_panel.tile.favorite" },
				{ tostring(s.sessions or 0), "chat.config_panel.tile.sessions" },
				{ tostring(s.active_days or 0), "chat.config_panel.tile.active_days" },
				{ ui.t("chat.config_panel.days", { n = s.streak or 0 }):get(), "chat.config_panel.tile.streak" },
				{ ui.t("chat.config_panel.no_data"):get(), "chat.config_panel.tile.longest" },
				{ s.tokens or "0", "chat.config_panel.tile.tokens" },
			}
			local grid: { any } = { mx = g(14), my = g(10), gap = g(8) }
			for row = 0, 1 do
				local line_tiles: { any } = { w_full = true, gap = g(8) }
				for col = 1, 3 do
					local t = tiles[row * 3 + col]
					table.insert(line_tiles, ui.column {
						grow = 1,
						basis = 0,
						min_w = 0,
						px = g(10),
						py = g(8),
						rounded = g(8),
						border = 1,
						border_color = "$border",
						ui.text { t[1], text_size = g(13.5), color = "$text.primary", wrap = false },
						ui.text { ui.t(t[2]), text_size = g(11.5), color = "$text.tertiary", wrap = false },
					})
				end
				table.insert(grid, ui.row(line_tiles))
			end
			return ui.column {
				w_full = true,
				ui.text { ui.t("chat.config_panel.activity"), mx = g(14), mt = g(8), text_size = g(11.5), color = "$text.tertiary", wrap = false },
				ui.row(weeks),
				ui.column(grid),
			}
		end, { w_full = true })
		return ui.scroll {
			id = "claude-config-stats",
			grow = 1,
			min_h = 0,
			w_full = true,
			p = g(6),
			body,
			ui.text { ui.t("chat.config_panel.stats_footnote"), mx = g(14), text_size = g(11.5), color = "$warning", wrap = true },
		}
	end

	local function zeron_pane(): Node
		return ui.column {
			id = "claude-config-zeron",
			grow = 1,
			min_h = 0,
			w_full = true,
			px = g(6),
			pt = g(8),
			ui.each(zeron_rows, function(r: any): Node
				return ui.column {
					w_full = true,
					line(r, r.on, "", function()
						if zeron_sel:peek() == r.i then
							activate_zeron(r.i)
						else
							zeron_sel:set(r.i)
							expanded:set(nil)
							refocus()
						end
					end, function()
						activate_zeron(r.i)
					end),
					if r.open then options_list(M.ZERON_ROWS[r.i], r.value, r.hot, pick_zeron) else nil,
				}
			end, { key = function(r: any): string
				return r.id
			end }),
			ui.div { grow = 1 },
			ui.text { ui.t("chat.config_panel.zeron.footnote"), mx = g(14), my = g(10), text_size = g(11.5), color = "$text.tertiary", wrap = true },
		}
	end

	local function footer(): Node
		local function hint(keys_shown: { string }, label: string): Node
			local items: { any } = { gap = g(4), items = "center", shrink = 0 }
			for _, k in keys_shown do
				table.insert(items, kbd(k))
			end
			table.insert(items, ui.text { ui.t(label), wrap = false })
			return ui.row(items)
		end
		return ui.row {
			w_full = true,
			shrink = 0,
			gap = g(12),
			px = g(14),
			py = g(8),
			items = "center",
			border_sides = { 1, 0, 0, 0 },
			border_color = "$zeron.config.hairline",
			text_size = g(11.5),
			color = "$text.tertiary",
			overflow = "hidden",
			hint({ "↑↓" }, "chat.config_panel.hint.select"),
			hint({ "Enter", "Space" }, "chat.config_panel.hint.change"),
			hint({ "/" }, "chat.config_panel.hint.search"),
			hint({ "Tab" }, "chat.config_panel.hint.tab"),
			hint({ "Esc" }, "chat.config_panel.hint.close"),
			ui.text {
				function(): string
					return if M.claude:get() then view().footer or "" else ""
				end,
				ml = "auto",
				min_w = 0,
				color = "$text.secondary",
				wrap = false,
				truncate = true,
			},
		}
	end

	local card = ui.column {
		id = "claude-config",
		ref = root,
		role = "dialog",
		label = ui.t("chat.config_panel.title"),
		focusable = true,
		focus = { outline = 0 },
		on_key = keys,
		w_full = true,
		h = function(): number
			return props.height()
		end,
		rounded = g(14),
		border = 1,
		border_color = "$border.strong",
		bg = function(): string
			return if material.see_through() then "$zeron.popup.tint" else "$zeron.popup.solid"
		end,
		backdrop_blur = function(): number
			return if material.see_through() then 16 else 0
		end,
		shadow = function(): string
			return if material.see_through() then "$zeron.composer.rest" else "$shadow.lg"
		end,
		overflow = "hidden",
		pointer_events = "auto",
		header(),
		ui.switch(function(): string
			return tab:get()
		end, {
			config = config_pane(),
			status = status_pane(),
			stats = stats_pane(),
			zeron = zeron_pane(),
		}),
		footer(),
	}
	return card
end

function M.Over(props: Props): Node
	return ui.presence(function(): string?
		return if M.shown:get() then "on" else nil
	end, function(): Node
		return M.Panel(props)
	end, {
		absolute = true,
		left = 0,
		right = 0,
		bottom = g(8),
		h = function(): number
			return props.height()
		end,
		pointer_events = "none",
		enter = motion.transition({ opacity = { 0, 1 }, y = { 10, 0 }, duration = 220, ease = EASE_IN }),
		exit = motion.transition({ opacity = 0, y = 4, duration = 100, ease = EASE_OUT }),
	})
end

return M
