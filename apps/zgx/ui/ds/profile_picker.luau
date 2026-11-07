-- The profiles card, as Zeron's model picker shows it with its profiles
-- alone (crates/ui/src/pickers/profiles.rs): a search over the profiles'
-- names and notes, "No profile" first, each profile with its brand's badge,
-- its line, Active or Edited and the drift line, Revert and Save on an edited
-- one, "switch" on another agent's, a pencil under the pointer; then "Save
-- current as profile". Up, Down and Enter walk and apply them.
--
-- The pencil, and Save current, open the edit page in the card: its name,
-- agent, model, mode, effort, the model's options in pairs, when to use it
-- (opened out over the rest, with its token count), Use current selection,
-- More… (the draft handed to Settings → Agent profiles) and Save. Each
-- chooser is a page of its own; Escape steps out of the chooser, then the
-- field opened out, then the page.
--
-- `ProfilePicker { list = …, applied = …, apply = …, trigger = … }`: a
-- Dropdown underneath, its trigger the caller's (the model chip's two
-- halves), so it hangs where the model picker does.

local Dropdown = require("@self/dropdown")
local Icon = require("@self/icon")
local Menu = require("@self/menu")
local AgentMark = require("@self/agent_mark")
local Button = require("@self/button")
local TextField = require("@self/text_field")
local TextArea = require("@self/text_area")
local look = require("@self/profile_look")
local locale = require("@self/locale")

export type Agent = { id: string, label: string, mark: string?, icon: string?, access_modes: { string }? }
export type Props = {
	-- The list as the host has it, the chat's profile and how it stands
	-- against it (nil: none applied), and whether the chat's agent can be
	-- saved as a profile (not a generic ACP agent, as Zeron's).
	list: () -> look.List,
	applied: () -> look.Applied?,
	can_save: () -> boolean,
	-- The chat's agent once it has one: another agent's rows say "switch".
	switching: (() -> string?)?,
	-- The catalog's agents and models, for the edit page's choosers.
	agents: () -> { Agent },
	models: () -> { any },
	-- Applied (nil: "No profile"), Revert's too; Save writes the chat's
	-- drift back into its profile; the chat's picks as a draft.
	apply: (id: string?) -> (),
	save_drift: (id: string) -> (),
	current: () -> look.Draft,
	-- The edit page's Save (a promise: rejected with the engine's words),
	-- and More… (the draft to Settings).
	save: (id: string?, draft: look.Draft) -> any,
	more: (id: string?, draft: look.Draft) -> (),
	on_open: (() -> ())?,
	trigger: (menu: Dropdown.Menu) -> Node,
	open_with: State<number>?,
	close_with: State<number>?,
	showing: State<boolean>?,
	outside: State<boolean>?,
	placement: { side: string?, align: string?, offset: number? }?,
	bottom_up: boolean?,
}

-- The card's width (the full model picker's) and a row's inset from it.
local WIDTH = 304
local CARD_INSET = 4

local function t(key: string, args: { [string]: any }?): string
	return locale.s(`ds.profiles.{key}`, args)
end

-- A model's bare id from its catalog value ("<agent>/<id>").
local function bare(value: string): string
	return string.match(value, "^[^/]+/(.+)$") or value
end

type Edit = { id: string?, draft: look.Draft, error: string?, saving: boolean }

