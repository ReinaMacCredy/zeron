-- Settings → Agent profiles: the saved agents, models, modes and efforts
-- (Zeron's crates/ui/src/settings/agent_profiles.rs). The list in its order,
-- each with its brand's badge, its line and notes, "by agent" on one an agent
-- made, arrows to move it and Edit; New profile at the title's end; the host's
-- System prompt under them. The editor is a dialog: name, agent, model, mode,
-- effort, the model's options in pairs, the tools it turns off, its
-- instructions and when to use it (each opening out over the rest, its token
-- count showing), Remove (asked once more), Cancel and Save.
--
-- The model picker's profiles card has its own quick edit page; its More…
-- hands the draft here (screens/chat/composer_profiles `pending`). The two
-- are Zeron's two doors, kept as they are there.

local ds = require("@ds")
local Icon = require("@ds/icon")
local IconButton = require("@ds/icon_button").IconButton
local look = require("@ds/profile_look")
local search = require("@app/screens/settings/search")
local route = require("@app/screens/shell/route")
local store = require("@app/screens/chat/store")
local profiles = require("@app/screens/chat/composer_profiles")

local M = {}

M.ID = "agent-profiles"
M.TITLE = "Agent profiles"

search.index(M.ID, ui.t("settings.agent_profiles.title"), ui.t("settings.agent_profiles.terms"), {
	{ key = "new_profile", title = ui.t("settings.agent_profiles.new"), terms = ui.t("settings.agent_profiles.new_terms") },
	{ key = "system_prompt", title = ui.t("settings.agent_profiles.system_prompt"), terms = ui.t("settings.agent_profiles.system_prompt_terms") },
})

local function t(key: string, args: { [string]: any }?): string
	return ui.t(`settings.agent_profiles.{key}`, args):get()
end
local host = profiles.host
local function message_of(err: any): string
	return if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
end

type Edit = { id: string?, draft: look.Draft, error: string?, saving: boolean, confirm: boolean }

-- The editor, and what's typed in its long fields.
local editor = ui.state("settings_profiles_editor", nil :: Edit?)
local editor_open = ui.state("settings_profiles_editor_open", false)
local name = ui.state("settings_profiles_name", "")
local notes = ui.state("settings_profiles_notes", "")
local instructions = ui.state("settings_profiles_instructions", "")
-- The field opened out over the rest: "instructions", "notes" or "tools".
local expanded = ui.state("settings_profiles_expanded", nil :: string?)
-- What a profile can turn off (`profiles.tools`), and the groups opened.
local tools = ui.state("settings_profiles_tools", nil :: any?)
local tools_open = ui.state("settings_profiles_tools_open", {} :: { [string]: boolean })
-- A reorder that didn't take.
local page_error = ui.state("settings_profiles_error", nil :: string?)
-- The host's own prompt, as read, and as typed.
local prompt = ui.state("settings_profiles_prompt", nil :: string?)
local prompt_text = ui.state("settings_profiles_prompt_text", "")
local prompt_saving = ui.state("settings_profiles_prompt_saving", false)

local function agents(): { any }
	return store.catalog:get().harnesses
end
local function agent_of(id: string): any?
	for _, a in agents() do
		if a.id == id then
			return a
		end
	end
	return nil
end
local function models_of(h: string): { any }
	local out = {}
	for _, m in store.catalog:get().models do
		if m.harness == h then
			table.insert(out, m)
		end
	end
	return out
end
local function bare(value: string): string
	return string.match(value, "^[^/]+/(.+)$") or value
end
local function model_of(d: look.Draft): any?
	if not d.model then
		return nil
	end
	for _, m in models_of(d.harness) do
		if bare(m.value) == d.model then
			return m
		end
	end
	return nil
end

function M.open_editor(id: string?, draft: look.Draft)
	ui.batch(function()
		name:set(draft.name or "")
		notes:set(draft.notes or "")
		instructions:set(draft.instructions or "")
		expanded:set(nil)
		tools:set(nil)
		tools_open:set({})
		editor:set({ id = id, draft = draft, saving = false, confirm = false })
		editor_open:set(true)
	end)
end

local function change(f: (look.Draft) -> ())
	local e = editor:peek()
	if not e then
		return
	end
	local d = table.clone(e.draft)
	d.options = table.clone(d.options or {})
	d.tools_off = table.clone(d.tools_off or {})
	f(d)
	editor:set({ id = e.id, draft = d, saving = e.saving, confirm = false })
end
local function trimmed(s: string): string
	return string.match(s, "^%s*(.-)%s*$") or ""
end

local function save()
	local e = editor:peek()
	if not e or e.saving then
		return
	end
	local d = table.clone(e.draft)
	d.name = trimmed(name:peek())
	d.notes = trimmed(notes:peek())
	d.instructions = trimmed(instructions:peek())
	if d.name == "" then
		editor:set({ id = e.id, draft = e.draft, saving = false, confirm = false, error = t("name_needed") })
		return
	end
	editor:set({ id = e.id, draft = e.draft, saving = true, confirm = false })
	local p = host.save({ id = e.id, draft = d })
	if not p then
		editor:set({ id = e.id, draft = e.draft, saving = false, confirm = false, error = t("save_failed") })
		return
	end
	p:after(function()
		editor_open:set(false)
		editor:set(nil)
	end, function(err: any)
		local now = editor:peek()
		if now then
			editor:set({ id = now.id, draft = now.draft, saving = false, confirm = false, error = message_of(err) })
		end
	end)
end

local function remove()
	local e = editor:peek()
	if not e or not e.id or e.saving then
		return
	end
	if not e.confirm then
		editor:set({ id = e.id, draft = e.draft, saving = false, confirm = true })
		return
	end
	editor:set({ id = e.id, draft = e.draft, saving = true, confirm = true })
	local p = host.remove({ id = e.id })
	if not p then
		return
	end
	p:after(function()
		editor_open:set(false)
		editor:set(nil)
	end, function(err: any)
		local now = editor:peek()
		if now then
			editor:set({ id = now.id, draft = now.draft, saving = false, confirm = false, error = message_of(err) })
		end
	end)
end

-- One place up or down (gpui's `move_profile`): the list as it will stand
-- shown at once is the host's to say; a refusal says so over the list.
function M.move(from: number, to: number)
	local list = profiles.list:peek().profiles
	if to < 1 or to > #list then
		return
	end
	local ids: { string } = {}
	for _, p in list do
		table.insert(ids, p.id)
	end
	ids[from], ids[to] = ids[to], ids[from]
	local p = host.reorder({ ids = ids })
	if p then
		p:after(function()
			page_error:set(nil)
		end, function(err: any)
			page_error:set(t("couldnt_save", { error = message_of(err) }))
		end)
	end
end

local function load_tools()
	local e = editor:peek()
	if not e then
		return
	end
	local p = host.tools({ harness = e.draft.harness, tools_off = e.draft.tools_off })
	if not p then
		tools:set({ groups = {}, unlisted = {}, ceiling_error = t("tools_unavailable_host") })
		return
	end
	p:after(function(v: any)
		tools:set(v)
	end, function(err: any)
		tools:set({ groups = {}, unlisted = {}, ceiling_error = message_of(err) })
	end)
end

local function set_rules(rules: { string }, on: boolean)
	change(function(d)
		local keep: { string } = {}
		for _, r in d.tools_off do
			if not table.find(rules, r) then
				table.insert(keep, r)
			end
		end
		if not on then
			for _, r in rules do
				table.insert(keep, r)
			end
		end
		d.tools_off = keep
	end)
end

local function draft(): look.Draft
	local e = editor:get()
	return if e then e.draft else look.blank("claude-code")
end

-- A switch whose state follows `on()` and whose flip goes to `flip`.
local function switch(label: string, on: () -> boolean, enabled: () -> boolean, flip: (on: boolean) -> ()): Node
	local v = ui.use_state(on())
	ui.effect(function()
		local now = on()
		if v:peek() ~= now then
			v:set(now)
		end
	end)
	return ui.div {
		shrink = 0,
		opacity = function(): number
			return if enabled() then 1 else 0.45
		end,
		pointer_events = function(): string
			return if enabled() then "auto" else "none"
		end,
		ds.Switch {
			value = v,
			label = label,
			on_change = function(now: boolean)
				flip(now)
			end,
		},
	}
end

local function tools_heading(text: string): Node
	return ui.text { text, mt = "$zeron.space.snug", mb = "$zeron.space.hair", text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.tertiary" }
end
local function tools_note(text: string | () -> string): Node
	return ui.text { text, py = "$zeron.space.tight", wrap = true, text_size = "$font.size.sm", color = "$text.tertiary" }
end
local function tools_row(nested: boolean, children: { Child }): Node
	local p: { [any]: any } = { w_full = true, min_h = 36, py = "$space.1", pl = if nested then ui.raw(18, "gpui's nested tool row") else 0, gap = "$zeron.space.snug", items = "center" }
	for _, c in children do
		table.insert(p, c)
	end
	return ui.column { w_full = true, ui.div { w_full = true, h = 1, bg = "$border" }, ui.row(p) }
end
local function tools_text(title: string, detail: string?, mono: boolean?): Node
	return ui.column {
		grow = 1,
		min_w = 0,
		ui.text { title, truncate = true, font = if mono then "$font.mono" else nil, text_size = if mono then "$font.size.sm" else "$font.size.md", color = "$text.primary" },
		if detail and detail ~= "" then ui.text { detail, truncate = true, text_size = "$font.size.sm", color = "$text.tertiary" } else nil,
	}
end

-- What a profile turns off: Zeron's tool groups (each opening to its tools,
-- what Settings → Tools turned off for every chat locked), Claude Code's own
-- tools and MCP servers, and the rules none of those list.
local function tools_list(): Node
	return ui.presence(function(): string
		return if tools:get() then "ready" else "loading"
	end, function(state: string): Node
		if state == "loading" then
			return tools_note(t("tools_loading"))
		end
		local v = tools:peek()
		local function off(rule: string): boolean
			return table.find(draft().tools_off, rule) ~= nil
		end
		local rows: { Node } = { tools_heading(t("zeron_tools")) }
		if v.ceiling_error then
			table.insert(rows, tools_note(t("tools_unavailable", { error = v.ceiling_error })))
		end
		for _, g in v.groups or {} do
			local unlocked: { string } = {}
			for _, tool in g.tools do
				if not tool.locked then
					table.insert(unlocked, tool.rule)
				end
			end
			local function live(): number
				local n = 0
				for _, r in unlocked do
					if not off(r) then
						n += 1
					end
				end
				return n
			end
			table.insert(rows, tools_row(false, {
				ui.button {
					id = `profile-tools-open-{g.id}`,
					label = g.title,
					grow = 1,
					min_w = 0,
					gap = "$zeron.space.tight",
					items = "center",
					on_click = function()
						local now = table.clone(tools_open:peek())
						now[g.id] = not now[g.id] or nil
						tools_open:set(now)
					end,
					ui.div {
						shrink = 0,
						color = "$text.tertiary",
						Icon {
							name = function(): Icon.Name
								return if tools_open:get()[g.id] then "alt-arrow-down" else "alt-arrow-right"
							end,
							size = 12,
						},
					},
					ui.text { g.title, truncate = true, text_size = "$font.size.md", color = "$text.primary" },
					if #unlocked == 0
						then ds.Badge { t("off_in_settings"), tone = "neutral", size = "sm" }
						else ui.show(function(): boolean
							return live() < #unlocked
						end, ds.Badge {
							function(): string
								return t("n_of", { n = live(), total = #unlocked })
							end,
							tone = "accent",
							size = "sm",
						}),
				},
				switch(g.title, function(): boolean
					return #unlocked > 0 and live() > 0
				end, function(): boolean
					return #unlocked > 0
				end, function(on: boolean)
					if on then
						local all: { string } = {}
						for _, tool in g.tools do
							table.insert(all, tool.rule)
						end
						set_rules(all, true)
					else
						set_rules(unlocked, false)
					end
				end),
			}))
			table.insert(rows, ui.show(function(): boolean
				return tools_open:get()[g.id] == true
			end, ui.column {
				w_full = true,
				table.unpack((function(): { Node }
					local out: { Node } = {}
					for _, tool in g.tools do
						table.insert(out, tools_row(true, {
							tools_text(tool.name, if tool.locked then t("off_in_settings") else nil, true),
							switch(tool.name, function(): boolean
								return not tool.locked and not off(tool.rule)
							end, function(): boolean
								return not tool.locked
							end, function(on: boolean)
								set_rules({ tool.rule }, on)
							end),
						}))
					end
					return out
				end)()),
			}))
		end
		local claude = v.claude
		if claude then
			table.insert(rows, tools_heading(t("claude_builtin")))
			if claude.state == "error" then
				table.insert(rows, tools_note(t("claude_failed", { error = claude.error or "" })))
			else
				for _, b in claude.builtin or {} do
					table.insert(rows, tools_row(false, {
						tools_text(b.title, b.summary),
						switch(b.title, function(): boolean
							for _, r in b.rules do
								if off(r) then
									return false
								end
							end
							return true
						end, function(): boolean
							return true
						end, function(on: boolean)
							set_rules(b.rules, on)
						end),
					}))
				end
				table.insert(rows, tools_heading(t("mcp_servers")))
				if #(claude.mcp or {}) == 0 then
					table.insert(rows, tools_note(t("no_mcp")))
				end
				for _, server in claude.mcp or {} do
					table.insert(rows, tools_row(false, {
						tools_text(server.name, server.status),
						switch(server.name, function(): boolean
							return not off(server.rule)
						end, function(): boolean
							return true
						end, function(on: boolean)
							set_rules({ server.rule }, on)
						end),
					}))
				end
			end
		end
		-- (Turned on, a rule nothing lists goes: gpui's "turn on to remove".)
		local unlisted: { string } = v.unlisted or {}
		if #unlisted > 0 then
			table.insert(rows, ui.show(function(): boolean
				for _, r in unlisted do
					if off(r) then
						return true
					end
				end
				return false
			end, tools_heading(t("not_found"))))
			for _, rule in unlisted do
				table.insert(rows, ui.show(function(): boolean
					return off(rule)
				end, tools_row(false, {
					tools_text(rule, t("not_found_hint"), true),
					switch(rule, function(): boolean
						return false
					end, function(): boolean
						return true
					end, function(on: boolean)
						set_rules({ rule }, on)
					end),
				})))
			end
		end
		return ui.scroll { id = "profile-tools", w_full = true, h = 300, ui.column { w_full = true, table.unpack(rows) } }
	end, { w_full = true })
end

local function label_of(text: string | () -> string): Node
	return ui.text { text, mt = "$space.3", mb = ui.raw(5, "gpui's measure"), text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.tertiary" }
end
local function folded_row(id: string, text: () -> string, on_click: () -> ()): Node
	return ui.button {
		id = id,
		label = text,
		w_full = true,
		h = 30,
		mt = "$zeron.space.tight",
		px = ui.raw(9, "gpui's measure"),
		gap = "$space.2",
		items = "center",
		rounded = ui.raw(8, "gpui's corner"),
		border = 1,
		border_color = "$border",
		hover = { bg = "$hover" },
		on_click = on_click,
		ui.text { text, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.sm", color = "$text.tertiary" },
		ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "alt-arrow-down", size = 11 } },
	}
end
local function toggle(id: string, which: string): Node
	return ui.button {
		id = id,
		label = function(): string
			return if expanded:get() == which then t("fold") else t("expand")
		end,
		absolute = true,
		top = 4,
		right = 4,
		w = 22,
		h = 22,
		rounded = ui.raw(6, "gpui's corner"),
		items = "center",
		justify = "center",
		bg = "$zeron.secondary.bg",
		hover = { bg = "$pressed" },
		color = "$text.tertiary",
		on_click = function()
			if expanded:peek() == which then
				expanded:set(nil)
			else
				expanded:set(which)
				if which == "tools" then
					load_tools()
				end
			end
		end,
		Icon {
			name = function(): Icon.Name
				return if expanded:get() == which then "collapse-arrows" else "expand-arrows"
			end,
			size = 12,
		},
	}
end
local function long_field(which: string, value: State<string>, limit: number, rows: number): Node
	local open = function(): boolean
		return expanded:get() == which
	end
	return ui.column {
		w_full = true,
		ui.row {
			w_full = true,
			items = "center",
			label_of(function(): string
				return t(which)
			end),
			ui.div { grow = 1 },
			ui.show(open, ui.text {
				function(): string
					return look.token_count(value:get(), limit)
				end,
				class = `profile-{which}-count`,
				mt = "$space.3",
				mb = ui.raw(5, "gpui's measure"),
				text_size = "$font.size.sm",
				color = "$text.tertiary",
			}),
		},
		ui.div {
			w_full = true,
			relative = true,
			ds.TextArea {
				value = value,
				name = t(which),
				placeholder = t(`{which}_placeholder`),
				rows = rows,
				max_rows = rows + 6,
				w_full = true,
			},
			toggle(`profile-{which}-expand`, which),
		},
		ui.text { t(`{which}_hint`), mt = "$space.1", wrap = true, text_size = "$font.size.sm", color = "$text.tertiary" },
	}
end

local function tools_line(): string
	local n = #draft().tools_off
	return if n == 0 then t("all_tools_on") else t("tools_off", { n = n })
end

local function form(): Node
	local harness_v = ui.use_state(draft().harness)
	local model_v = ui.use_state(draft().model or "")
	local mode_v = ui.use_state(draft().access or "")
	local effort_v = ui.use_state(draft().effort or "")
	ui.effect(function()
		local d = draft()
		ui.batch(function()
			if harness_v:peek() ~= d.harness then
				harness_v:set(d.harness)
			end
			if model_v:peek() ~= (d.model or "") then
				model_v:set(d.model or "")
			end
			if mode_v:peek() ~= (d.access or "") then
				mode_v:set(d.access or "")
			end
			if effort_v:peek() ~= (d.effort or "") then
				effort_v:set(d.effort or "")
			end
		end)
	end)
	local keep = { value = "", label = t("keep_current") }
	local function two(a: Node, b: Node?): Node
		return ui.row { w_full = true, gap = "$space.3", ui.column { grow = 1, basis = 0, min_w = 0, a }, ui.column { grow = 1, basis = 0, min_w = 0, b } }
	end
	local function field(label: string, control: Node): Node
		return ui.column { w_full = true, label_of(label), control }
	end
	local harness_select = ds.Select {
		value = harness_v,
		name = t("agent"),
		w_full = true,
		options = function(): { any }
			local out = {}
			for _, a in agents() do
				table.insert(out, { value = a.id, label = a.label })
			end
			return out
		end,
		on_change = function(h: string)
			change(function(d)
				if d.harness ~= h then
					d.harness = h
					d.model = nil
					d.options = {}
				end
			end)
		end,
	}
	local model_select = ds.Select {
		value = model_v,
		name = t("model"),
		w_full = true,
		options = function(): { any }
			local out = { keep }
			for _, m in models_of(draft().harness) do
				table.insert(out, { value = bare(m.value), label = m.label })
			end
			return out
		end,
		on_change = function(id: string)
			change(function(d)
				d.model = if id == "" then nil else id
				local m = model_of(d)
				local kept: { [string]: string } = {}
				for _, o in (if m then m.options else nil) or {} do
					for _, c in o.choices do
						if c.id == d.options[o.id] then
							kept[o.id] = c.id
						end
					end
				end
				d.options = kept
			end)
		end,
	}
	local mode_select = ds.Select {
		value = mode_v,
		name = t("mode"),
		w_full = true,
		options = function(): { any }
			local out = { keep }
			for _, m in look.modes(agent_of(draft().harness)) do
				table.insert(out, { value = m, label = look.access_word(m) or m, description = ui.t(`ds.composer.access.{m}_hint`):get() })
			end
			return out
		end,
		on_change = function(m: string)
			change(function(d)
				d.access = if m == "" then nil else m
			end)
		end,
	}
	local effort_select = ds.Select {
		value = effort_v,
		name = t("effort"),
		w_full = true,
		options = function(): { any }
			local out = { keep }
			local d = draft()
			local m = model_of(d)
			local levels = if m and m.efforts then m.efforts else nil
			if not levels then
				for _, x in models_of(d.harness) do
					if x.efforts then
						levels = x.efforts
						break
					end
				end
			end
			for _, l in levels or {} do
				table.insert(out, { value = l.value, label = l.label })
			end
			return out
		end,
		on_change = function(l: string)
			change(function(d)
				d.effort = if l == "" then nil else l
			end)
		end,
	}
	local option_rows = ui.presence(function(): string
		local d = draft()
		return `{d.harness}/{d.model or ""}`
	end, function(_key: string): Node
		local m = model_of(draft())
		local fields: { Node } = {}
		for _, o in (if m then m.options else nil) or {} do
			local v = ui.use_state(draft().options[o.id] or "")
			table.insert(fields, field(o.label, ds.Select {
				value = v,
				name = o.label,
				w_full = true,
				options = (function(): { any }
					local out = { keep }
					for _, c in o.choices do
						table.insert(out, { value = c.id, label = c.label })
					end
					return out
				end)(),
				on_change = function(choice: string)
					change(function(d)
						d.options[o.id] = if choice == "" then nil else choice
					end)
				end,
			}))
		end
		local rows: { Node } = {}
		for i = 1, #fields, 2 do
			table.insert(rows, two(fields[i], fields[i + 1]))
		end
		return ui.column { w_full = true, table.unpack(rows) }
	end, { w_full = true })

	local unfolded = ui.column {
		w_full = true,
		field(t("name"), ds.TextField {
			value = name,
			name = t("name"),
			placeholder = t("name_placeholder"),
			w_full = true,
			autofocus = true,
			on_key = function(e: any): boolean
				if e.key == "Enter" then
					save()
					return true
				end
				return false
			end,
		}),
		two(field(t("agent"), harness_select), field(t("model"), model_select)),
		two(field(t("mode"), mode_select), field(t("effort"), effort_select)),
		option_rows,
		field(t("tools"), folded_row("profile-tools-folded", tools_line, function()
			expanded:set("tools")
			load_tools()
		end)),
		long_field("instructions", instructions, look.INSTRUCTIONS_MAX, 5),
		long_field("notes", notes, look.NOTES_MAX, 3),
	}
	-- Opened out, the field takes the keys (Escape there folds it back).
	local opened, opened_tools = ui.ref(), ui.ref()
	ui.effect(function()
		local which = expanded:get()
		if which ~= nil then
			ui.after(0, function()
				local el = if which == "tools" then opened_tools:peek() else opened:peek()
				if el then
					el:focus()
				end
			end)
		end
	end)
	local function folded(): Node
		return ui.column {
			w_full = true,
			folded_row("profile-folded", function(): string
				local d = table.clone(draft())
				d.name = name:get()
				local a = agent_of(d.harness)
				local m = model_of(d)
				return look.draft_line(d, if a then a.label else nil, if m then m.label else nil)
			end, function()
				expanded:set(nil)
			end),
			ui.show(function(): boolean
				return expanded:get() ~= "instructions"
			end, folded_row("profile-folded-instructions", function(): string
				return t("instructions")
			end, function()
				expanded:set("instructions")
			end)),
			ui.show(function(): boolean
				return expanded:get() ~= "notes"
			end, folded_row("profile-folded-notes", function(): string
				return t("notes")
			end, function()
				expanded:set("notes")
			end)),
			ui.show(function(): boolean
				return expanded:get() ~= "tools"
			end, folded_row("profile-folded-tools", function(): string
				return `{t("tools")} · {tools_line()}`
			end, function()
				expanded:set("tools")
				load_tools()
			end)),
			ui.presence(function(): string
				return expanded:get() or ""
			end, function(which: string): Node
				if which == "instructions" then
					return ui.column { ref = opened, w_full = true, long_field("instructions", instructions, look.INSTRUCTIONS_MAX, 12) }
				elseif which == "notes" then
					return ui.column { ref = opened, w_full = true, long_field("notes", notes, look.NOTES_MAX, 12) }
				end
				return ui.div {}
			end, { w_full = true }),
			ui.show(function(): boolean
				return expanded:get() == "tools"
			end, ui.column {
				ref = opened_tools,
				w_full = true,
				ui.div { w_full = true, relative = true, label_of(function(): string
					return t("tools")
				end), toggle("profile-tools-collapse", "tools") },
				tools_list(),
			}),
		}
	end
	return ui.column {
		id = "agent-profile-card",
		w_full = true,
		-- Escape folds a field opened out before it closes the editor.
		on_key = function(e: any): boolean
			if e.key == "Escape" and expanded:peek() ~= nil then
				expanded:set(nil)
				return true
			end
			return false
		end,
		ui.row {
			w_full = true,
			gap = "$zeron.space.snug",
			items = "center",
			ui.presence(function(): string
				return draft().harness
			end, function(h: string): Node
				local a = agent_of(h) or { id = h, label = h }
				return look.badge({ harness = h, mark = a.mark, icon = a.icon, harness_label = a.label }, 24)
			end, { shrink = 0 }),
			ui.text {
				function(): string
					local a = agent_of(draft().harness)
					return if a then a.label else draft().harness
				end,
				text_size = "$font.size.md",
				color = "$text.secondary",
			},
		},
		ui.show(function(): boolean
			return expanded:get() == nil
		end, unfolded),
		ui.show(function(): boolean
			return expanded:get() ~= nil
		end, folded()),
		ui.show(function(): boolean
			local e = editor:get()
			return e ~= nil and e.error ~= nil
		end, ui.text {
			function(): string
				local e = editor:get()
				return if e and e.error then e.error else ""
			end,
			id = "profile-editor-error",
			mt = "$zeron.space.snug",
			wrap = true,
			text_size = "$font.size.sm",
			color = "$danger",
		}),
		ui.row {
			w_full = true,
			mt = "$space.4",
			gap = "$space.2",
			items = "center",
			ui.show(function(): boolean
				local e = editor:get()
				return e ~= nil and e.id ~= nil
			end, ds.Button {
				id = "profile-remove",
				tone = "ghost",
				size = "sm",
				on_click = remove,
				ui.text {
					function(): string
						local e = editor:get()
						return if e and e.confirm then t("confirm_remove") else t("remove")
					end,
					color = "$danger",
				},
			}),
			ui.div { grow = 1 },
			ds.Button {
				id = "profile-cancel",
				tone = "ghost",
				size = "sm",
				on_click = function()
					editor_open:set(false)
				end,
				ui.text { t("cancel") },
			},
			ds.Button {
				id = "profile-save",
				tone = "primary",
				size = "sm",
				on_click = save,
				ui.text {
					function(): string
						local e = editor:get()
						return if e and e.saving then t("saving") else t("save")
					end,
				},
			},
		},
	}
end

local function profile_row(item: look.Profile, i: number, now: State<any>): Node
	local function p(): look.Profile
		return now:get() :: any
	end
	local function count(): number
		return #profiles.list:get().profiles
	end
	local function index(): number
		for n, x in profiles.list:get().profiles do
			if x.id == p().id then
				return n
			end
		end
		return i
	end
	return ds.Settings.Row {
		lead = ui.presence(function(): string
			local x = p()
			return `{x.harness}\0{x.mark or ""}\0{x.icon or ""}`
		end, function(_key: string): Node
			return look.badge(p(), 30)
		end, { shrink = 0 }),
		label = function(): string
			return p().name
		end,
		badge = ui.show(function(): boolean
			return p().by_agent == true
		end, ds.Badge { t("by_agent"), tone = "neutral", size = "sm" }),
		description = function(): string
			local x = p()
			local line = look.line(x)
			return if x.notes and x.notes ~= "" then `{line}\n{x.notes}` else line
		end,
		control = ui.row {
			items = "center",
			gap = "$zeron.space.hair",
			IconButton {
				id = `profile-up-{item.id}`,
				icon = "alt-arrow-up",
				size = "md",
				tone = "quiet",
				label = ui.t("settings.agent_profiles.move_up", { name = item.name }),
				opacity = function(): number
					return if index() > 1 then 1 else 0.35
				end,
				on_click = function()
					local at = index()
					if at > 1 then
						M.move(at, at - 1)
					end
				end,
			},
			IconButton {
				id = `profile-down-{item.id}`,
				icon = "alt-arrow-down",
				size = "md",
				tone = "quiet",
				label = ui.t("settings.agent_profiles.move_down", { name = item.name }),
				opacity = function(): number
					return if index() < count() then 1 else 0.35
				end,
				on_click = function()
					local at = index()
					if at < count() then
						M.move(at, at + 1)
					end
				end,
			},
			ds.Button {
				id = `profile-edit-{item.id}`,
				tone = "secondary",
				size = "sm",
				ml = "$zeron.space.tight",
				on_click = function()
					M.open_editor(p().id, look.draft_of(p()))
				end,
				ui.text { t("edit") },
			},
		},
	}
end

local listed = ui.computed(function(): { look.Profile }
	return profiles.list:get().profiles
end)

function M.view(): Node
	-- A visit asks again (an agent may have saved one since), and reads the
	-- host's prompt; a draft handed over from the picker opens.
	profiles.load()
	local asked_prompt = host.system_prompt()
	if asked_prompt then
		asked_prompt:after(function(v: any)
			local text = if type(v) == "string" then v else ""
			prompt:set(text)
			prompt_text:set(text)
		end, function() end)
	end
	ui.effect(function()
		local p = profiles.pending:get()
		if p then
			profiles.pending:set(nil)
			M.open_editor(p.id, p.draft)
		end
	end)
	local function default_harness(): string
		local first = agents()[1]
		return if first then first.id else "claude-code"
	end
	return ds.Settings.Page {
		title = ui.t("settings.agent_profiles.title"),
		description = ui.t("settings.agent_profiles.description"),
		trail = search.anchor("new_profile", ui.row { ui.show(function(): boolean
			return not profiles.absent:get()
		end, ds.Button {
			id = "profile-new",
			tone = "primary",
			size = "sm",
			on_click = function()
				M.open_editor(nil, look.blank(default_harness()))
			end,
			ui.text { t("new") },
		}) }),
		ui.show(function(): boolean
			return page_error:get() ~= nil
		end, ui.button {
			id = "agent-profiles-error",
			label = function(): string
				return page_error:get() or ""
			end,
			w_full = true,
			p = "$space.2",
			rounded = "$radius.md",
			bg = "$zeron.tone.danger.wash",
			on_click = function()
				page_error:set(nil)
			end,
			ui.text {
				function(): string
					return page_error:get() or ""
				end,
				wrap = true,
				text_size = "$font.size.sm",
				color = "$danger",
			},
		}),
		ds.Settings.Section {
			ui.presence(function(): string
				local l = profiles.list:get()
				if l.state == "error" then
					return "error"
				elseif #l.profiles == 0 then
					if l.state == "ready" then
						return "empty"
					end
					return if profiles.absent:get() then "absent" else "loading"
				end
				return "list"
			end, function(state: string): Node
				if state == "loading" then
					return ui.text { t("loading"), id = "agent-profiles-loading", text_size = "$font.size.md", color = "$text.tertiary" }
				elseif state == "absent" then
					return ui.text { t("unavailable_host"), id = "agent-profiles-absent", wrap = true, text_size = "$font.size.md", color = "$text.tertiary" }
				elseif state == "error" then
					return ui.text {
						function(): string
							return profiles.list:get().error or ""
						end,
						id = "agent-profiles-list-error",
						wrap = true,
						text_size = "$font.size.md",
						color = "$danger",
					}
				elseif state == "empty" then
					return ds.Settings.Group {
						ds.Settings.Row { label = t("empty"), id = "agent-profiles-empty" },
					}
				end
				return ds.Settings.Each(listed, profile_row, {
					key = function(p: look.Profile): string
						return p.id
					end,
				})
			end, { w_full = true }),
		},
		search.anchor("system_prompt", ds.Settings.Section {
			title = ui.t("settings.agent_profiles.system_prompt"),
			description = ui.t("settings.agent_profiles.system_prompt_hint"),
			ds.Settings.Group {
				ds.Settings.Row {
					label = t("system_prompt"),
					stacked = true,
					control = ui.column {
						w_full = true,
						gap = "$space.2",
						ds.TextArea {
							value = prompt_text,
							name = t("system_prompt"),
							placeholder = t("system_prompt_placeholder"),
							rows = 3,
							max_rows = 10,
							w_full = true,
						},
						ui.row {
							w_full = true,
							justify = "end",
							ds.Button {
								id = "profile-system-prompt-save",
								tone = "secondary",
								size = "sm",
								disabled = function(): boolean
									return prompt_saving:get() or prompt:get() == nil or trimmed(prompt_text:get()) == trimmed(prompt:get() or "")
								end,
								on_click = function()
									local p = host.set_system_prompt({ prompt = prompt_text:peek() })
									if not p then
										return
									end
									prompt_saving:set(true)
									p:after(function(v: any)
										prompt_saving:set(false)
										local text = if type(v) == "string" then v else ""
										prompt:set(text)
										prompt_text:set(text)
									end, function(err: any)
										prompt_saving:set(false)
										page_error:set(t("couldnt_save", { error = message_of(err) }))
									end)
								end,
								ui.text { t("save") },
							},
						},
					},
				},
			},
		}),
		ds.Dialog {
			open = editor_open,
			title = function(): string
				local e = editor:get()
				return if e and e.id then t("edit_title") else t("new_title")
			end,
			cancel = false,
			size = "lg",
			build = form,
		},
	}
end

return M
