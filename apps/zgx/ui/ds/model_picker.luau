-- Picking the model an agent runs, as Zeron's picker does (crates/ui/src/
-- pickers.rs and pickers/compact.rs). Two layouts, a toggle in the panel
-- between them, kept as a setting:
--
-- Compact (the default, 256 wide) opens on the chosen model's panel: its
-- reasoning's level over its name (a press on them goes to the list), Fast
-- mode's bolt, the layout toggle, the EffortSlider, then its options as rows.
-- The list is its own page: back and an icon a provider, each jumping the one
-- list of every provider to its group (starred first), then a search.
--
-- Full (304 wide) has a tab per agent over a search, that tab's rows, and the
-- panel's controls in a tray under them.
--
-- A dropdown underneath: its trigger, keys, search, list and panel.
--
-- `ModelPicker { model = state, effort = state, options = state, harnesses = { … }, models = { … } }`

local control = require("@self/control")
local Dropdown = require("@self/dropdown")
local Mark = require("@self/mark")
local AgentMark = require("@self/agent_mark")
local EffortSlider = require("@self/effort_slider")
local Switch = require("@self/switch")
local Icon = require("@self/icon")
local icon_button = require("@self/icon_button")
local IconButton = icon_button.IconButton
local Menu = require("@self/menu")
local FadeText = require("@self/fade_text")
local RollText = require("@self/roll_text")
local Tooltip = require("@self/tooltip")
local PrivateText = require("@self/private_text")
local motion = require("@self/motion")
local context_usage = require("@self/context_usage")
local locale = require("@self/locale")