return ui.component(function(props: Props): Node
	local query = ui.use_state("")
	local hi = ui.use_state(nil :: string?)
	local value = ui.use_state(nil :: string?)
	local shut = ui.use_state(0)
	local edit = ui.use_state(nil :: Edit?)
	local choosing = ui.use_state(nil :: string?)
	local expanded = ui.use_state(false)
	local name = ui.use_state("")
	local notes = ui.use_state("")
	local name_ref = ui.ref()

	-- (The caller's close too.)
	if props.close_with then
		local asked = props.close_with
		local seen = asked:peek()
		ui.effect(function()
			local n = asked:get()
			if n ~= seen then
				seen = n
				shut:set(shut:peek() + 1)
			end
		end)
	end
	local function close()
		shut:set(shut:peek() + 1)
	end
	-- The row of the profile applied, highlighted as the card opens.
	ui.effect(function()
		local a = props.applied()
		local now = if a then `p:{a.id}` else "none"
		if value:peek() ~= now then
			value:set(now)
		end
	end)

	local function profile_of(id: string): look.Profile?
		for _, p in props.list().profiles do
			if p.id == id then
				return p
			end
		end
		return nil
	end
	local function applied_to(id: string): look.Applied?
		local a = props.applied()
		return if a and a.id == id then a else nil
	end

	local function open_edit(id: string?, draft: look.Draft)
		ui.batch(function()
			name:set(draft.name or "")
			notes:set(draft.notes or "")
			choosing:set(nil)
			expanded:set(false)
			edit:set({ id = id, draft = draft, saving = false })
		end)
		-- The name takes the keys, once the list has handed them on.
		ui.after(0, function()
			ui.after(0, function()
				local el = name_ref:peek()
				if el and edit:peek() and not choosing:peek() then
					el:focus()
				end
			end)
		end)
	end
	local function change(f: (look.Draft) -> ())
		local e = edit:peek()
		if not e then
			return
		end
		local d = table.clone(e.draft)
		d.options = table.clone(d.options or {})
		f(d)
		edit:set({ id = e.id, draft = d, saving = e.saving })
		choosing:set(nil)
	end
	-- What's typed, with the draft.
	local function typed(): look.Draft?
		local e = edit:peek()
		if not e then
			return nil
		end
		local d = table.clone(e.draft)
		d.name = string.match(name:peek(), "^%s*(.-)%s*$") or ""
		d.notes = string.match(notes:peek(), "^%s*(.-)%s*$") or ""
		return d
	end
	local function save_edit()
		local e = edit:peek()
		local d = typed()
		if not e or not d or e.saving then
			return
		end
		if d.name == "" then
			edit:set({ id = e.id, draft = e.draft, error = t("name_needed"), saving = false })
			return
		end
		edit:set({ id = e.id, draft = e.draft, saving = true })
		local p = props.save(e.id, d)
		if not p then
			edit:set({ id = e.id, draft = e.draft, saving = false, error = t("save_failed") })
			return
		end
		p:after(function()
			edit:set(nil)
		end, function(err: any)
			local now = edit:peek()
			if now then
				local said = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
				edit:set({ id = now.id, draft = now.draft, saving = false, error = said })
			end
		end)
	end

	local function activate(v: string)
		if v == "none" then
			props.apply(nil)
			close()
		elseif v == "save" then
			if props.can_save() then
				open_edit(nil, props.current())
			end
		elseif string.sub(v, 1, 2) == "p:" then
			props.apply(string.sub(v, 3))
			close()
		end
	end

	-- The rows: a note (unavailable, or nothing found), No profile (with
	-- profiles and nothing searched), the profiles found, Save current.
	local function items(): { Dropdown.Item }
		local l = props.list()
		local q = query:get()
		local out: { Dropdown.Item } = {}
		local found: { look.Profile } = {}
		for _, p in l.profiles do
			if look.matches(p, q) then
				table.insert(found, p)
			end
		end
		if l.state == "error" then
			table.insert(out, { value = "note", label = t("unavailable", { error = l.error or "" }), disabled = true })
		elseif #found == 0 and string.match(q, "%S") then
			table.insert(out, { value = "note", label = t("not_found"), disabled = true })
		end
		if #l.profiles > 0 and not string.match(q, "%S") then
			table.insert(out, { value = "none", label = t("none") })
		end
		for _, p in found do
			table.insert(out, { value = `p:{p.id}`, label = p.name })
		end
		table.insert(out, { value = "save", label = t("save_current"), disabled = not props.can_save() })
		return out
	end

	local function tag(label: () -> string, ink: string, wash: string): Node
		return ui.text {
			label,
			shrink = 0,
			px = ui.raw(5, "gpui's measure"),
			rounded = ui.raw(4, "gpui's corner"),
			bg = wash,
			text_size = "$font.size.xs",
			weight = "$font.weight.medium",
			color = ink,
		}
	end
	local function small_button(id: string, label: string, on_click: () -> ()): Node
		return ui.button {
			id = id,
			label = label,
			shrink = 0,
			px = ui.raw(7, "gpui's measure"),
			py = "$zeron.space.hair",
			rounded = ui.raw(6, "gpui's corner"),
			border = 1,
			border_color = "$border",
			hover = { bg = "$hover" },
			text_size = "$font.size.sm",
			color = "$text.primary",
			on_click = on_click,
			ui.text { label },
		}
	end
	local function tile(icon: Icon.Name): Node
		return ui.column {
			w = 24,
			h = 24,
			shrink = 0,
			rounded = ui.raw(7, "gpui's corner"),
			bg = "$zeron.secondary.bg",
			items = "center",
			justify = "center",
			color = "$text.tertiary",
			Icon { name = icon, size = 13 },
		}
	end
	local function row_shell(v: string, label: string, children: { Child }, extra: { [string]: any }?): Node
		local p: { [any]: any } = {
			role = "option",
			label = label,
			w_full = true,
			relative = true,
			px = "$space.2",
			py = ui.raw(5, "gpui's measure"),
			gap = "$zeron.space.snug",
			items = "center",
			rounded = Menu.row_radius(),
			bg = function(): string
				if hi:get() == v then
					return "$hover"
				end
				return if value:get() == v and v ~= "none" then "$zeron.popup.selected" else "#00000000"
			end,
			on_hover = function(inside: boolean)
				if inside then
					hi:set(v)
				end
			end,
			on_click = function()
				activate(v)
			end,
		}
		for k, x in extra or {} do
			p[k] = x
		end
		for _, c in children do
			table.insert(p, c)
		end
		return ui.row(p)
	end

	local GONE: look.Profile = { id = "", name = "", harness = "" }
	local function profile_row(id: string): Node
		local v = `p:{id}`
		local function p(): look.Profile
			return profile_of(id) or GONE
		end
		local function applied(): look.Applied?
			return applied_to(id)
		end
		local function edited(): boolean
			local a = applied()
			return a ~= nil and a.edited
		end
		return row_shell(v, p().name, {
			-- The applied row's bar, at the card's edge.
			ui.show(function(): boolean
				return applied() ~= nil
			end, ui.div {
				absolute = true,
				left = -CARD_INSET,
				top = 8,
				bottom = 8,
				w = 3,
				rounded = ui.raw(2, "the applied row's bar: its outer corners round"),
				bg = "$accent",
			}),
			ui.presence(function(): string
				local x = p()
				return `{x.harness}\0{x.mark or ""}\0{x.icon or ""}`
			end, function(_key: string): Node
				return look.badge(p(), 24)
			end, { shrink = 0 }),
			ui.column {
				grow = 1,
				min_w = 0,
				gap = ui.raw(1, "gpui's measure"),
				ui.row {
					w_full = true,
					items = "center",
					gap = "$zeron.space.tight",
					ui.text {
						function(): string
							return p().name
						end,
						min_w = 0,
						truncate = true,
						text_size = "$font.size.md",
						weight = "$font.weight.medium",
						color = "$text.primary",
					},
					ui.show(function(): boolean
						return applied() ~= nil and not edited()
					end, tag(function(): string
						return t("active")
					end, "$accent", "$zeron.tone.accent.wash")),
					ui.show(edited, tag(function(): string
						return t("edited")
					end, "$warning", "$zeron.tone.warning.wash")),
					ui.show(function(): boolean
						return p().by_agent == true
					end, tag(function(): string
						return t("by_agent")
					end, "$text.tertiary", "$zeron.secondary.bg")),
					ui.show(edited, ui.row {
						grow = 1,
						justify = "end",
						gap = "$space.1",
						-- (Re-applied, as Zeron's: the card closes.)
						small_button(`profile-revert-{id}`, t("revert"), function()
							props.apply(id)
							close()
						end),
						ui.show(function(): boolean
							local a = applied()
							return a ~= nil and a.savable and props.can_save()
						end, small_button(`profile-save-{id}`, t("save"), function()
							props.save_drift(id)
						end)),
					}),
				},
				ui.text {
					function(): string
						return look.line(p())
					end,
					truncate = true,
					text_size = "$font.size.sm",
					color = "$text.tertiary",
				},
				ui.show(edited, ui.text {
					function(): string
						local a = applied()
						return if a then look.drift_line(a.drift) else ""
					end,
					class = "profile-drift",
					wrap = true,
					text_size = "$font.size.sm",
					color = "$warning",
				}),
			},
			ui.show(function(): boolean
				local a = applied()
				return a ~= nil and not a.edited
			end, ui.div { shrink = 0, color = "$accent", Icon { name = "check", size = 14 } }),
			ui.show(function(): boolean
				local s = if props.switching then props.switching() else nil
				return applied() == nil and s ~= nil and s ~= p().harness
			end, ui.text { t("switch"), shrink = 0, text_size = "$font.size.sm", color = "$text.tertiary" }),
			ui.show(function(): boolean
				return hi:get() == v
			end, ui.button {
				id = `profile-quick-edit-{id}`,
				label = function(): string
					return t("edit_named", { name = p().name })
				end,
				w = 24,
				h = 24,
				shrink = 0,
				rounded = ui.raw(6, "gpui's corner"),
				items = "center",
				justify = "center",
				hover = { bg = "$hover" },
				color = "$text.tertiary",
				on_click = function()
					local x = profile_of(id)
					if x then
						open_edit(id, look.draft_of(x))
					end
				end,
				Icon { name = "pen", size = 13 },
			}),
		})
	end

	local function row_of(item: Dropdown.Item): Node
		local v = item.value
		if v == "note" then
			return ui.text {
				function(): string
					local l = props.list()
					return if l.state == "error" then t("unavailable", { error = l.error or "" }) else t("not_found")
				end,
				class = "profile-note",
				w_full = true,
				px = "$space.2",
				py = "$zeron.space.tight",
				wrap = true,
				text_size = "$font.size.sm",
				color = "$text.tertiary",
			}
		elseif v == "none" then
			return row_shell(v, item.label, {
				tile("lucide-circle-dashed"),
				ui.column {
					grow = 1,
					min_w = 0,
					gap = ui.raw(1, "gpui's measure"),
					ui.text { t("none"), text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
					ui.text { t("none_note"), truncate = true, text_size = "$font.size.sm", color = "$text.tertiary" },
				},
				ui.show(function(): boolean
					return props.applied() == nil
				end, ui.div { shrink = 0, color = "$accent", Icon { name = "check", size = 14 } }),
			})
		elseif v == "save" then
			local function label(): string
				return if props.can_save() then t("save_current") else t("no_acp")
			end
			return ui.column {
				w_full = true,
				gap = "$zeron.space.hair",
				ui.show(function(): boolean
					return #props.list().profiles > 0
				end, ui.div { w_full = true, h = 1, my = "$zeron.space.hair", bg = "$border" }),
				row_shell(v, label(), {
					tile("plus"),
					ui.text { label, text_size = "$font.size.md", color = "$text.tertiary" },
				}, {
					label = label,
					opacity = function(): any
						return if props.can_save() then 1 else ui.raw(0.5, "gpui's greyed footer row")
					end,
					cursor = function(): string
						return if props.can_save() then "pointer" else "default"
					end,
				}),
			}
		end
		return profile_row(string.sub(v, 3))
	end

	-- The edit page ---------------------------------------------------------

	local function draft(): look.Draft
		local e = edit:get()
		return if e then e.draft else look.blank("claude-code")
	end
	local function agent_of(id: string): Agent?
		for _, a in props.agents() do
			if a.id == id then
				return a
			end
		end
		return nil
	end
	local function models_of(h: string): { any }
		local out = {}
		for _, m in props.models() do
			if m.harness == h then
				table.insert(out, m)
			end
		end
		return out
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
	local function model_word(d: look.Draft): string
		if not d.model then
			return t("keep_current")
		end
		local m = model_of(d)
		return if m then m.label else d.model
	end
	local function levels(d: look.Draft): { any }
		local m = model_of(d)
		if m and m.efforts then
			return m.efforts
		end
		for _, x in models_of(d.harness) do
			if x.efforts then
				return x.efforts
			end
		end
		return {}
	end
	local function modes(d: look.Draft): { string }
		local a = agent_of(d.harness)
		return if a and a.access_modes and #a.access_modes > 0 then a.access_modes else { "supervised", "edits", "full" }
	end
	local function level_word(d: look.Draft): string
		if not d.effort then
			return t("keep_current")
		end
		for _, l in levels(d) do
			if l.value == d.effort then
				return l.label
			end
		end
		return d.effort
	end
	local function options_of(d: look.Draft): { any }
		local m = model_of(d)
		return if m and m.options then m.options else {}
	end

	-- A chooser's choices: its words, whether it's the draft's, and its mark.
	type Choice = { label: string, on: boolean, mark: any?, pick: () -> () }
	local function choices(field: string): { Choice }
		local d = draft()
		local out: { Choice } = {}
		if field == "harness" then
			for _, a in props.agents() do
				table.insert(out, {
					label = a.label,
					on = a.id == d.harness,
					mark = a,
					pick = function()
						change(function(x)
							if x.harness ~= a.id then
								x.harness = a.id
								x.model = nil
								x.options = {}
							end
						end)
					end,
				})
			end
			return out
		end
		local function keep(on: boolean, pick: () -> ()): Choice
			return { label = t("keep_current"), on = on, pick = pick }
		end
		if field == "model" then
			local a = agent_of(d.harness) or { id = d.harness, label = d.harness }
			table.insert(out, keep(d.model == nil, function()
				change(function(x)
					x.model = nil
					x.options = {}
				end)
			end))
			for _, m in models_of(d.harness) do
				table.insert(out, {
					label = m.label,
					on = d.model == bare(m.value),
					mark = a,
					pick = function()
						change(function(x)
							x.model = bare(m.value)
							-- (gpui's `options_for_model`: what the new model offers.)
							local kept: { [string]: string } = {}
							for _, o in m.options or {} do
								local was = x.options[o.id]
								for _, c in o.choices do
									if c.id == was then
										kept[o.id] = was
									end
								end
							end
							x.options = kept
						end)
					end,
				})
			end
		elseif field == "mode" then
			table.insert(out, keep(d.access == nil, function()
				change(function(x)
					x.access = nil
				end)
			end))
			for _, m in modes(d) do
				table.insert(out, {
					label = look.access_word(m) or m,
					on = d.access == m,
					pick = function()
						change(function(x)
							x.access = m
						end)
					end,
				})
			end
		elseif field == "effort" then
			table.insert(out, keep(d.effort == nil, function()
				change(function(x)
					x.effort = nil
				end)
			end))
			for _, l in levels(d) do
				table.insert(out, {
					label = l.label,
					on = d.effort == l.value,
					pick = function()
						change(function(x)
							x.effort = l.value
						end)
					end,
				})
			end
		else
			local id = string.match(field, "^option:(.+)$")
			for _, o in options_of(d) do
				if o.id == id then
					table.insert(out, keep(d.options[o.id] == nil, function()
						change(function(x)
							x.options[o.id] = nil
						end)
					end))
					for _, c in o.choices do
						table.insert(out, {
							label = c.label,
							on = d.options[o.id] == c.id,
							pick = function()
								change(function(x)
									x.options[o.id] = c.id
								end)
							end,
						})
					end
				end
			end
		end
		return out
	end

	local function mark_of(a: any?): Node?
		if not a then
			return nil
		end
		return AgentMark { label = a.label or a.id, mark = a.mark, icon = a.icon, seed = a.id, size = 13, bare = true }
	end
	local function label_of(text: string | () -> string): Node
		return ui.text { text, mb = ui.raw(3, "gpui's measure"), text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.tertiary" }
	end
	local function chooser(id: string, field: string, word: () -> string, mark: (() -> any?)?): Node
		return ui.button {
			id = id,
			label = word,
			w_full = true,
			h = 30,
			px = ui.raw(9, "gpui's measure"),
			gap = ui.raw(7, "gpui's measure"),
			items = "center",
			rounded = ui.raw(8, "gpui's corner"),
			border = 1,
			border_color = "$border",
			bg = "$zeron.field.bg",
			hover = { bg = "$hover" },
			on_click = function()
				choosing:set(field)
			end,
			if mark
				then ui.presence(function(): string
					local a = mark()
					return if a then a.id else ""
				end, function(key: string): Node
					return if key == "" then ui.div {} else (mark_of(agent_of(key) or { id = key, label = key }) :: Node)
				end, { shrink = 0 })
				else nil,
			ui.text { word, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.md", color = "$text.primary" },
			ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "alt-arrow-down", size = 11 } },
		}
	end
	local function field(label: string | () -> string, control: Node): Node
		return ui.column { grow = 1, basis = 0, min_w = 0, label_of(label), control }
	end
	local function two(a: Node, b: Node?): Node
		return ui.row { w_full = true, gap = "$space.2", a, b or ui.div { grow = 1, basis = 0 } }
	end

	local function header(): Node
		return ui.column {
			w_full = true,
			ui.row {
			w_full = true,
			h = 40,
			px = "$space.1",
			gap = "$space.2",
			items = "center",
			ui.button {
				id = "profile-edit-back",
				label = t("back"),
				w = 28,
				h = 28,
				shrink = 0,
				rounded = Menu.row_radius(),
				items = "center",
				justify = "center",
				hover = { bg = "$hover" },
				color = "$text.tertiary",
				on_click = function()
					if choosing:peek() then
						choosing:set(nil)
					else
						edit:set(nil)
					end
				end,
				Icon { name = "alt-arrow-left", size = 14 },
			},
			ui.presence(function(): string
				local d = draft()
				return d.harness
			end, function(h: string): Node
				local a = agent_of(h) or { id = h, label = h }
				return look.badge({ harness = h, mark = a.mark, icon = a.icon, harness_label = a.label }, 20)
			end, { shrink = 0 }),
			ui.text {
				function(): string
					local e = edit:get()
					if e and e.id then
						return t("edit_title", { name = e.draft.name })
					end
					return t("new_title")
				end,
				id = "profile-edit-title",
				min_w = 0,
				truncate = true,
				text_size = "$font.size.md",
				weight = "$font.weight.medium",
				color = "$text.primary",
			},
			},
			Menu.Rule(),
		}
	end

	local function choices_page(): Node
		return ui.presence(function(): string
			return choosing:get() or ""
		end, function(f: string): Node
			if f == "" then
				return ui.div {}
			end
			local list: { Node } = {}
			for _, c in choices(f) do
				table.insert(list, ui.button {
					label = c.label,
					role = "option",
					selected = c.on,
					w_full = true,
					px = "$space.2",
					py = "$zeron.space.tight",
					gap = "$space.2",
					items = "center",
					rounded = Menu.row_radius(),
					bg = if c.on then "$zeron.popup.selected" else "#00000000",
					hover = { bg = if c.on then "$zeron.popup.selected" else "$hover" },
					on_click = c.pick,
					mark_of(c.mark),
					ui.text { c.label, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.md", color = "$text.primary" },
				})
			end
			return ui.scroll {
				w_full = true,
				max_h = 320,
				ui.column { id = "profile-edit-choices", w_full = true, p = "$space.1", gap = "$zeron.space.hair", table.unpack(list) },
			}
		end, { w_full = true })
	end

	local function notes_field(): Node
		return ui.column {
			w_full = true,
			ui.row {
				w_full = true,
				label_of(function(): string
					return t("when_to_use")
				end),
				ui.div { grow = 1 },
				ui.show(expanded, ui.text {
					function(): string
						return look.token_count(notes:get(), look.NOTES_MAX)
					end,
					id = "profile-edit-notes-count",
					mb = ui.raw(3, "gpui's measure"),
					text_size = "$font.size.sm",
					color = "$text.tertiary",
				}),
			},
			ui.div {
				w_full = true,
				relative = true,
				-- (Opened out, it's a taller field over the same words.)
				ui.show(function(): boolean
					return not expanded:get()
				end, TextArea {
					value = notes,
					name = t("when_to_use"),
					placeholder = t("when_to_use_placeholder"),
					rows = 2,
					max_rows = 4,
					w_full = true,
				}),
				ui.show(expanded, TextArea {
					value = notes,
					name = t("when_to_use"),
					placeholder = t("when_to_use_placeholder"),
					rows = 10,
					max_rows = 14,
					w_full = true,
				}),
				ui.button {
					id = "profile-edit-notes-expand",
					label = function(): string
						return if expanded:get() then t("fold") else t("expand")
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
						expanded:set(not expanded:peek())
					end,
					Icon {
						name = function(): Icon.Name
							return if expanded:get() then "collapse-arrows" else "expand-arrows"
						end,
						size = 12,
					},
				},
			},
		}
	end

	local function form(): Node
		local option_rows = ui.presence(function(): string
			local d = draft()
			local ids = {}
			for _, o in options_of(d) do
				table.insert(ids, o.id)
			end
			return `{d.harness}/{d.model or ""}:{table.concat(ids, ",")}`
		end, function(_key: string): Node
			local fields: { Node } = {}
			for _, o in options_of(draft()) do
				table.insert(fields, field(o.label, chooser(`profile-edit-option-{o.id}`, `option:{o.id}`, function(): string
					local d = draft()
					local picked = d.options[o.id]
					for _, c in o.choices do
						if c.id == picked then
							return c.label
						end
					end
					return t("keep_current")
				end)))
			end
			local rows: { Node } = {}
			for i = 1, #fields, 2 do
				table.insert(rows, two(fields[i], fields[i + 1]))
			end
			return ui.column { w_full = true, gap = "$zeron.space.tight", table.unpack(rows) }
		end, { w_full = true })
		return ui.column {
			w_full = true,
			gap = "$zeron.space.tight",
			ui.show(function(): boolean
				return not expanded:get()
			end, ui.column {
				w_full = true,
				gap = "$zeron.space.tight",
				field(function(): string
					return t("name")
				end, TextField {
					value = name,
					name = t("name"),
					placeholder = t("name_placeholder"),
					w_full = true,
					input_ref = name_ref,
					on_key = function(e: any): boolean
						if e.key == "Enter" then
							save_edit()
							return true
						end
						return false
					end,
				}),
				two(
					field(function(): string
						return t("agent")
					end, chooser("profile-edit-harness", "harness", function(): string
						local d = draft()
						local a = agent_of(d.harness)
						return if a then a.label else d.harness
					end, function(): any?
						return agent_of(draft().harness) or { id = draft().harness }
					end)),
					field(function(): string
						return t("model")
					end, chooser("profile-edit-model", "model", function(): string
						return model_word(draft())
					end, function(): any?
						return agent_of(draft().harness) or { id = draft().harness }
					end))
				),
				two(
					field(function(): string
						return t("mode")
					end, chooser("profile-edit-mode", "mode", function(): string
						local d = draft()
						return look.access_word(d.access) or t("keep_current")
					end)),
					field(function(): string
						return t("effort")
					end, chooser("profile-edit-effort", "effort", function(): string
						return level_word(draft())
					end))
				),
				option_rows,
			}),
			-- Opened out: the rest folded into one row, a press folds it back.
			ui.show(expanded, ui.button {
				id = "profile-edit-folded",
				label = function(): string
					local d = table.clone(draft())
					d.name = name:get()
					local a = agent_of(d.harness)
					return look.draft_line(d, if a then a.label else nil, if d.model then model_word(d) else nil)
				end,
				w_full = true,
				h = 30,
				px = ui.raw(9, "gpui's measure"),
				gap = "$space.2",
				items = "center",
				rounded = ui.raw(8, "gpui's corner"),
				border = 1,
				border_color = "$border",
				hover = { bg = "$hover" },
				on_click = function()
					expanded:set(false)
				end,
				ui.text {
					function(): string
						local d = table.clone(draft())
						d.name = name:get()
						local a = agent_of(d.harness)
						return look.draft_line(d, if a then a.label else nil, if d.model then model_word(d) else nil)
					end,
					grow = 1,
					min_w = 0,
					truncate = true,
					text_size = "$font.size.sm",
					color = "$text.tertiary",
				},
				ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "alt-arrow-down", size = 11 } },
			}),
			notes_field(),
			ui.show(function(): boolean
				local e = edit:get()
				return e ~= nil and e.error ~= nil
			end, ui.text {
				function(): string
					local e = edit:get()
					return if e and e.error then e.error else ""
				end,
				id = "profile-edit-error",
				wrap = true,
				text_size = "$font.size.sm",
				color = "$danger",
			}),
		}
	end

	local function link(id: string, label: string, on_click: () -> ()): Node
		return ui.button {
			id = id,
			label = label,
			shrink = 0,
			text_size = "$font.size.sm",
			color = "$text.tertiary",
			hover = { color = "$text.primary" },
			on_click = on_click,
			ui.text { label },
		}
	end

	local page = ui.column {
		id = "profile-edit",
		w_full = true,
		header(),
		ui.show(function(): boolean
			return choosing:get() ~= nil
		end, choices_page()),
		ui.show(function(): boolean
			return choosing:get() == nil
		end, ui.column {
			w_full = true,
			ui.scroll { w_full = true, max_h = 420, ui.column { w_full = true, px = "$space.3", py = "$space.2", form() } },
			Menu.Rule(),
			ui.row {
				w_full = true,
				px = "$space.3",
				py = "$space.2",
				gap = "$zeron.space.snug",
				items = "center",
				link("profile-edit-use-current", t("use_current"), function()
					local c = props.current()
					change(function(x)
						x.harness = c.harness
						x.model = c.model
						x.effort = c.effort
						x.access = c.access
						x.options = table.clone(c.options or {})
						x.tools_off = table.clone(c.tools_off or {})
					end)
				end),
				ui.div { grow = 1 },
				link("profile-edit-more", t("more"), function()
					local e = edit:peek()
					local d = typed()
					if e and d then
						props.more(e.id, d)
						edit:set(nil)
						close()
					end
				end),
				Button {
					id = "profile-edit-save",
					tone = "primary",
					size = "sm",
					on_click = save_edit,
					ui.text {
						function(): string
							local e = edit:get()
							return if e and e.saving then t("saving") else t("save")
						end,
					},
				},
			},
		}),
	}

	return Dropdown {
		value = value,
		items = items,
		highlight = hi,
		query = query,
		rank = function(list: { Dropdown.Item }, _q: string): { Dropdown.Item }
			return list
		end,
		search = t("search"),
		-- (The card fits its rows, as Zeron's profiles card does, up to this.)
		view = 440,
		label = locale.t("ds.profiles.title") :: any,
		width = WIDTH,
		keep_open = true,
		take = function(v: string): boolean
			activate(v)
			return true
		end,
		wrap_item = function(item: Dropdown.Item, _row: Node): Node
			return row_of(item)
		end,
		list_when = function(): boolean
			return edit:get() == nil
		end,
		tab_inside = function(): boolean
			return edit:get() ~= nil
		end,
		head = page,
		head_when = function(): boolean
			return edit:get() ~= nil
		end,
		on_open = function()
			ui.batch(function()
				edit:set(nil)
				choosing:set(nil)
				expanded:set(false)
			end)
			if props.on_open then
				props.on_open()
			end
		end,
		on_key = function(e: any): boolean
			-- The edit page's own keys: its fields and buttons answer theirs.
			if edit:peek() ~= nil then
				return e.key == "ArrowUp" or e.key == "ArrowDown" or e.key == "Enter"
			end
			return false
		end,
		on_escape = function(): boolean
			if edit:peek() == nil then
				return false
			end
			if choosing:peek() then
				choosing:set(nil)
			elseif expanded:peek() then
				expanded:set(false)
			else
				edit:set(nil)
			end
			return true
		end,
		trigger = props.trigger,
		open_with = props.open_with,
		close_with = shut,
		showing = props.showing,
		outside = props.outside,
		placement = props.placement,
		bottom_up = props.bottom_up,
	}
end)