-- An agent: a tab of its own.
-- (`icon`: an agent from the ACP registry's one-ink SVG, in its mark's
-- place. `state`: its models still coming, or what stopped them, an agent
-- with none yet still having its tab.)
export type Harness = {
	id: string,
	label: string,
	mark: Mark.Name,
	icon: string?,
	state: ("ready" | "loading" | "error")?,
	error: string?,
	auto: boolean?,
}
-- One of a model option's choices.
export type Choice = { id: string, label: string }
-- An option of a model (Devin Fusion's lead and sidekick): its choices and
-- the one it starts on (by id).
export type Option = { id: string, label: string, default: string, choices: { Choice } }
-- Where a model is served from, for an agent reaching models through
-- several providers (Pi signed in to Codex and OpenCode): its id and name.
export type Provider = { id: string, name: string }
-- A model: its agent, its reasoning levels (low to high) and the one it
-- starts on, whether it offers Fast mode, its other options, whether it's
-- its agent's default (what a chat that names no model runs), and its
-- provider.
export type Model = {
	value: string,
	label: string,
	harness: string,
	provider: Provider?,
	efforts: { EffortSlider.Level }?,
	effort: string?,
	fast: boolean?,
	options: { Option }?,
	description: string?,
	new: boolean?,
	default: boolean?,
}
-- An account of an agent: how much of its plan it has used (what the ring
-- and the row say; nil: nothing reported), its windows said in words (or why
-- not), whether it's the one in use, and whether it can be moved to.
export type Account = {
	id: string,
	email: string,
	plan: string?,
	used: number?,
	detail: string,
	active: boolean,
	can_switch: boolean,
	-- Its plan's windows, for the footer's plan chip and card.
	windows: { { label: string, used: number, resets: number? } }?,
}
-- The accounts a model runs on, where its agent keeps a login per model
-- provider (Pi, OpenCode, Hermes): who they're with ("ChatGPT"), and, where
-- Zeron can sign in to it, what adds one and what that does.
export type AccountGroup = { name: string, add: string?, on_add: (() -> ())? }
export type Props = {
	-- The accounts of a model's agent (nil or none: no row, no ring), followed
	-- as they change; for an agent with a login per model provider, its
	-- provider's alone (`provider`, the model's). `on_account` puts one in
	-- use; `on_accounts_open` asks after them again as the panel opens.
	accounts: ((harness: string, model: string, provider: string?) -> { Account }?)?,
	-- The model's provider's accounts, named (nil: the agent keeps one
	-- login): the row is "<name> account", and offers to add one, even with
	-- none yet.
	account_group: ((harness: string, provider: string?) -> AccountGroup?)?,
	on_account: ((harness: string, id: string) -> ())?,
	on_accounts_open: (() -> ())?,
	model: State<string?>,
	effort: State<string?>,
	fast: State<boolean>?,
	-- The choices made of the model's options (option id to choice id; one
	-- left out is at its default), changed as they're picked.
	options: State<{ [string]: string }>?,
	-- Its catalog is still on its way (a device's first look).
	loading: (() -> boolean)?,
	-- The chosen model's name while its catalog doesn't list it (a chat on
	-- another device, its catalog not looked at yet or short of it): what
	-- the chip says rather than nothing, as Zeron's `selected_model_label`.
	model_label: (() -> string?)?,
	-- What runs while no model is chosen (a chat naming none runs its agent's
	-- own): shown as the chosen one is, its mark and reasoning, as Zeron's
	-- `effective_model_id` falls back to the agent's default, but never
	-- written into `model` or `effort`, so it isn't sent as a pick.
	default_model: (() -> string?)?,
	-- The agents and their models: lists, or states or functions of them
	-- (a catalog that arrives or changes, followed in place: the tabs and
	-- rows come and go, the picker stays).
	harnesses: { Harness } | State<{ Harness }> | () -> { Harness },
	models: { Model } | State<{ Model }> | () -> { Model },
	tone: Dropdown.Tone?,
	label: string?,
	placement: { side: string?, align: string?, offset: number? }?,
	-- The panel read bottom up (opening above its trigger).
	bottom_up: boolean?,
	-- The models starred (by value), kept across launches unless given.
	favorites: State<{ [string]: boolean }>?,
	-- Bumped, it opens (a shortcut to it; Dropdown's `open_with`).
	open_with: State<number>?,
	close_with: State<number>?,
	-- Bumped, the effort moves `by` stops along the slider, held at its ends (Zeron's ⌘− ⌘=).
	step_with: State<{ n: number, by: number }>?,
	-- Set to whether it is open (Dropdown's `showing`).
	showing: State<boolean>?,
	-- Set as it closes: whether from outside it (Dropdown's `outside`).
	outside: State<boolean>?,
	-- How far its chip folds (a footer short of room): 1, its name shortened;
	-- 2, to its least. The effort, the bolt and the ring always stay.
	fold: (() -> number)?,
	-- This chat has its agent: another agent's tab only browses, and its models
	-- go through the switch. (Not given or false: a new chat, whose agent is
	-- whichever was picked last.)
	committed: (() -> boolean)?,
	-- A model picked (a dialog to ask first: true takes it, nothing changes).
	take: ((value: string) -> boolean)?,
	-- Whether the catalog is still coming, and asking for it again.
	looking: (() -> boolean)?,
	on_retry: (() -> ())?,
	-- What the chat holds of its context, for the warnings on a smaller window.
	context_tokens: (() -> number?)?,
	-- The chip's mark only while true (a profile's half carries it).
	mark_when: (() -> boolean)?,
	-- The full layout's Profiles tab: what it opens (the profiles card), and its
	-- dot while a profile is applied ("accent", or "warning" drifted).
	on_profiles: (() -> ())?,
	profile_dot: (() -> string?)?,
}

-- Devin Fusion's lead (what runs), listed first among its options.
local LEAD = "lead"
-- A choice list's width.
local CHOICES_W = 232
-- The chip's name at each fold: at its most (a chip's label), shortened as
-- the footer runs short, at its least. Past it, it ends in "…"; the effort
-- after it is always whole.
local NAME_AT: { [number]: number } = { [0] = 160, [1] = 72, [2] = 32 }
-- The two layouts' widths, and their rows.
local COMPACT_W, FULL_W = 256, 304
local COMPACT_ROW, FULL_ROW = 32, 30
local COMPACT_ROWS, FULL_ROWS = 7, 6
-- Option ids a model uses for an effort ladder (Cursor's carry no levels).
local EFFORT_OPTIONS = { effort = true, reasoning = true, reasoning_effort = true }
local CONTEXT_WINDOW, AUTO_COMPACT = "contextWindow", "autoCompact"
-- The CLI compacts this far below the chosen window.
local HEADROOM = 33000
-- The model's options after six rows scroll.
local OPTIONS_MAX = 6
-- The option rows' pitch.
local OPTION_PITCH = 30

-- A model's options, in order: the lead first (Zeron's `card_order`),
-- then the rest as the agent lists them.
local function ordered(m: Model): { Option }
	local out: { Option } = {}
	for _, o in m.options or {} do
		if o.id == LEAD then
			table.insert(out, o)
		end
	end
	for _, o in m.options or {} do
		if o.id ~= LEAD then
			table.insert(out, o)
		end
	end
	return out
end
-- The tab of everything starred, and the compact list's rail of everything.
local STARRED, ALL = "*starred", "*all"

-- The models starred (by value), kept across launches: one set every
-- picker shares (a star given in one shows in the rest), unless a picker
-- is given its own.
local FAVORITES = ui.persist("model_favorites", {} :: { [string]: boolean })
-- Compact or full (Zeron's `compact_model_picker`, compact by default).
local COMPACT = ui.persist("model_picker_compact", true)
-- The last model used on each agent, and the level last used on each model:
-- what a provider switched to with Tab opens on.
local LAST_MODEL = ui.persist("model_last_used", {} :: { [string]: string })
local LAST_EFFORT = ui.persist("model_last_effort", {} :: { [string]: string })
-- The full layout's tab last picked ("" none yet): it opens there again.
local LAST_TAB = ui.persist("model_picker_tab", "")

-- A list, however it was given.
local function read(given: any): any
	if type(given) == "function" then
		local call: () -> any = given
		return call()
	elseif type(given) == "userdata" then
		local state: State<any> = given
		return state:get()
	end
	return given or {}
end

-- Where a search finds `q` in `text`: its start, then anywhere in it.
local function match_rank(q: string, text: string): number?
	local at = string.find(string.lower(text), q, 1, true)
	if not at then
		return nil
	end
	return if at == 1 then 0 else 1
end

-- A provider found as the dropdown finds a hint: from one of its words' start
-- ("code" finds OpenCode, "pen" doesn't), or read on from the name into it.
local function provider_found(q: string, label: string, provider: string): boolean
	local low = string.lower(provider)
	local at = string.find(low, q, 1, true)
	while at do
		local before = if at > 1 then string.sub(provider, at - 1, at - 1) else " "
		if not string.match(before, "%w") or (string.match(before, "%l") and string.match(string.sub(provider, at, at), "%u")) then
			return true
		end
		at = string.find(low, q, at + 1, true)
	end
	local name = string.lower(label)
	local together = name .. " " .. low
	local from, to = string.find(together, q, 1, true)
	while from do
		if from <= #name and (to :: number) > #name + 1 then
			return true
		end
		from, to = string.find(together, q, from + 1, true)
	end
	return false
end
-- Tokens as gpui's: 212K, 1.0M.
local function short_tokens(n: number): string
	if n >= 999500 then
		return string.format("%.1fM", n / 1000000)
	end
	return `{math.floor((n + 500) / 1000)}K`
end

return ui.component(function(props: Props): Node
	local function harness_list(): { Harness }
		return read(props.harnesses) :: { Harness }
	end
	local by_id = ui.computed(function(): { [string]: Model }
		local out: { [string]: Model } = {}
		for _, m in read(props.models) :: { Model } do
			out[m.value] = m
		end
		return out
	end)
	local agents = ui.computed(function(): { [string]: Harness }
		local out: { [string]: Harness } = {}
		for _, h in harness_list() do
			out[h.id] = h
		end
		return out
	end)
	-- The model shown: the one chosen, else what runs without one.
	local function shown_id(): string?
		local v = props.model:get()
		if v == nil and props.default_model then
			return props.default_model()
		end
		return v
	end
	local function current(): Model?
		return by_id:get()[shown_id() or ""]
	end
	-- The level a model starts on.
	local function own_effort(m: Model): string?
		if not m.efforts or #m.efforts == 0 then
			return nil
		end
		return m.effort or m.efforts[math.ceil(#m.efforts / 2)].value
	end
	-- The effort shown: the one chosen, else the shown model's own.
	local function shown_effort(): string?
		local e = props.effort:get()
		if e == nil then
			local m = current()
			return if m then own_effort(m) else nil
		end
		return e
	end
	-- What the dropdown and the slider show, followed from the above. A pick
	-- reaches `model` and `effort` by `on_choose` and `on_change`, so the
	-- default shown is never written back as one.
	local shown_model = ui.use_state(shown_id())
	local shown_level = ui.use_state(shown_effort())
	ui.effect(function()
		local v = shown_id()
		if shown_model:peek() ~= v then
			shown_model:set(v)
		end
	end)
	ui.effect(function()
		local e = shown_effort()
		if shown_level:peek() ~= e then
			shown_level:set(e)
		end
	end)
	-- The chosen model's name when the catalog can't say it, or what runs
	-- when none is chosen (nil: the catalog says it, or no one can).
	local function named(): string?
		if current() ~= nil or not props.model_label then
			return nil
		end
		local said = props.model_label()
		return if said and said ~= "" then said else nil
	end
	local function committed(): boolean
		return props.committed ~= nil and props.committed()
	end
	-- Read bottom up (the tabs by the trigger, the list over them): see
	-- Dropdown's `bottom_up`.
	local bottom_up = props.bottom_up == true
	-- The choice made of a model's option: the pick, or the option's own
	-- start (a pick the option doesn't offer reads as that too).
	local function chosen(opt: Option): Choice
		local picks = if props.options then props.options:get() else {}
		local id = picks[opt.id] or opt.default
		for _, c in opt.choices do
			if c.id == id then
				return c
			end
		end
		return opt.choices[1] or { id = opt.default, label = "" }
	end
	-- A choice picked: kept by option id; the option's own start is no pick
	-- (Zeron leaves a default out of the chat's options).
	local function pick(opt: Option, choice: string)
		if not props.options then
			return
		end
		local now = table.clone(props.options:peek())
		now[opt.id] = if choice == opt.default then nil else choice
		props.options:set(now)
	end
	local favorites: State<{ [string]: boolean }> = props.favorites or FAVORITES
	local function starred(value: string): boolean
		return favorites:get()[value] == true
	end
	local function toggle_star(value: string)
		local now = table.clone(favorites:peek())
		now[value] = if now[value] then nil else true
		favorites:set(now)
	end

	-- The list's search, scroll and highlight (the tabs and keys move them).
	local query = ui.use_state("")
	local offset = ui.use_state(0)
	local highlight = ui.use_state(nil :: string?)
	-- Compact, the page: the panel, or the list.
	local listing = ui.use_state(false)
	local function compact(): boolean
		return COMPACT:get()
	end
	local function on_list(): boolean
		return not compact() or listing:get()
	end
	-- The full layout's tab: the one picked since the panel opened, else the
	-- one last picked while its agent is listed, else the chosen model's agent.
	local tab = ui.use_state(nil :: string?)
	local function viewed(): string
		local t = tab:get()
		if t then
			return t
		end
		local last = LAST_TAB:get()
		if last == STARRED or agents:get()[last] then
			return last
		end
		local m = current()
		if m then
			return m.harness
		end
		-- No model chosen: the first agent's tab; with none (a catalog
		-- still coming), the starred one.
		local first = harness_list()[1]
		return if first then first.id else STARRED
	end
	local function rail(): string
		return if compact() then ALL else viewed()
	end
	-- The slider's stops: the model's reasoning, else an option shaped like one
	-- (Cursor's), so every model with an effort has the same slider.
	local function effort_option(): Option?
		local m = current()
		if not m or (m.efforts and #m.efforts > 0) then
			return nil
		end
		for _, o in m.options or {} do
			if EFFORT_OPTIONS[o.id] and #o.choices > 1 then
				return o
			end
		end
		return nil
	end
	local function levels(): { EffortSlider.Level }
		local m = current()
		if m and m.efforts then
			return m.efforts
		end
		local o = effort_option()
		if o then
			local out: { EffortSlider.Level } = {}
			for _, c in o.choices do
				table.insert(out, { value = c.id, label = c.label })
			end
			return out
		end
		return {}
	end
	local dragged = ui.use_state(nil :: string?)
	local slider_value = ui.use_state(nil :: string?)
	ui.effect(function()
		local o = effort_option()
		local v = if o then chosen(o).id else props.effort:get()
		if slider_value:peek() ~= v then
			slider_value:set(v)
		end
	end)
	-- Remembered as a new chat picks them, as Zeron's `remember_model` and
	-- `remember_reasoning`: a chat only looked at teaches nothing.
	local function remember(level: boolean)
		local m = current()
		if not m or committed() then
			return
		end
		if not level and LAST_MODEL:peek()[m.harness] ~= m.value then
			local now = table.clone(LAST_MODEL:peek())
			now[m.harness] = m.value
			LAST_MODEL:set(now)
		end
		local e = props.effort:peek()
		if level and e and LAST_EFFORT:peek()[m.value] ~= e then
			local now = table.clone(LAST_EFFORT:peek())
			now[m.value] = e
			LAST_EFFORT:set(now)
		end
	end
	local function set_level(v: string)
		local o = effort_option()
		if o then
			pick(o, v)
		else
			props.effort:set(v)
			remember(true)
		end
	end
	-- The level shown: the one a drag is over, else the one set.
	local function level_label(): string?
		local v = dragged:get() or slider_value:get()
		for _, l in levels() do
			if l.value == v then
				return l.label
			end
		end
		return nil
	end

	-- A model moved to keeps the effort when it has it, else takes its own: the
	-- level last used on it, on a new chat.
	local function fit_effort()
		local m = current()
		if not m or not m.efforts or #m.efforts == 0 or props.model:get() == nil then
			return
		end
		local e = props.effort:peek()
		for _, l in m.efforts do
			if l.value == e then
				return
			end
		end
		props.effort:set(m.effort or m.efforts[1].value)
	end
	ui.effect(fit_effort)
	local stepping = false
	if props.step_with then
		local asked = props.step_with
		local seen = asked:peek().n
		ui.effect(function()
			local s = asked:get()
			if s.n == seen then
				return
			end
			seen = s.n
			-- (After the effect: the slider's reads would track.)
			ui.after(0, function()
				local ls = levels()
				local at = 0
				for i, l in ls do
					if l.value == slider_value:peek() then
						at = i
					end
				end
				local to = if at > 0 then ls[math.clamp(at + s.by, 1, #ls)] else nil
				if to and to.value ~= slider_value:peek() then
					stepping = true
					set_level(to.value)
					ui.after(0, function()
						stepping = false
					end)
				end
			end)
		end)
	end

	-- The models as the dropdown's items, each in its agent's mark. A model the
	-- chat runs that the catalog no longer lists has a row of its own, there to
	-- say so, that picks nothing.
	local items = ui.computed(function(): { Dropdown.Item }
		local out: { Dropdown.Item } = {}
		local by_agent = agents:get()
		local list = read(props.models) :: { Model }
		local v = props.model:get()
		local listed = v == nil
		for _, m in list do
			local agent = by_agent[m.harness]
			listed = listed or m.value == v
			local name = if agent then agent.label else m.harness
			table.insert(out, {
				value = m.value,
				label = m.label,
				mark = if agent then agent.mark else nil,
				svg = if agent then agent.icon else nil,
				svg_seed = m.harness,
				tip = if m.description and m.description ~= "" then `{m.label} · {name}\n{m.description}` else `{m.label} · {name}`,
				hint = if m.provider then m.provider.name else nil,
				-- (Always: where a model comes from is part of which it is.)
				hint_when_found = nil,
			})
		end
		if not listed and #list > 0 and v then
			local harness, id = string.match(v, "^([^/]+)/(.+)$")
			local agent = by_agent[harness or ""]
			table.insert(out, 1, {
				value = v,
				label = id or v,
				mark = if agent then agent.mark else nil,
				svg = if agent then agent.icon else nil,
				svg_seed = harness,
				inert = true,
				tip = locale.s("ds.model_picker.selected_only"),
			})
		end
		return out
	end)
	-- A row's words after its name: its description, when two rows share a name
	-- (opencode's providers); on the starred tab, its agent's.
	local same_name = ui.computed(function(): { [string]: boolean }
		local seen: { [string]: number } = {}
		local out: { [string]: boolean } = {}
		for _, m in read(props.models) :: { Model } do
			local viewing = if compact() then true else m.harness == viewed()
			if viewing then
				seen[m.label] = (seen[m.label] or 0) + 1
			end
		end
		for label, n in seen do
			if n > 1 then
				out[label] = true
			end
		end
		return out
	end)

	-- What shows, in what order, as Zeron's `scoped_model_rows`: the search never
	-- leaves the tab; it ranks a prefix of the name, then anywhere in it, then the
	-- description, stars and the catalog's order breaking ties (across every
	-- agent, stars first). Without one, a tab lists its catalog stars first, and
	-- the starred tab every star.
	local function rank(list: { Dropdown.Item }, raw: string): { Dropdown.Item }
		local q = string.lower((string.gsub(raw, "^%s*(.-)%s*$", "%1")))
		local by = by_id:peek()
		local where = rail()
		local stars = favorites:get()
		local function in_scope(item: Dropdown.Item): boolean
			if where == ALL then
				return not item.inert
			elseif where == STARRED then
				return stars[item.value] == true
			end
			if item.inert then
				return viewed() == string.match(item.value, "^([^/]+)/")
			end
			local m = by[item.value]
			return m ~= nil and m.harness == where
		end
		type Row = { item: Dropdown.Item, r: number, off: number, ix: number }
		local rows: { Row } = {}
		for ix, item in list do
			if in_scope(item) then
				local m = by[item.value]
				local r: number? = 0
				if q ~= "" then
					r = match_rank(q, item.label)
					if not r then
						local d = match_rank(q, `{m and m.description or ""} {item.label}`)
						r = if d then d + 2 else nil
						if m and m.provider and provider_found(q, item.label, m.provider.name) then
							r = 2
						end
					end
				end
				if r then
					table.insert(rows, { item = item, r = r, off = if stars[item.value] then 0 else 1, ix = ix })
				end
			end
		end
		local all = where == ALL
		table.sort(rows, function(a: Row, b: Row): boolean
			if q == "" then
				if a.off ~= b.off then
					return a.off < b.off
				end
				return a.ix < b.ix
			end
			if all and a.off ~= b.off then
				return a.off < b.off
			end
			if a.r ~= b.r then
				return a.r < b.r
			end
			if a.off ~= b.off then
				return a.off < b.off
			end
			return a.ix < b.ix
		end)
		local out: { Dropdown.Item } = {}
		for _, row in rows do
			table.insert(out, row.item)
		end
		return out
	end
	-- The rows as they stand, for the keys and the groups.
	local shown_rows = ui.computed(function(): { Dropdown.Item }
		return rank(items:get(), query:get())
	end)
	local function row_step(): number
		return (if compact() then COMPACT_ROW else FULL_ROW) + ((ui.theme.get("zeron.list.gap") :: number?) or 2)
	end

	-- The compact list's groups: the starred models, then a run per agent; where
	-- each starts (none while searching).
	type Group = { id: string, start: number }
	local groups = ui.computed(function(): { Group }
		local out: { Group } = {}
		if query:get() ~= "" or not compact() then
			return out
		end
		local by = by_id:get()
		local stars = favorites:get()
		for i, item in shown_rows:get() do
			local m = by[item.value]
			local id = if stars[item.value] then STARRED elseif m then m.harness else STARRED
			if #out == 0 or out[#out].id ~= id then
				table.insert(out, { id = id, start = i })
			end
		end
		return out
	end)
	local function group_at(): string?
		local top = math.floor(offset:get() / row_step() + 0.5) + 1
		local found: string? = nil
		for _, g in groups:get() do
			if g.start <= top then
				found = g.id
			end
		end
		return found
	end
	local function jump_to(g: Group)
		local rows = shown_rows:peek()
		local row = rows[g.start]
		ui.batch(function()
			offset:set((g.start - 1) * row_step())
			highlight:set(if row then row.value else nil)
		end)
	end

	-- The corner of what sits against the panel's edge.
	local corner = control.nest("$radius.lg", 1, "$space.1")
	-- A tab's icon, any agent's.
	local function agent_mark(h: Harness, size: number): Node
		return AgentMark { label = h.label, seed = h.id, mark = if h.icon then nil else h.mark, icon = h.icon, size = size, bare = true }
	end

	-- Moving to an agent: browsing it on a chat, on a new chat its last model at
	-- that model's last settings.
	local function go_to_agent(id: string)
		if not compact() and LAST_TAB:peek() ~= id then
			LAST_TAB:set(id)
		end
		if id == STARRED then
			ui.batch(function()
				tab:set(id)
				offset:set(0)
			end)
			return
		end
		ui.batch(function()
			tab:set(id)
			offset:set(0)
			if committed() then
				return
			end
			local to = LAST_MODEL:peek()[id]
			local list = read(props.models) :: { Model }
			local found: Model? = nil
			for _, m in list do
				if m.harness == id and (found == nil or m.value == to) then
					found = m
				end
			end
			if found and props.model:peek() ~= found.value then
				local take = LAST_EFFORT:peek()[found.value]
				props.model:set(found.value)
				if take then
					props.effort:set(take)
				end
				fit_effort()
			end
		end)
	end

	-- The tabs: the starred models, then an agent's mark each; the shown
	-- one bright, with its bar on the rule below.
	local function tab_of(id: string, label: string, glyph: Node): Node
		local shown = function(): boolean
			return viewed() == id
		end
		return ui.button {
			label = label,
			role = "tab",
			shrink = 0,
			w = 32,
			h = 32,
			relative = true,
			items = "center",
			justify = "center",
			tooltip = label,
			-- A button's corner, as the icon buttons beside it.
			rounded = "$zeron.icon_button.radius",
			bg = "#00000000",
			color = function()
				return if shown() then "$text.primary" else "$text.tertiary"
			end,
			hover = { bg = "$hover", color = "$text.primary" },
			active = { bg = "$pressed" },
			on_click = function()
				go_to_agent(id)
			end,
			glyph,
			ui.show(
				shown,
				ui.div {
					absolute = true,
					left = 8,
					right = 8,
					-- On the rule between the tabs and the list: under
					-- them, or over them when the panel reads bottom up.
					top = if bottom_up then -4 else nil,
					bottom = if bottom_up then nil else -4,
					h = 2,
					rounded = "$radius.full",
					bg = "$accent",
				}
			),
		}
	end
	local starred_tab = tab_of(
		STARRED,
		locale.s("ds.model_picker.starred"),
		Icon {
			name = function(): Icon.Name
				return if viewed() == STARRED then "star-bold" else "star"
			end,
			size = 16,
		}
	)
	-- Profiles, after the starred (gpui's `model-tab-profiles`): the card of
	-- them, in the picker's place.
	local on_profiles = props.on_profiles
	local profile_dot = props.profile_dot
	local profiles_tab = if on_profiles
		then ui.button {
			id = "model-tab-profiles",
			label = locale.t("ds.profiles.title"),
			role = "tab",
			shrink = 0,
			w = 32,
			h = 32,
			relative = true,
			items = "center",
			justify = "center",
			tooltip = locale.s("ds.profiles.title"),
			rounded = "$zeron.icon_button.radius",
			bg = "#00000000",
			color = "$text.tertiary",
			hover = { bg = "$hover", color = "$text.primary" },
			active = { bg = "$pressed" },
			on_click = on_profiles,
			Icon { name = "user-rounded", size = 16 },
			ui.show(function(): boolean
				return profile_dot ~= nil and profile_dot() ~= nil
			end, ui.div {
				id = "model-tab-profiles-dot",
				absolute = true,
				top = 6,
				right = 6,
				w = 5,
				h = 5,
				rounded = "$radius.full",
				bg = function(): string
					return if profile_dot and profile_dot() == "warning" then "$warning" else "$accent"
				end,
			}),
		}
		else nil
	-- An agent's tab each, following the agents as they come and go.
	local agent_tabs = ui.each(ui.computed(harness_list), function(h: Harness): Node
		return tab_of(h.id, h.label, agent_mark(h, 16))
	end, {
		key = function(h: Harness): string
			return h.id
		end,
	})
	-- The strip's scroll: the shown tab in sight as the panel opens or the keys
	-- move it, as the compact list's group as it scrolls.
	local strip_offset = ui.use_state(0)
	local function reveal_strip(place: number)
		local w = 34
		local left, right = (place - 1) * w, place * w
		local at = strip_offset:peek()
		local room = (if compact() then COMPACT_W - 44 else FULL_W - 44) - 12
		if left < at then
			strip_offset:set(left)
		elseif right > at + room then
			strip_offset:set(right - room)
		end
	end
	ui.effect(function()
		if not on_list() then
			return
		end
		if compact() then
			local now = group_at()
			for i, g in groups:get() do
				if g.id == now then
					reveal_strip(i)
					return
				end
			end
			return
		end
		local v = viewed()
		for i, h in harness_list() do
			if h.id == v then
				reveal_strip(i + (if on_profiles then 2 else 1))
				return
			end
		end
		reveal_strip(1)
	end)

	local toggle_layout: () -> ()
	-- The layout's toggle: the panel's header, or the end of the tabs.
	local function layout_toggle(stretch: boolean): Node
		local hint = function(): string
			return locale.s(if compact() then "ds.model_picker.to_full" else "ds.model_picker.to_compact")
		end
		return ui.button {
			id = "model-picker-layout",
			label = hint,
			tooltip = Tooltip { hint },
			w = 32,
			h = if stretch then nil else 32,
			self = if stretch then "stretch" else nil,
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = "$zeron.menu.row_radius",
			bg = "#00000000",
			color = "$text.tertiary",
			hover = { bg = "$hover" },
			active = { bg = "$pressed" },
			on_click = function()
				toggle_layout()
			end,
			Icon {
				name = function(): Icon.Name
					return if compact() then "expand-arrows" else "collapse-arrows"
				end,
				size = 15,
			},
		}
	end

	-- A row's star: shown under the pointer (or the keys), and always on a
	-- starred model; a click stars it or takes the star off.
	local function star(item: Dropdown.Item, lit: () -> boolean): Node
		if item.inert then
			return ui.div {}
		end
		local function hint(): string
			return locale.s(if starred(item.value) then "ds.model_picker.unstar_named" else "ds.model_picker.star_named", { label = item.label })
		end
		local button: icon_button.Props = {
			-- (Followed as it's starred and unstarred.)
			label = hint,
			tooltip = Tooltip {
				function(): string
					return `{hint()}\n{locale.s("ds.model_picker.star_keys")}`
				end,
			} :: any,
			size = "sm",
			tone = "quiet",
			self = "center",
			-- Starred, the accent (the warning's, in the full list).
			color = function()
				if starred(item.value) then
					return if compact() then "$accent" else "$warning"
				end
				return "$text.tertiary"
			end,
			on_click = function()
				toggle_star(item.value)
			end,
			icon = function(): Icon.Name
				return if starred(item.value) then "star-bold" else "star"
			end,
		}
		-- The first nine rows' keys, in the full list.
		local function key_hint(): string?
			if compact() then
				return nil
			end
			for i, row in shown_rows:get() do
				if i > 9 then
					return nil
				end
				if row.value == item.value then
					return `cmd+{i}`
				end
			end
			return nil
		end
		local m = by_id:peek()[item.value]
		return ui.row {
			self = "center",
			shrink = 0,
			gap = "$space.1",
			items = "center",
			if m and m.new
				then ui.show(function(): boolean
					return not compact()
				end, ui.text {
					locale.s("ds.model_picker.new_badge"),
					px = "$zeron.space.tight",
					rounded = "$radius.sm",
					bg = "$zeron.slider.accent_wash",
					color = "$accent",
					text_size = ui.raw(10, "gpui's small print"),
					weight = "$font.weight.semibold",
				})
				else nil,
			ui.show(function(): boolean
				return key_hint() ~= nil and lit()
			end, ui.text {
				function(): string
					local k = key_hint()
					return if k then `⌘{string.sub(k, 5)}` else ""
				end,
				text_size = "$font.size.xs",
				color = "$text.tertiary",
			}),
			-- Never unmounted: a click lands on its row's star even when hover lit another row.
			ui.div {
				shrink = 0,
				opacity = function(): number
					return if lit() or starred(item.value) then 1 else 0.01
				end,
				IconButton(button),
			},
		}
	end

	-- Fast mode as Zeron's picker shows it beside the reasoning: a bare
	-- bolt (a plate only under the pointer), filling as it turns on, the
	-- glyph's colour the state.
	local fast = props.fast
	local function fast_tip(): string
		local on = fast ~= nil and fast:get()
		return locale.s(if on then "ds.model_picker.fast_on" else "ds.model_picker.fast_off")
	end
	local function bolt(size: number): Node?
		if not fast then
			return nil
		end
		local on = fast
		local g = 15
		return ui.button {
			id = "model-picker-fast",
			label = locale.t("ds.model_picker.fast_mode"),
			tooltip = Tooltip { fast_tip },
			role = "switch",
			checked = on,
			w = size,
			h = if size == 32 then nil else size,
			self = if size == 32 then "stretch" else "center",
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = "$zeron.menu.row_radius",
			bg = "#00000000",
			hover = { bg = "$hover" },
			active = { bg = "$pressed" },
			-- On, the accent.
			color = function()
				return if on:get() then "$accent" else "$text.tertiary"
			end,
			on_click = function()
				on:toggle()
			end,
			ui.overlay {
				w = g,
				h = g,
				Icon { name = "fast-tier", size = g },
				ui.div {
					opacity = ui.tween(on, { duration = 160 }),
					Icon { name = "fast-tier-bold", size = g },
				},
			},
		}
	end

	-- An option's row (Zeron's setting row): its name, the choice made
	-- (quiet) and a chevron, opening its choices beside the panel as the
	-- composer's Where rows do: a dropdown whose trigger is the row, the
	-- chosen one in the selected state, the option's start tagged.
	-- `lit`: the keys are on it; `ask`, bumped, it opens.
	-- The option whose choices stand open beside the panel. A row hovered opens
	-- its own at once with none open, as Zeron's hover intent; with another
	-- open, only once the pointer has stayed 300 ms, so a path across a row to
	-- the open list doesn't switch it.
	local open_option: Dropdown.Menu? = nil
	local function option_row(opt: Option, lit: (() -> boolean)?, ask: State<number>?): Node
		local value = ui.use_state(chosen(opt).id :: string?)
		ui.effect(function()
			local c = chosen(opt).id
			if value:peek() ~= c then
				value:set(c)
			end
		end)
		local items: { Dropdown.Item } = {}
		for _, c in opt.choices do
			table.insert(items, { value = c.id, label = c.label, tag = if c.id == opt.default then locale.s("ds.model_picker.default") else nil })
		end
		return Dropdown {
			value = value,
			items = items,
			label = opt.label,
			search = false,
			width = CHOICES_W,
			open_with = ask,
			-- Close beside the panel, as a submenu sits: 2 from its edge (the
			-- row it opens from sits 5 inside it, its inset and border).
			placement = { side = "right", align = "start", offset = 7 },
			-- Left closes it, as Escape does.
			on_key = function(e: any, _hi: string?, panel_api: { close: () -> (), choose: (value: string) -> () }): boolean
				if e.key == "ArrowLeft" then
					panel_api.close()
					return true
				end
				return false
			end,
			on_choose = function(c: string)
				pick(opt, c)
			end,
			trigger = function(menu: Dropdown.Menu): Node
				local by_hover = false
				local waiting: (() -> ())? = nil
				ui.effect(function()
					if menu.open:get() then
						open_option = menu
					else
						by_hover = false
					end
				end)
				local function stop_waiting()
					if waiting then
						waiting()
						waiting = nil
					end
				end
				local function open_by_hover()
					local other = open_option
					if other and other ~= menu and other.open:peek() then
						other.open:set(false)
					end
					by_hover = true
					menu.toggle()
				end
				local row: ContainerProps = {
					label = opt.label,
					bg = function()
						return if menu.open:get() or (lit ~= nil and lit()) then "$hover" else "#00000000"
					end,
					on_hover = function(inside: boolean)
						stop_waiting()
						if not inside or menu.open:peek() then
							return
						end
						local other = open_option
						if other == nil or other == menu or not other.open:peek() then
							open_by_hover()
							return
						end
						waiting = ui.after(300, function()
							waiting = nil
							if not menu.open:peek() then
								open_by_hover()
							end
						end)
					end,
					-- (Opened by the hover, a press keeps it open.)
					on_click = function()
						stop_waiting()
						if menu.open:peek() and by_hover then
							by_hover = false
							return
						end
						menu.toggle()
					end,
					ui.text { opt.label, text_size = "$font.size.md", color = "$text.primary", grow = 1 },
					-- The choice, quiet, held to a width (fading past it).
					FadeText {
						function(): string
							return chosen(opt).label
						end,
						text_size = "$font.size.md",
						color = "$text.tertiary",
						max_w = 120,
					},
					Menu.chevron(),
				}
				Menu.look(row)
				return ui.row(row)
			end,
		}
	end

	-- The chosen model's agent's accounts (its provider's, where the agent
	-- keeps a login per provider), and the one in use.
	local function provider_of(m: Model?): string?
		return if m and m.provider then m.provider.id else nil
	end
	local function accounts(): { Account }
		local m = current()
		return (if m and props.accounts then props.accounts(m.harness, m.value, provider_of(m)) else nil) or {}
	end
	-- Those accounts' provider, named, and how to add one (nil: the agent
	-- keeps one login).
	local function group(): AccountGroup?
		local m = current()
		return if m and props.account_group then props.account_group(m.harness, provider_of(m)) else nil
	end
	-- Whether the row shows: an account to show, or one the provider can add.
	local function has_account_row(): boolean
		if #accounts() > 0 then
			return true
		end
		local g = group()
		return g ~= nil and g.add ~= nil
	end
	local function account_label(): string
		local g = group()
		return if g then locale.s("ds.model_picker.account_of", { name = g.name }) else locale.s("ds.model_picker.account")
	end
	local function account_by_id(id: string?): Account?
		for _, a in accounts() do
			if a.id == id then
				return a
			end
		end
		return nil
	end
	local function in_use(): Account?
		for _, a in accounts() do
			if a.active then
				return a
			end
		end
		return nil
	end
	local function used(a: Account?): number
		return if a and a.used then a.used else 0
	end
	local function percent(f: number): string
		return locale.s("ds.composer.context.percent", { n = math.floor(f * 100 + 0.5) })
	end

	-- The account row: its name, the address in use and how much of it is
	-- used (quiet, as an option's choice is; the figure in the ring's ink),
	-- opening the agent's accounts beside the panel as an option's choices do.
	local function account_row(): Node
		local value = ui.use_state(nil :: string?)
		local function sync()
			local a = in_use()
			value:set(if a then a.id else nil)
		end
		ui.effect(sync)
		return Dropdown {
			value = value,
			items = function(): { Dropdown.Item }
				local out: { Dropdown.Item } = {}
				for _, a in accounts() do
					local words = { a.plan or "", a.detail }
					table.insert(out, {
						value = a.id,
						label = a.email,
						private = true,
						disabled = not a.active and not a.can_switch,
						description = if words[1] ~= "" and words[2] ~= ""
							then `{words[1]} · {words[2]}`
							else words[1] .. words[2],
					})
				end
				return out
			end,
			label = account_label,
			search = false,
			width = 300,
			placement = { side = "right", align = "start", offset = 7 },
			on_open = sync,
			-- (Another of the model's provider's accounts, where Zeron can
			-- sign in to it: "Add ChatGPT account".)
			actions = {
				{
					label = function(): string
						local g = group()
						return if g and g.add then g.add else ""
					end,
					icon = "plus",
					shown = function(): boolean
						local g = group()
						return g ~= nil and g.add ~= nil and g.on_add ~= nil
					end,
					on_click = function()
						local g = group()
						if g and g.on_add then
							g.on_add()
						end
					end,
				},
			},
			trailing_item = function(item: Dropdown.Item): Node?
				return ui.show(function(): boolean
					local a = account_by_id(item.value)
					return a ~= nil and a.used ~= nil
				end, ui.div {
					self = "center",
					shrink = 0,
					context_usage.Ring {
						ink = context_usage.usage_ink,
						fraction = function(): number
							return used(account_by_id(item.value))
						end,
					},
				})
			end,
			on_choose = function(id: string)
				local m = current()
				if m and props.on_account then
					props.on_account(m.harness, id)
				end
			end,
			trigger = function(menu: Dropdown.Menu): Node
				local row: ContainerProps = {
					id = "model-picker-account",
					label = account_label,
					bg = function()
						return if menu.open:get() then "$hover" else "#00000000"
					end,
					on_click = menu.toggle,
					ui.text { account_label, text_size = "$font.size.md", color = "$text.primary", grow = 1, min_w = 0, truncate = true },
					-- (None in use: none signed in to the model's provider.)
					ui.show(function(): boolean
						return in_use() == nil
					end, ui.text { locale.t("ds.model_picker.no_account"), shrink = 0, text_size = "$font.size.md", color = "$text.tertiary" }),
					PrivateText {
						function(): string
							local a = in_use()
							return if a then a.email else ""
						end,
						reveal = false,
						truncate = true,
						text_size = "$font.size.md",
						color = "$text.tertiary",
						max_w = 128,
						shrink = 1,
					},
					ui.show(function(): boolean
						local a = in_use()
						return a ~= nil and a.used ~= nil
					end, ui.text {
						function(): string
							return percent(used(in_use()))
						end,
						tabular = true,
						shrink = 0,
						text_size = "$font.size.md",
						-- (Quiet until it's near a limit, as Zeron's ring's figure.)
						color = function(): string
							local f = used(in_use())
							return if f >= 0.8 then context_usage.usage_ink(f) else "$text.tertiary"
						end,
					}),
					Menu.chevron(),
				}
				Menu.look(row)
				return ui.row(row)
			end,
		}
	end

	-- The model's context options, when it has both: a card of their own.
	local function context_pair(): (Option?, Option?)
		local m = current()
		local window: Option?, auto: Option? = nil, nil
		for _, o in if m then m.options or {} else {} do
			if o.id == CONTEXT_WINDOW then
				window = o
			elseif o.id == AUTO_COMPACT then
				auto = o
			end
		end
		if window and auto then
			return window, auto
		end
		return nil, nil
	end
	-- The stops of the long track: the sizes, then Auto (the model's 1M), last.
	local function long_stops(auto: Option): { Choice }
		local out: { Choice } = {}
		for _, c in auto.choices do
			if c.id ~= "auto" then
				table.insert(out, c)
			end
		end
		for _, c in auto.choices do
			if c.id == "auto" then
				table.insert(out, c)
			end
		end
		return out
	end
	-- Picks both as one write: a window, and where it compacts on it.
	local function set_context(window: Option, auto: Option, choice: string?)
		if not props.options then
			return
		end
		local now = table.clone(props.options:peek())
		if choice == nil then
			now[window.id] = if window.default == "200k" then nil else "200k"
		else
			now[window.id] = if window.default == "1m" then nil else "1m"
			now[auto.id] = if choice == auto.default then nil else choice
		end
		props.options:set(now)
	end
	local function context_card(): Node
		local function pair(): (Option, Option)
			local w, a = context_pair()
			return w :: Option, a :: Option
		end
		local function long_on(): boolean
			local w = context_pair()
			return w ~= nil and chosen(w).id == "1m"
		end
		local function name_of(id: string): string
			local w = context_pair()
			for _, c in if w then w.choices else {} do
				if c.id == id then
					return c.label
				end
			end
			return string.upper(id)
		end
		local function long_levels(): { EffortSlider.Level }
			local _, a = context_pair()
			local out: { EffortSlider.Level } = {}
			for _, c in if a then long_stops(a) else {} do
				table.insert(out, { value = c.id, label = if c.id == "auto" then name_of("1m") else c.label })
			end
			return out
		end
		local names = ui.ref()
		local stop = ui.use_state(nil :: string?)
		ui.effect(function()
			local _, a = context_pair()
			local v = if a then chosen(a).id else nil
			if stop:peek() ~= v then
				stop:set(v)
			end
		end)
		local function threshold(): string
			local window = if long_on() then 1000000 else 200000
			local _, a = context_pair()
			local sizes = { ["300k"] = 300000, ["400k"] = 400000, ["600k"] = 600000, ["800k"] = 800000 }
			local at = window
			if window > 200000 and a then
				at = math.min(sizes[chosen(a).id] or window, window)
			end
			return short_tokens(at - HEADROOM)
		end
		-- The warning: more in use than the smaller window holds.
		local function warning(): string?
			local n = if props.context_tokens then props.context_tokens() else nil
			if not n or n <= 200000 then
				return nil
			end
			if long_on() then
				return locale.s("ds.model_picker.context_switching", { size = short_tokens(200000), in_use = short_tokens(n) })
			end
			return locale.s("ds.model_picker.context_selected", { in_use = short_tokens(n) })
		end
		return ui.column {
			id = "model-picker-context",
			w_full = true,
			px = "$space.1",
			pt = "$zeron.space.hair",
			pb = "$space.1",
			-- Its heading, the long context switch beside it.
			ui.row {
				w_full = true,
				h = 20,
				pl = "$zeron.space.hair",
				items = "center",
				justify = "between",
				gap = "$space.2",
				ui.text { locale.t("ds.model_picker.context"), text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
				ui.button {
					id = "model-picker-long",
					role = "switch",
					checked = long_on,
					label = locale.t("ds.model_picker.long_context"),
					h = 20,
					px = ui.raw(7, "gpui's long context pill"),
					gap = "$zeron.space.tight",
					items = "center",
					rounded = "$radius.full",
					bg = function()
						return if long_on() then "$zeron.slider.accent_wash" else "$hover"
					end,
					color = function()
						return if long_on() then "$accent" else "$text.tertiary"
					end,
					text_size = "$font.size.xs",
					on_click = function()
						local w, a = pair()
						set_context(w, a, if long_on() then nil else chosen(a).id)
					end,
					ui.div {
						w = 24,
						h = 14,
						relative = true,
						rounded = "$radius.full",
						bg = function()
							return if long_on() then "$accent" else "$border.strong"
						end,
						ui.div {
							absolute = true,
							top = 2,
							w = 10,
							h = 10,
							rounded = "$radius.full",
							bg = "$zeron.switch.knob",
							left = ui.tween(ui.computed(function(): number
								return if long_on() then 12 else 2
							end), { duration = 140 }),
						},
					},
					ui.text { locale.t("ds.model_picker.long_context") },
				},
			},
			-- The 200K zone left of the track, then the long stops.
			ui.row {
				w_full = true,
				mt = "$zeron.space.hair",
				gap = "$space.1",
				items = "center",
				ui.button {
					id = "model-picker-200k",
					label = function(): string
						return name_of("200k")
					end,
					shrink = 0,
					w = 44,
					h = 28,
					items = "center",
					justify = "center",
					rounded = "$radius.full",
					text_size = "$font.size.xs",
					border = 1,
					border_color = "$zeron.slider.rail_edge",
					bg = function()
						return if long_on() then "#00000000" else "$zeron.slider.warning_wash"
					end,
					color = function()
						return if long_on() then "$text.tertiary" else "$warning"
					end,
					hover = { bg = "$hover" },
					on_click = function()
						local w, a = pair()
						set_context(w, a, nil)
					end,
					ui.text {
						function(): string
							return name_of("200k")
						end,
					},
				},
				ui.div {
					grow = 1,
					min_w = 0,
					opacity = ui.tween(ui.computed(function(): number
						return if long_on() then 1 else 0.5
					end), { duration = 120 }),
					EffortSlider {
						plain = true,
						keys = false,
						label = locale.t("ds.model_picker.context"),
						value = stop,
						levels = long_levels,
						none = function(): boolean
							return not long_on()
						end,
						on_change = function(v: string)
							local w, a = pair()
							set_context(w, a, v)
						end,
					},
				},
			},
			-- Each stop's name under its stop, the current one bold.
			ui.div {
				ref = names,
				w_full = true,
				relative = true,
				h = 14,
				mt = "$zeron.space.hair",
				opacity = ui.tween(ui.computed(function(): number
					return if long_on() then 1 else 0.5
				end), { duration = 120 }),
				ui.each(ui.computed(long_levels), function(l: EffortSlider.Level): Node
					return ui.button {
						label = l.label,
						absolute = true,
						top = 0,
						w = 40,
						h = 14,
						items = "center",
						justify = "center",
						bg = "#00000000",
						hover = { color = "$text.primary" },
						left = function(): number
							local list = long_levels()
							local at = 0
							for i, other in list do
								if other.value == l.value then
									at = i - 1
								end
							end
							-- Past the 200K zone (44 and its gap), the stops share the
							-- thumb's half width as their inset.
							local run = math.max((names.width or 200) - 48 - 44, 0)
							return 48 + 22 + (at / math.max(#list - 1, 1)) * run - 20
						end,
						text_size = ui.raw(10.5, "gpui's stop names"),
						weight = function()
							return if long_on() and stop:get() == l.value then "$font.weight.semibold" else "$font.weight.regular"
						end,
						color = function()
							return if long_on() and stop:get() == l.value then "$text.primary" else "$text.tertiary"
						end,
						on_click = function()
							local w, a = pair()
							set_context(w, a, l.value)
						end,
						ui.text { l.label },
					}
				end, {
					key = function(l: EffortSlider.Level): string
						return l.value
					end,
				}),
			},
			ui.row {
				id = "model-picker-context-readout",
				h = 14,
				mt = "$space.1",
				px = "$zeron.space.hair",
				items = "center",
				text_size = "$font.size.xs",
				color = "$text.tertiary",
				ui.text {
					function(): string
						return locale.s("ds.model_picker.window", { size = name_of(if long_on() then "1m" else "200k") })
					end,
					color = function()
						return if long_on() then "$text.tertiary" else "$warning"
					end,
				},
				ui.text { "\u{a0}·\u{a0}" },
				ui.text {
					function(): string
						return locale.s("ds.model_picker.compacts_at", { at = threshold() })
					end,
				},
			},
			ui.show(function(): boolean
				return warning() ~= nil
			end, ui.text {
				function(): string
					return warning() or ""
				end,
				mt = "$space.1",
				px = "$zeron.space.hair",
				wrap = true,
				text_size = "$font.size.xs",
				color = "$warning",
			}),
		}
	end

	-- The panel's controls. One slider while the model has levels, following the
	-- model as it changes (its levels, its bolt), so moving between models moves
	-- its thumb rather than making it again. The rows follow the model too, each
	-- keyed by its model and option: a model's own rows come and go with it.
	local function has_fast(): boolean
		local m = current()
		return m ~= nil and m.fast == true
	end
	local function has_levels(): boolean
		return #levels() > 0
	end
	-- The keys in the full tray: which of its controls they're on.
	local tray_at = ui.use_state(nil :: number?)
	type Owned = { model: Model, option: Option, place: number }
	-- Bumped, an option's row opens (the keys'), by its model and option.
	local asks: { [string]: State<number> } = {}
	local function ask_of(key: string): State<number>
		asks[key] = asks[key] or ui.state(`model_option_{key}`, 0)
		return asks[key]
	end
	local owned = ui.computed(function(): { Owned }
		local m = current()
		local out: { Owned } = {}
		local eff = effort_option()
		local window = context_pair()
		for _, o in if m then ordered(m) else {} do
			local hidden = (eff and o.id == eff.id) or (window ~= nil and (o.id == CONTEXT_WINDOW or o.id == AUTO_COMPACT))
			if not hidden then
				table.insert(out, { model = m :: Model, option = o, place = #out + 1 })
			end
		end
		return out
	end)
	local function has_context(): boolean
		local w = context_pair()
		return w ~= nil
	end
	-- The tray's stops: the slider, then each option, then the context card.
	local function tray_stops(): number
		return (if has_levels() then 1 else 0) + #owned:get() + (if has_context() then 1 else 0)
	end
	local function option_rows(): Node
		local base = function(): number
			return if has_levels() then 1 else 0
		end
		local rows = ui.each(owned, function(it: Owned): Node
			return option_row(it.option, function(): boolean
				return tray_at:get() == base() + it.place
			end, ask_of(`{it.model.value}/{it.option.id}`))
		end, {
			key = function(it: Owned): string
				return `{it.model.value}/{it.option.id}`
			end,
			w_full = true,
			gap = "$zeron.list.gap",
		})
		return ui.show(function(): boolean
			return #owned:get() > 0
		end, ui.scroll {
			w_full = true,
			h = function(): number
				return math.min(#owned:get(), OPTIONS_MAX) * OPTION_PITCH - 2
			end,
			fade = { 12, 12 },
			fade_overflow = true,
			rows,
		})
	end
	local function slider(): Node
		return EffortSlider {
			value = slider_value,
			levels = levels,
			on_change = set_level,
			live = dragged,
			keys = false,
			highlighted = function(): boolean
				return tray_at:get() == 1 and has_levels()
			end,
			fast = function(): boolean
				return fast ~= nil and fast:get() and has_fast()
			end,
		}
	end
	local go_list: (typed: string?) -> ()

	-- Compact: the panel's title (the level over the model's name, one button
	-- filling the row up to the bolt), then its controls.
	local function chosen_name(): string
		local m = current()
		if m then
			return m.label
		end
		local v = props.model:get()
		if v then
			return string.match(v, "^[^/]+/(.+)$") or v
		end
		if props.looking and props.looking() then
			return locale.s("ds.model_picker.loading")
		end
		if #harness_list() == 0 then
			return locale.s("ds.model_picker.no_agents")
		end
		return locale.s("ds.model_picker.select")
	end
	local title_hover = ui.use_state(false)
	local function title(): Node
		return ui.button {
			id = "model-picker-title",
			grow = 1,
			min_w = 0,
			self = "stretch",
			px = "$space.2",
			items = "start",
			justify = "center",
			direction = "column",
			gap = 0,
			rounded = "$zeron.menu.row_radius",
			bg = "#00000000",
			hover = { bg = "$hover" },
			active = { bg = "$pressed" },
			label = function(): string
				return `{level_label() or locale.s("ds.model_picker.default")} · {chosen_name()} · {locale.s("ds.model_picker.change_model")}`
			end,
			on_hover = function(inside: boolean)
				title_hover:set(inside)
			end,
			on_click = function()
				go_list()
			end,
			ui.show(has_levels, RollText {
				function(): string
					return level_label() or ""
				end,
				text_size = ui.raw(14, "gpui's panel title"),
				line_height = 17,
				weight = "$font.weight.semibold",
				color = "$text.primary",
			}),
			ui.row {
				max_w = "100%",
				min_w = 0,
				gap = "$zeron.space.tight",
				items = "center",
				ui.text {
					chosen_name,
					truncate = true,
					min_w = 0,
					shrink = 1,
					line_height = function(): number
						return if has_levels() then 15 else 17
					end,
					text_size = function()
						return if has_levels() then "$font.size.sm" else ui.raw(14, "gpui's panel title")
					end,
					weight = function()
						return if has_levels() then "$font.weight.regular" else "$font.weight.semibold"
					end,
					color = function()
						return if not has_levels() or title_hover:get() then "$text.primary" else "$text.tertiary"
					end,
				},
				ui.div {
					shrink = 0,
					x = ui.tween(ui.computed(function(): number
						return if title_hover:get() then 3 else 0
					end), { duration = 150 }),
					opacity = ui.tween(ui.computed(function(): number
						return if title_hover:get() then 1 else 0.55
					end), { duration = 150 }),
					color = function()
						return if title_hover:get() then "$text.primary" else "$text.tertiary"
					end,
					Icon { name = "alt-arrow-right", size = 10 },
				},
			},
		}
	end
	local function panel(): Node
		return ui.column {
			id = "model-picker-panel",
			w_full = true,
			ui.row {
				w_full = true,
				h = function(): number
					return if has_levels() then 48 else 36
				end,
				gap = "$space.1",
				title(),
				if fast then ui.show(has_fast, bolt(32) :: Node) else nil,
				layout_toggle(true),
			},
			ui.show(has_levels, ui.div {
				w_full = true,
				px = "$space.2",
				pt = "$zeron.space.hair",
				pb = "$space.1",
				slider(),
			}),
			ui.show(function(): boolean
				return has_account_row() or #owned:get() > 0 or has_context()
			end, ui.column {
				w_full = true,
				mt = "$space.1",
				gap = "$zeron.list.gap",
				ui.show(function(): boolean
					return has_account_row()
				end, account_row()),
				ui.show(has_context, context_card()),
				option_rows(),
			}),
		}
	end

	-- Compact: the list's head: back to the panel, then the providers as tabs,
	-- each jumping the list to its group, the group at the top lighting its tab.
	local function group_tab(g: Group): Node
		local h = agents:peek()[g.id]
		local name = if g.id == STARRED then locale.s("ds.model_picker.starred") elseif h then h.label else g.id
		local function viewed_now(): boolean
			return group_at() == g.id
		end
		return ui.button {
			label = locale.s("ds.model_picker.jump_to", { name = name }),
			tooltip = name,
			shrink = 0,
			w = 32,
			h = 32,
			items = "center",
			justify = "center",
			rounded = "$zeron.menu.row_radius",
			bg = function()
				return if viewed_now() then "$hover" else "#00000000"
			end,
			opacity = function()
				return if viewed_now() then 1 else 0.7
			end,
			hover = { bg = "$hover", opacity = 1 },
			color = function()
				return if viewed_now() then "$text.primary" else "$text.tertiary"
			end,
			on_click = function()
				for _, now in groups:peek() do
					if now.id == g.id then
						jump_to(now)
					end
				end
			end,
			if g.id == STARRED then Icon { name = "star-bold", size = 14 } elseif h then agent_mark(h, 14) else nil,
		}
	end
	local function back_head(): Node
		return ui.row {
			w_full = true,
			h = 32,
			gap = "$zeron.list.gap",
			items = "center",
			IconButton {
				label = locale.t("ds.model_picker.back"),
				tooltip = locale.s("ds.model_picker.back_esc"),
				size = "lg",
				tone = "quiet",
				icon = "alt-arrow-left",
				on_click = function()
					listing:set(false)
				end,
			},
			ui.scroll_x {
				grow = 1,
				min_w = 0,
				h = 32,
				offset = strip_offset,
				fade_x = { 20, 20 },
				fade_overflow = true,
				ui.each(groups, group_tab, {
					key = function(g: Group): string
						return g.id
					end,
					direction = "row",
					h = 32,
					gap = "$zeron.list.gap",
				}),
			},
		}
	end
	-- Full: the tabs, the layout's toggle at their end.
	local function tab_head(): Node
		return ui.row {
			w_full = true,
			h = 36,
			mt = if bottom_up then ui.raw(-4, "the tab bar meets the rule") else nil,
			mb = if bottom_up then nil else ui.raw(-4, "the tab bar meets the rule"),
			gap = "$zeron.list.gap",
			items = if bottom_up then "end" else "start",
			-- The tabs scroll across when there are more than fit, each edge
			-- fading only where more runs past it. (36 tall, 4 into the head's
			-- inset by the list: the shown tab's bar meets the rule.)
			ui.scroll_x {
				-- Its tabs and buttons sit in the panel's corner: concentric with it.
				theme = { zeron = { icon_button = { radius = corner } } },
				grow = 1,
				min_w = 0,
				h = 36,
				offset = strip_offset,
				fade_x = { 20, 20 },
				fade_overflow = true,
				ui.row {
					h = 36,
					gap = "$zeron.space.hair",
					-- Its tabs to the side away from the list, so the bar (4 past
					-- them, on the rule by the list) stays inside the strip's clip.
					items = if bottom_up then "end" else "start",
					starred_tab,
					profiles_tab,
					agent_tabs,
				},
			},
			layout_toggle(false),
		}
	end

	-- Full: the tray: a bolt, the level's name and a reset over the slider; then
	-- the options; a model with Fast mode and no reasoning has its switch row.
	local function at_default(): boolean
		local m = current()
		if not m then
			return true
		end
		local fast_off = fast == nil or not fast:get()
		return fast_off and (not m.efforts or #m.efforts == 0 or props.effort:get() == m.effort)
	end
	local function effort_card(): Node
		return ui.column {
			id = "model-picker-effort",
			w_full = true,
			px = "$space.1",
			pt = "$zeron.space.hair",
			ui.row {
				w_full = true,
				h = 28,
				items = "center",
				gap = "$space.1",
				ui.div {
					w = 24,
					shrink = 0,
					if fast then ui.show(has_fast, bolt(24) :: Node) else nil,
				},
				ui.div {
					grow = 1,
					min_w = 0,
					items = "center",
					RollText {
						function(): string
							return level_label() or ""
						end,
						text_size = "$font.size.md",
						weight = "$font.weight.medium",
						color = "$accent",
					},
				},
				IconButton {
					label = locale.t("ds.model_picker.reset_effort"),
					tooltip = locale.s("ds.model_picker.reset_effort"),
					size = "sm",
					tone = "quiet",
					icon = "restart",
					opacity = function(): number
						return if at_default() then 0.35 else 1
					end,
					on_click = function()
						local m = current()
						if not m or at_default() then
							return
						end
						ui.batch(function()
							if m.effort then
								props.effort:set(m.effort)
							end
							if fast then
								fast:set(false)
							end
						end)
					end,
				},
			},
			ui.div { w_full = true, px = "$zeron.space.hair", slider() },
		}
	end
	local function fast_row(): Node?
		if not fast then
			return nil
		end
		local on = fast
		local row: ContainerProps = {
			on_click = function()
				on:toggle()
			end,
			ui.text { locale.t("ds.model_picker.fast_mode"), text_size = "$font.size.md", color = "$text.primary", grow = 1 },
			Switch {
				value = on,
				label = locale.t("ds.model_picker.fast_mode") :: any,
			},
		}
		Menu.look(row)
		return ui.row(row)
	end
	local switch_row = fast_row()
	local tray: { Node } = {
		ui.show(function(): boolean
			return has_account_row()
		end, account_row()),
		ui.show(has_levels, effort_card()),
		ui.show(has_context, context_card()),
		option_rows(),
	}
	if switch_row then
		table.insert(
			tray,
			ui.show(function(): boolean
				return has_fast() and not has_levels()
			end, switch_row)
		)
	end

	-- Going to the list: on the chosen model's row, `typed` starting a search.
	go_list = function(typed: string?)
		ui.batch(function()
			listing:set(true)
			query:set(typed or "")
			-- On the chosen model: its group at the top when it's near the start of
			-- it (a row of the last above, so the list shows there's more), else
			-- with the rows around it.
			local rows = shown_rows:peek()
			local at = 1
			for i, r in rows do
				if r.value == props.model:peek() then
					at = i
				end
			end
			local start = 1
			for _, g in groups:peek() do
				if g.start <= at then
					start = g.start
				end
			end
			if at - start < COMPACT_ROWS - 2 then
				offset:set(math.max(0, (start - (if start > 1 then 2 else 1)) * row_step()))
			else
				offset:set(math.max(0, (at - 1 - math.floor(COMPACT_ROWS / 2)) * row_step()))
			end
			highlight:set(if rows[at] then rows[at].value else nil)
		end)
	end
	-- Changing page, the new one fades in (the panel itself stays solid).
	local reveal = ui.use_state(1)
	local reveal_was = listing:peek()
	ui.effect(function()
		local now = listing:get()
		if now == reveal_was then
			return
		end
		reveal_was = now
		if motion.reduced_now() then
			return
		end
		reveal:set(0)
		anime.animate(reveal, { value = 1, duration = 200, ease = "out(3)" })
	end)
	-- The other layout, opening on what it opens on.
	toggle_layout = function()
		ui.batch(function()
			COMPACT:set(not COMPACT:peek())
			listing:set(false)
			tab:set(nil)
			tray_at:set(nil)
			query:set("")
		end)
	end

	-- The keys, as Zeron's picker has them.
	local function step_level(key: string): boolean
		local list = levels()
		local n = #list
		if n == 0 then
			return false
		end
		local i = 1
		for at, l in list do
			if l.value == slider_value:peek() then
				i = at
			end
		end
		local to = i
		if key == "ArrowLeft" then
			to = math.max(1, i - 1)
		elseif key == "ArrowRight" then
			to = math.min(n, i + 1)
		elseif key == "Home" then
			to = 1
		else
			to = n
		end
		if to ~= i then
			set_level(list[to].value)
			pcall(ui.invoke, "haptic.step")
		end
		return true
	end
	local function cycle_agent(by: number)
		local list = harness_list()
		if #list == 0 then
			return
		end
		local at = 0
		local now = if compact() then (current() or { harness = "" }).harness else viewed()
		for i, h in list do
			if h.id == now then
				at = i
			end
		end
		local to = (at + by - 1) % #list + 1
		go_to_agent(list[to].id)
		if compact() and committed() then
			go_list()
			-- (The list on that agent's group, as it browses.)
			ui.after(0, function()
				for _, g in groups:peek() do
					if g.id == list[to].id then
						jump_to(g)
					end
				end
			end)
		end
	end
	local function panel_key(e: any): boolean
		local plain = not (e.cmd or e.ctrl or e.alt or e.meta)
		local key: string = e.key
		if key == "ArrowUp" or key == "ArrowDown" then
			go_list()
			-- On the model beside the chosen one.
			ui.after(0, function()
				local rows = shown_rows:peek()
				local at = 1
				for i, r in rows do
					if r.value == props.model:peek() then
						at = i
					end
				end
				local to = math.clamp(at + (if key == "ArrowUp" then -1 else 1), 1, math.max(#rows, 1))
				if rows[to] then
					highlight:set(rows[to].value)
				end
			end)
			return true
		elseif key == "Enter" or key == " " or key == "Space" then
			go_list()
			return true
		elseif key == "Tab" then
			cycle_agent(if e.shift then -1 else 1)
			return true
		elseif key == "ArrowLeft" or key == "ArrowRight" or key == "Home" or key == "End" then
			step_level(key)
			return true
		elseif plain and string.lower(key) == "f" and not e.shift then
			if fast and has_fast() then
				fast:toggle()
			end
			return true
		elseif plain and #key == 1 then
			-- Typing starts a search.
			go_list(key)
			return true
		end
		return false
	end
	local function on_key(e: any, hi: string?, panel_api: { close: () -> (), choose: (value: string) -> () }): boolean
		local key: string = e.key
		-- ⌘⇧F stars the highlighted model, picking nothing.
		if e.cmd and e.shift and not e.alt and string.lower(key) == "f" then
			local v = hi or props.model:peek()
			if v and by_id:peek()[v] then
				toggle_star(v)
				return true
			end
			return false
		end
		if not on_list() then
			return panel_key(e)
		end
		-- ⌘1…⌘9 pick the Nth row.
		if e.cmd and not e.shift and not e.alt then
			local n = tonumber(key)
			if n and n >= 1 and n <= 9 then
				local row = shown_rows:peek()[n]
				if row then
					panel_api.choose(row.value)
				end
				return true
			end
			return false
		end
		if e.ctrl or e.alt then
			return false
		end
		-- Tab: the next tab or group.
		if key == "Tab" then
			if compact() then
				local list = groups:peek()
				local at = 0
				local now = group_at()
				for i, g in list do
					if g.id == now then
						at = i
					end
				end
				if #list > 0 then
					jump_to(list[(at + (if e.shift then -1 else 1) - 1) % #list + 1])
				end
			else
				cycle_agent(if e.shift then -1 else 1)
			end
			return true
		end
		if compact() then
			return false
		end
		-- Full: the arrows go on from the rows into the tray, where ←/→ step the
		-- control they're on, → opens a row's choices.
		local at = tray_at:peek()
		local rows = shown_rows:peek()
		local stops = tray_stops()
		if at then
			if key == "ArrowDown" then
				if at >= stops then
					tray_at:set(nil)
					highlight:set(if rows[1] then rows[1].value else nil)
					offset:set(0)
				else
					tray_at:set(at + 1)
				end
				return true
			elseif key == "ArrowUp" then
				if at <= 1 then
					tray_at:set(nil)
					highlight:set(if rows[#rows] then rows[#rows].value else nil)
				else
					tray_at:set(at - 1)
				end
				return true
			end
			local on_slider = has_levels() and at == 1
			local place = at - (if has_levels() then 1 else 0)
			local option = owned:peek()[place]
			if key == "ArrowLeft" or key == "ArrowRight" or key == "Home" or key == "End" then
				if on_slider then
					step_level(key)
				elseif option and key == "ArrowRight" then
					local ask = ask_of(`{option.model.value}/{option.option.id}`)
					ask:set(ask:peek() + 1)
				elseif not option and has_context() then
					local w, a = context_pair()
					if w and a then
						local list = long_stops(a)
						local i = 0
						if chosen(w).id == "1m" then
							for n, c in list do
								if c.id == chosen(a).id then
									i = n
								end
							end
						end
						i = math.clamp(i + (if key == "ArrowLeft" then -1 else 1), 0, #list)
						set_context(w, a, if i == 0 then nil else list[i].id)
					end
				end
				return true
			elseif key == "Enter" then
				if option then
					local ask = ask_of(`{option.model.value}/{option.option.id}`)
					ask:set(ask:peek() + 1)
				elseif not on_slider and has_context() then
					local w, a = context_pair()
					if w and a then
						set_context(w, a, if chosen(w).id == "1m" then nil else chosen(a).id)
					end
				end
				return true
			end
			return false
		end
		if key == "ArrowDown" and stops > 0 and #rows > 0 and hi == rows[#rows].value then
			highlight:set(nil)
			tray_at:set(1)
			return true
		end
		if key == "ArrowUp" and stops > 0 and #rows > 0 and hi == rows[1].value then
			highlight:set(nil)
			tray_at:set(stops)
			return true
		end
		return false
	end

	-- The chip: the bolt after the effort, its 700 ms sheen as Fast comes on;
	-- then the ring.
	local chip_fast = ui.presence(function(): string?
		if fast and fast:get() and has_fast() then
			return "on"
		end
		return nil
	end, function(): Node
		return ui.div {
			id = "model-picker-chip-fast",
			self = "center",
			shrink = 0,
			color = "$accent",
			on_mount = function(node)
				if not motion.reduced_now() then
					anime.animate(node, {
						opacity = { 0.65, 1 },
						duration = 700,
						ease = "inOutSine",
					})
				end
			end,
			Icon { name = "fast-tier-bold", size = 13 },
		}
	end, { shrink = 0 })

	-- Starting up: the mark's and the name's places held, the chip no narrower.
	local function loading_chip(): boolean
		return current() == nil and props.looking ~= nil and props.looking()
	end

	return Dropdown {
		name_max = function(): number
			return NAME_AT[if props.fold then props.fold() else 0] or NAME_AT[0]
		end,
		mark_when = props.mark_when,
		value = shown_model,
		items = items :: any,
		tone = props.tone or "model_chip",
		mark = true,
		detail = function(): string
			local e = slider_value:get() or shown_effort()
			for _, l in levels() do
				if l.value == e then
					return l.label
				end
			end
			return ""
		end,
		trail = ui.show(function(): boolean
			return fast ~= nil and fast:get() and has_fast()
		end, ui.row { shrink = 0, gap = "$zeron.space.tight", items = "center", chip_fast }),
		-- (Another chat's model and effort, shown as it's moved to: at once.)
		roll_in_panel = true,
		roll_also = function(): boolean
			return stepping
		end,
		-- (Dropdown hands its label on, as the trigger's name.)
		label = props.label or locale.t("ds.model_picker.model") :: any,
		ghost = loading_chip,
		no_mark_icon = function(): Icon.Name?
			return if current() == nil and props.model:get() == nil and not loading_chip() and #harness_list() == 0 then "terminal" else nil
		end,
		content_opacity = function(): number
			return reveal:get()
		end,
		-- (The name the catalog can't give, as the host says it; else, with no model, what stands in.)
		say = function(): string?
			local said = named()
			if said then
				return said
			end
			if current() ~= nil or props.model:get() ~= nil then
				return nil
			end
			return if loading_chip() then "" else chosen_name()
		end,
		search = function(): string
			if compact() then
				return locale.s("ds.model_picker.search_all")
			end
			if viewed() == STARRED then
				return locale.s("ds.model_picker.search_starred")
			end
			local agent = agents:get()[viewed()]
			return if agent then locale.s("ds.model_picker.search_agent", { agent = agent.label }) else locale.s("ds.model_picker.search_all")
		end,
		list_full = function(): boolean
			return not compact()
		end,
		rank = rank,
		query = query,
		offset = offset,
		highlight = highlight,
		take = props.take,
		row_h = COMPACT_ROW,
		view = function(): number
			local rows = if compact() then COMPACT_ROWS else FULL_ROWS
			return 8 + rows * (COMPACT_ROW + 2)
		end,
		row_note = function(item: Dropdown.Item): string?
			local m = by_id:peek()[item.value]
			if not m or not m.description or m.description == "" then
				return nil
			end
			return if same_name:peek()[item.label] then m.description else nil
		end,
		trailing_item = star,
		-- Starred and nothing starred: how to star one. A search that finds
		-- nothing: what it looked for, and where. An agent still listing or
		-- failing: the row that says so.
		empty = function(q: string): Dropdown.Empty
			local agent = agents:get()[viewed()]
			if q == "" then
				if #harness_list() == 0 then
					if props.looking and props.looking() then
						return { title = locale.s("ds.model_picker.loading") }
					end
					return { title = locale.s("ds.model_picker.no_agents"), hint = locale.s("ds.model_picker.no_agents_hint") }
				end
				if viewed() == STARRED then
					return { title = locale.s("ds.model_picker.no_starred"), hint = locale.s("ds.model_picker.no_starred_hint") }
				end
				if agent and agent.state == "loading" then
					return { title = "", skeleton = true }
				end
				local on_retry = props.on_retry
				if agent and agent.state == "error" and on_retry then
					return {
						title = locale.s("ds.model_picker.no_models"),
						hint = agent.error,
						action = { label = locale.s("ds.model_picker.retry"), on_click = on_retry },
					}
				end
				return {
					title = locale.s("ds.model_picker.no_models"),
					hint = if agent then locale.s("ds.model_picker.none_to_offer", { agent = agent.label }) else nil,
				}
			end
			local where = if compact()
				then locale.s("ds.model_picker.where_models")
				elseif viewed() == STARRED then locale.s("ds.model_picker.where_starred")
				elseif agent then agent.label
				else locale.s("ds.model_picker.where_models")
			return { title = locale.s("ds.model_picker.not_found"), hint = locale.s("ds.model_picker.not_found_hint", { where = where, q = q }) }
		end,
		on_open = function()
			-- A new chat with stars opens the full list on them, until a tab is picked.
			local has_stars = next(favorites:peek()) ~= nil
			local fresh = LAST_TAB:peek() == ""
			ui.batch(function()
				tab:set(if fresh and not COMPACT:peek() and not committed() and has_stars then STARRED else nil)
				listing:set(false)
				tray_at:set(nil)
				dragged:set(nil)
			end)
			if props.on_accounts_open then
				props.on_accounts_open()
			end
		end,
		-- Compact, only the list's page has a list.
		list_when = on_list,
		keep_open = true,
		on_choose = function(value: string)
			props.model:set(value)
			ui.batch(function()
				listing:set(false)
				tab:set(nil)
				-- A new chat takes the level last used on the model.
				local remembered = LAST_EFFORT:peek()[value]
				if not committed() and remembered then
					props.effort:set(remembered)
				end
				fit_effort()
				remember(false)
			end)
		end,
		on_key = on_key,
		on_escape = function(): boolean
			-- On the list, Escape steps back to the panel; on the panel, it closes.
			if COMPACT:peek() and listing:peek() then
				listing:set(false)
				return true
			end
			return false
		end,
		width = function(): number
			return if compact() then COMPACT_W else FULL_W
		end,
		open_with = props.open_with,
		close_with = props.close_with,
		showing = props.showing,
		outside = props.outside,
		head = ui.column {
			w_full = true,
			ui.show(function(): boolean
				return compact() and not listing:get()
			end, panel()),
			ui.show(function(): boolean
				return compact() and listing:get()
			end, back_head()),
			ui.show(function(): boolean
				return not compact()
			end, tab_head()),
		},
		-- (The tray's slider and rows in the panel's foot corners.)
		extra = ui.column {
			w_full = true,
			theme = { radius = { md = corner, sm = control.nest(corner, "$zeron.space.hair") } },
			table.unpack(tray),
		},
		extra_when = function(): boolean
			local m = current()
			return not compact()
				and m ~= nil
				and (has_levels() or (m.fast == true and fast ~= nil) or #owned:get() > 0 or has_context() or has_account_row())
		end,
		placement = props.placement,
		bottom_up = bottom_up,
	}
end)
