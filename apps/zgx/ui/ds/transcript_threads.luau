local Icon = require("@self/icon")
local AgentMark = require("@self/agent_mark")
local FadeText = require("@self/fade_text")
local status = require("@self/status")
local locale = require("@self/locale")
local Button = require("@self/button")
local motion = require("@self/motion")
local ProgressBar = require("@self/progress").ProgressBar
local SessionStats = require("@self/session_stats")

local M = {}

ui.theme.tokens {
	zeron = {
		peer_card = {
			fill = "alpha({text.primary}, 0.02)",
			ring = "alpha({accent}, 0.16)",
			sweep = "alpha({accent}, 0.12)",
			rule = "alpha({border}, 0.6)",
			done = "alpha({success}, 0.15)",
			failed = "alpha({danger}, 0.15)",
			stopped = "alpha({text.secondary}, 0.15)",
		},
	},
}

-- (A table's length can't be trusted past a hole.)
local function nodes(...: any): { Node }
	local out: { Node } = {}
	for i = 1, select("#", ...) do
		local n = select(i, ...)
		if n ~= nil then
			table.insert(out, n)
		end
	end
	return out
end

export type Child = {
	id: string,
	title: string?,
	harness: string?,
	model: string?,
	mark: string?,
	agent: string?,
	agent_icon: string?,
	delegation: string?,
	delegated_at: number?,
	delegated_end: number?,
	route: any?,
	status: string?,
}

export type Threads = {
	child: (id: string) -> Child?,
	chat: (key: string) -> { id: string, title: string, mark: string? }?,
	limit: ((id: string) -> any?)?,
	is_open: ((id: string) -> boolean)?,
	open_child: ((id: string) -> ())?,
	open_chat: ((id: string) -> ())?,
	limit_op: ((id: string, op: string) -> ())?,
	agent: (() -> { label: string?, mark: string?, model: string?, harness: string? }?)?,
	model_label: ((harness: string?, model: string) -> string?)?,
	plan: (() -> any?)?,
	approve_plan: (() -> ())?,
	edit_plan: (() -> ())?,
}

local CARD, TILE = 30, 18
local TAG_SIZE = 11

local NAMES = { ["claude-code"] = "Claude Code", codex = "Codex", grok = "Grok", cursor = "Cursor", mock = "Mock", opencode = "OpenCode", devin = "Devin", pi = "Pi", hermes = "Hermes", antigravity = "Antigravity", muse = "Muse" }
function M.harness_name(harness: string?): string
	if not harness then
		return locale.s("ds.threads.agent")
	end
	return NAMES[harness] or harness
end

-- Zeron's `format_reset`, without the "resets".
function M.clock(ms: number?): string?
	if not ms then
		return nil
	end
	local at = ms / 1000
	local ahead = at - os.time()
	local said = if ahead < 22 * 3600
		then string.gsub(os.date("%I:%M %p", at) :: string, "^0", "")
		elseif ahead < 7 * 86400 then os.date("%a", at)
		else string.gsub(os.date("%b %d", at) :: string, " 0", " ")
	return tostring(said)
end

function M.duration(ms: number): string
	local mins = math.max(0, math.floor(ms / 60000))
	local h, m = math.floor(mins / 60), mins % 60
	if h == 0 then
		return `{m}m`
	end
	return if m == 0 then `{h}h` else `{h}h {m}m`
end

function M.elapsed(secs: number): string
	secs = math.max(0, math.floor(secs))
	if secs < 60 then
		return `{secs}s`
	elseif secs < 3600 then
		return `{math.floor(secs / 60)}m {secs % 60}s`
	elseif secs < 86400 then
		return `{math.floor(secs / 3600)}h {math.floor((secs % 3600) / 60)}m`
	end
	return `{math.floor(secs / 86400)}d {math.floor((secs % 86400) / 3600)}h`
end

function M.route_label(route: any): string?
	if type(route) ~= "table" then
		return nil
	end
	if route.source == "routed" then
		return locale.s("ds.threads.route.routed")
	elseif route.source == "chosen_by_agent" then
		return locale.s("ds.threads.route.chosen")
	elseif route.source == "fell_back" then
		local at = M.clock(route.resets_at)
		local why = if at then locale.s("ds.threads.route.reason_resets", { reason = route.short or "", at = at }) else route.short or ""
		return locale.s("ds.threads.route.fell_back", { reason = why })
	end
	return nil
end

local TINT = { routed = "accent", chosen_by_agent = "warning", fell_back = "info" }

function M.RouteBadge(props: { route: any, id: string? }): Node?
	local label = M.route_label(props.route)
	if not label then
		return nil
	end
	local tone = TINT[props.route.source] or "neutral"
	local role = props.route.role
	return ui.div {
		id = props.id,
		class = "route-badge",
		shrink = 0,
		min_w = 0,
		px = ui.raw(6, "route_badge.rs"),
		rounded = ui.raw(4, "route_badge.rs"),
		bg = `$zeron.tone.{tone}.wash`,
		label = label,
		tooltip = if role then `{label} · {role}` else label,
		ui.text { label, literal = true, wrap = false, text_size = ui.raw(10.5, "route_badge.rs"), color = `$zeron.tone.{tone}.ink` },
	}
end

local function route_key(route: any): string
	local label = M.route_label(route)
	return if label then `{route.source}:{label}:{route.role or ""}` else ""
end

local function read(v: any): any
	return if type(v) == "function" then v() else v
end

local function tile(mark: string?, icon: string?, seed: string?, failed: (() -> boolean)?): Node
	return ui.div {
		w = TILE,
		h = TILE,
		shrink = 0,
		rounded = ui.raw(5, "transcript.rs subagent tile"),
		bg = "$hover",
		items = "center",
		justify = "center",
		aria_hidden = true,
		color = function(): string
			return if failed and failed() then "$danger" else "$text.secondary"
		end,
		if mark or icon
			then AgentMark.Mark { label = seed or mark or "", mark = mark, icon = icon, seed = seed, size = 12, bare = true }
			else Icon { name = "bot", size = 12 },
	}
end

local function arrow(): Node
	return ui.div {
		class = "thread-open-arrow",
		w = TILE,
		h = TILE,
		shrink = 0,
		rounded = ui.raw(5, "transcript.rs open tile"),
		bg = "$hover",
		items = "center",
		justify = "center",
		color = "$text.secondary",
		aria_hidden = true,
		Icon { name = "arrow-up-right", size = 11 },
	}
end

local function card(props: { id: string, label: () -> string, open: (() -> boolean)?, on_open: (() -> ())?, header: { Node } }): Node
	local opens = props.on_open ~= nil
	local box: { [any]: any } = {
		id = props.id,
		class = "thread-card",
		w_full = true,
		h = CARD,
		min_w = 0,
		px = ui.raw(8, "transcript.rs CHIP_CARD_PAD_X"),
		gap = ui.raw(8, "transcript.rs chip header gap"),
		items = "center",
		overflow = "hidden",
		rounded = ui.raw(9, "transcript.rs agent_chip_card"),
		border = 1,
		border_color = function(): string
			return if props.open and props.open() then "$accent" else "$border"
		end,
		bg = "#00000000",
		text_size = ui.raw(13, "transcript.rs TOOL_LABEL_SIZE"),
		selectable = false,
		label = props.label,
	}
	for i, n in props.header do
		box[i] = n
	end
	-- (An edge's colour isn't something a test can find.)
	if props.open then
		table.insert(box, ui.show(props.open, ui.div { class = "thread-card-open", absolute = true, w = 0, h = 0, aria_hidden = true }))
	end
	if opens then
		box.cursor = "pointer"
		box.hover = { bg = "$hover" }
		box.active = { bg = "$hover" }
		box.on_click = props.on_open
		box.p = nil
		return ui.button(box)
	end
	return ui.row(box)
end

local SPAWN_STATE = { running = "ds.threads.state.running", done = "ds.threads.state.done", failed = "ds.threads.state.failed", stopped = "ds.threads.state.stopped" }

local function spawn_card(props: { now: () -> any, threads: Threads?, open_subagent: ((sub: any) -> ())?, agent: (() -> any)? }, sub: () -> any, look: () -> (string?, string?)): Node
	local function state(): string?
		local s = sub()
		return if s then s.status else nil
	end
	local function failed(): boolean
		return props.now().failed == true or state() == "failed"
	end
	local function title(): string
		local s = sub()
		return (if s then s.title else nil) or props.now().target or locale.s("ds.threads.subagent")
	end
	local s0 = sub() or {}
	local mark, icon = look()
	local threads = props.threads
	local on_open = if props.open_subagent and s0.ref
		then function()
			local s = sub()
			if s and props.open_subagent then
				props.open_subagent(s)
			end
		end
		else nil
	return card {
		id = `spawn-{props.now().id}`,
		label = function(): string
			if on_open then
				return locale.s("ds.activity.open_subagent", { title = title() })
			end
			return locale.s("ds.threads.spawn_label", { title = title() })
		end,
		open = function(): boolean
			local s = sub()
			return s ~= nil and threads ~= nil and threads.is_open ~= nil and threads.is_open(s.ref)
		end,
		on_open = on_open,
		header = nodes(
			tile(mark, icon, nil, failed),
			ui.text {
				locale.s("ds.threads.agent"),
				shrink = 0,
				wrap = false,
				weight = "$font.weight.medium",
				color = function(): string
					return if failed() then "$danger" else "$text.secondary"
				end,
			},
			FadeText {
				title,
				literal = true,
				grow = 1,
				basis = 0,
				min_w = 0,
				color = function(): string
					return if failed() then "$danger" else "$text.primary"
				end,
			},
			ui.show(function(): boolean
				return props.now().tag ~= nil
			end, ui.text {
				function(): string
					return props.now().tag or ""
				end,
				literal = true,
				shrink = 0,
				wrap = false,
				text_size = ui.raw(TAG_SIZE, "transcript.rs spawn tag"),
				color = "$text.tertiary",
			}),
			ui.text {
				function(): string
					local key = SPAWN_STATE[state() or ""]
					return if key then locale.s(key) else ""
				end,
				shrink = 0,
				wrap = false,
				text_size = ui.raw(TAG_SIZE, "transcript.rs delegate state"),
				color = function(): string
					return if failed() then "$danger" else "$text.tertiary"
				end,
			},
			ui.show(function(): boolean
				return state() == "running"
			end, ui.div { class = "thread-spinner", shrink = 0, aria_hidden = true, status.Spinner { size = 12, ink = "$text.secondary" } }),
			if on_open then arrow() else nil
		),
	}
end

function M.Spawn(props: { now: () -> any, threads: Threads?, open_subagent: ((sub: any) -> ())?, agent: (() -> any)? }): Node
	local function sub(): any
		return props.now().subagent
	end
	local function look(): (string?, string?)
		local s = sub()
		if s and s.agent then
			return s.agent, nil
		end
		local a = if props.agent then props.agent() else nil
		return if a then a.mark else nil, if a then a.icon else nil
	end
	-- (Built again as the engine binds it: it stamps the chat after the call starts.)
	return ui.presence(function(): string
		local s = sub()
		local mark, icon = look()
		return `{if s and s.ref and props.open_subagent then s.ref else ""}\31{mark or ""}\31{icon or ""}`
	end, function(): Node
		return spawn_card(props, sub, look)
	end, { w_full = true })
end

local DELEGATION_STATE = { running = "ds.threads.state.running", completed = "ds.threads.state.completed", failed = "ds.threads.state.failed", cancelled = "ds.threads.state.stopped", interrupted = "ds.threads.state.stopped" }

-- Zeron's `delegate_state_text`.
function M.delegate_state(c: Child?, limited: boolean): string
	if limited then
		return locale.s("ds.threads.state.limited")
	end
	local status_key = if c then DELEGATION_STATE[c.delegation or ""] else nil
	local word = if status_key then locale.s(status_key) else locale.s("ds.threads.state.unavailable")
	-- (Zeron's `delegate_view` counts to now when it has no end.)
	if c and c.delegation and c.delegation ~= "running" and c.delegated_at then
		return locale.s("ds.threads.state.after", { state = word, time = M.elapsed((c.delegated_end or os.time()) - c.delegated_at) })
	end
	return word
end

local function model_word(model: string?): string?
	if not model or model == "" then
		return nil
	end
	return string.match(model, "([^/]+)$") or model
end

export type DelegateProps = { id: string, ref: (string | () -> string?)?, title: (string | () -> string?)?, route: any?, native: boolean?, threads: Threads? }

local function delegate_card(props: DelegateProps, ref: () -> string?): Node
	local threads = props.threads
	local function child(): Child?
		local r = ref()
		return if r and threads then threads.child(r) else nil
	end
	local function limit(): any
		local r = ref()
		return if r and threads and threads.limit then threads.limit(r) else nil
	end
	local function failed(): boolean
		local c = child()
		return c ~= nil and c.delegation == "failed"
	end
	local function title(): string
		local c = child()
		local t = if c and c.title and c.title ~= "" then c.title else read(props.title)
		return t or locale.s("ds.threads.delegated_task")
	end
	local function agent(): any
		return if props.native and threads and threads.agent then threads.agent() else nil
	end
	local c0 = child()
	local harness = if c0 then c0.harness else nil
	local function name(): string
		if props.native then
			local a = agent()
			return locale.s("ds.threads.native", { agent = if a and a.label then a.label else locale.s("ds.threads.agent") })
		end
		local c = child()
		return M.harness_name(if c then c.harness else harness)
	end
	local function meta(): string?
		local c = child()
		local route = props.route or (if c then c.route else nil)
		local role = if type(route) == "table" then route.role else nil
		local a = agent()
		local model = if props.native then (if a then a.model else nil) else (if c then c.model else nil)
		local on = if props.native then (if a then a.harness else nil) else (if c then c.harness else nil)
		local said = if model and threads and threads.model_label then threads.model_label(on, model) else nil
		said = said or model_word(model)
		if role and said then
			return `{role} · {said}`
		end
		return role or said
	end
	local function state(): string
		return M.delegate_state(child(), limit() ~= nil)
	end
	local function running(): boolean
		local c = child()
		return c ~= nil and c.delegation == "running" and limit() == nil
	end
	local r0 = ref()
	local open = if not props.native and r0 and threads and threads.open_child
		then function()
			(threads.open_child :: (string) -> ())(r0 :: string)
		end
		else nil
	local route = props.route or (if c0 then c0.route else nil)
	local a0 = agent()
	local mark = if props.native then (if a0 then a0.mark else nil) else (if c0 then c0.mark or c0.agent else nil)
	local icon = if c0 then c0.agent_icon else nil
	return card {
		id = props.id,
		label = function(): string
			if props.native then
				return `{name()} {title()}`
			end
			return `{name()} {title()} {state()}`
		end,
		open = function(): boolean
			return r0 ~= nil and threads ~= nil and threads.is_open ~= nil and threads.is_open(r0)
		end,
		on_open = open,
		header = nodes(
			tile(mark, icon, harness, failed),
			ui.text {
				name,
				literal = true,
				shrink = 0,
				wrap = false,
				weight = "$font.weight.medium",
				color = function(): string
					return if failed() then "$danger" else "$text.secondary"
				end,
			},
			FadeText {
				title,
				literal = true,
				grow = 1,
				basis = 0,
				min_w = 0,
				color = function(): string
					return if failed() then "$danger" else "$text.primary"
				end,
			},
			ui.show(function(): boolean
				return meta() ~= nil
			end, ui.text {
				function(): string
					return meta() or ""
				end,
				literal = true,
				shrink = 0,
				wrap = false,
				text_size = ui.raw(TAG_SIZE, "transcript.rs delegate meta"),
				color = "$text.tertiary",
			}),
			M.RouteBadge { route = route, id = `{props.id}-route` },
			if props.native
				then nil
				else ui.text {
					state,
					literal = true,
					shrink = 0,
					wrap = false,
					text_size = ui.raw(TAG_SIZE, "transcript.rs delegate state"),
					color = function(): string
						if failed() then
							return "$danger"
						end
						return if limit() ~= nil then "$warning" else "$text.tertiary"
					end,
				},
			ui.show(running, ui.div { class = "thread-spinner", shrink = 0, aria_hidden = true, status.Spinner { size = 12, ink = "$text.secondary" } }),
			if open then arrow() else nil
		),
	}
end

function M.Delegate(props: DelegateProps): Node
	local threads = props.threads
	local function ref(): string?
		return read(props.ref)
	end
	return ui.presence(function(): string
		local r = ref()
		local c = if r and threads then threads.child(r) else nil
		local a = if props.native and threads and threads.agent then threads.agent() else nil
		local mark = if props.native then (if a then a.mark else nil) else (if c then c.mark or c.agent else nil)
		local route = props.route or (if c then c.route else nil)
		return `{r or ""}\31{if c then c.harness or "" else ""}\31{mark or ""}\31{route_key(route)}\31{if c then c.agent_icon or "" else ""}`
	end, function(): Node
		return delegate_card(props, ref)
	end, { w_full = true })
end

-- After the group's cards, as Zeron's transcript.rs puts them.
function M.LimitCards(props: { refs: () -> { string }, threads: Threads }): Node
	local threads = props.threads
	local held = ui.computed(function(): { string }
		local out: { string } = {}
		if not threads.limit then
			return out
		end
		for _, r in props.refs() do
			local l = threads.limit(r)
			if l then
				table.insert(out, `{r}\31{l.act_at or ""}`)
			end
		end
		return out
	end)
	return ui.each(held, function(key: string): Node
		local r = string.match(key, "^(.-)\31") or key
		return M.LimitCard {
			child = r,
			limit = function(): any
				return if threads.limit then threads.limit(r) else nil
			end,
			threads = threads,
		}
	end, {
		key = function(key: string): string
			return key
		end,
	})
end

local ACT = { switch = "ds.threads.limit.switch_now", wait = "ds.threads.limit.wait_now" }
local VERB = { switch = "ds.threads.limit.switching_in", wait = "ds.threads.limit.waiting_in" }

-- (By child: a pick outlives the card made again.)
local CHOSEN = ui.state("threads_limit_chosen", {} :: { [string]: string })

-- Zeron's `limit_card`.
function M.LimitCard(props: { child: string, limit: () -> any, threads: Threads }): Node
	local threads = props.threads
	local now = ui.use_state(ui.now())
	local span = ui.use_state(nil :: { at: number, len: number }?)
	local function l(): any
		return props.limit() or {}
	end
	local function left_ms(): number?
		local at = l().act_at
		if not at then
			return nil
		end
		local left = at - now:get()
		return if left > 0 then left else nil
	end
	local counting = ui.computed(function(): boolean
		local at = l().act_at
		return at ~= nil and at > ui.now()
	end)
	ui.effect(function()
		if counting:get() then
			local at = l().act_at
			local sp = span:peek()
			if not sp or sp.at ~= at then
				span:set({ at = at, len = math.max(1, at - ui.now()) })
			end
			now:set(ui.now())
			ui.every(1000, function(): boolean
				now:set(ui.now())
				return ui.now() < at
			end)
		end
	end)
	local function selected(): string
		if not l().to then
			return "wait"
		end
		return CHOSEN:get()[props.child] or l().preselect or "switch"
	end
	local function choose(choice: string)
		if l().act_at and choice ~= (l().preselect or "switch") and threads.limit_op then
			threads.limit_op(props.child, "hold")
		end
		local all = table.clone(CHOSEN:peek())
		all[props.child] = choice
		CHOSEN:set(all)
	end
	local function name(): string
		return l().harness or locale.s("ds.threads.the_agent")
	end
	local function reset_words(): string
		local at = M.clock(l().resets_at)
		if not at then
			return locale.s("ds.threads.limit.reset_unknown")
		end
		local ms = (l().resets_at or 0) - ui.now()
		if ms > 0 then
			return locale.s("ds.threads.limit.resets_in", { at = at, left = M.duration(ms) })
		end
		return locale.s("ds.threads.limit.resets", { at = at })
	end
	local function radio(on: () -> boolean): Node
		return ui.div {
			w = 14,
			h = 14,
			shrink = 0,
			mt = ui.raw(2, "limit_card.rs radio"),
			rounded = "$radius.full",
			border = 1,
			border_color = function(): string
				return if on() then "$zeron.tone.info.ink" else "$text.tertiary"
			end,
			items = "center",
			justify = "center",
			ui.show(on, ui.div { w = 6, h = 6, rounded = "$radius.full", bg = "$zeron.tone.info.ink" }),
		}
	end
	local function option(choice: string, lead: Node?, line: () -> string, sub: () -> string): Node
		local function on(): boolean
			return selected() == choice
		end
		return ui.button {
			id = `limit-{props.child}-{choice}`,
			role = "radio",
			checked = on,
			label = line,
			w_full = true,
			h = "auto" :: any,
			px = ui.raw(10, "limit_card.rs option"),
			py = ui.raw(7, "limit_card.rs option"),
			gap = ui.raw(8, "limit_card.rs option"),
			items = "start",
			rounded = ui.raw(8, "limit_card.rs option"),
			border = 1,
			border_color = function(): string
				return if on() then "$zeron.tone.info.edge" else "$border"
			end,
			bg = function(): string
				return if on() then "$zeron.tone.info.wash" else "#00000000"
			end,
			hover = { bg = "$hover" },
			cursor = "pointer",
			on_click = function()
				choose(choice)
			end,
			radio(on),
			ui.column {
				grow = 1,
				min_w = 0,
				gap = ui.raw(2, "limit_card.rs option"),
				ui.row {
					gap = ui.raw(5, "limit_card.rs option"),
					items = "start",
					min_w = 0,
					weight = "$font.weight.medium",
					color = "$text.primary",
					lead,
					ui.text { line, literal = true, wrap = true, grow = 1, min_w = 0 },
				},
				ui.text { sub, literal = true, wrap = true, text_size = ui.raw(11.5, "limit_card.rs"), color = "$text.secondary" },
			},
		}
	end
	local function act(op: string)
		if threads.limit_op then
			threads.limit_op(props.child, op)
		end
	end
	return ui.column {
		id = `limit-card-{props.child}`,
		class = "limit-card",
		role = "group",
		label = function(): string
			return locale.s("ds.threads.limit.title", { name = name() })
		end,
		w_full = true,
		mt = ui.raw(4, "limit_card.rs py"),
		mb = ui.raw(4, "limit_card.rs py"),
		p = ui.raw(12, "limit_card.rs"),
		gap = ui.raw(8, "limit_card.rs"),
		rounded = ui.raw(10, "limit_card.rs"),
		border = 1,
		border_color = "$zeron.tone.warning.edge",
		bg = "$zeron.tone.warning.wash",
		text_size = ui.raw(12.5, "limit_card.rs"),
		ui.row {
			gap = ui.raw(8, "limit_card.rs"),
			items = "center",
			w_full = true,
			ui.div { color = "$warning", shrink = 0, Icon { name = "clock-circle", size = 14 } },
			ui.text {
				function(): string
					return locale.s("ds.threads.limit.title", { name = name() })
				end,
				literal = true,
				grow = 1,
				min_w = 0,
				wrap = true,
				weight = "$font.weight.medium",
				color = "$text.primary",
			},
			ui.text { reset_words, literal = true, shrink = 0, wrap = false, text_size = ui.raw(11.5, "limit_card.rs"), color = "$text.tertiary" },
		},
		ui.show(
			function(): boolean
				return l().to == true
			end,
			option(
				"switch",
				ui.row {
					gap = ui.raw(5, "limit_card.rs"),
					items = "center",
					shrink = 0,
					ui.text { locale.s("ds.threads.limit.switch_to"), shrink = 0, wrap = false },
					ui.presence(function(): string?
						return l().to_mark
					end, function(m: string): Node
						return AgentMark.Mark { label = m, mark = m, size = 12, bare = true }
					end, { shrink = 0 }),
				},
				function(): string
					return locale.s("ds.threads.limit.keeps", { target = l().target or l().to_label or "" })
				end,
				function(): string
					return locale.s("ds.threads.limit.switch_sub", { name = name() })
				end
			)
		),
		option("wait", nil, function(): string
			local at = M.clock(l().resets_at)
			return if at then locale.s("ds.threads.limit.wait_for_at", { at = at }) else locale.s("ds.threads.limit.wait_for")
		end, function(): string
			return if l().auto then locale.s("ds.threads.limit.wait_auto", { name = name() }) else locale.s("ds.threads.limit.wait_unarmed")
		end),
		ui.row {
			gap = ui.raw(10, "limit_card.rs footer"),
			items = "center",
			w_full = true,
			ui.show(
				function(): boolean
					return left_ms() ~= nil
				end,
				ui.row {
					grow = 1,
					min_w = 0,
					gap = ui.raw(10, "limit_card.rs footer"),
					items = "center",
					ui.text {
						function(): string
							local ms = left_ms() or 0
							return locale.s(VERB[l().preselect or "switch"] or VERB.switch, { n = tostring(math.ceil(ms / 1000)) })
						end,
						id = `limit-{props.child}-countdown`,
						shrink = 0,
						wrap = false,
						tabular = true,
						text_size = ui.raw(11.5, "limit_card.rs"),
						color = "$text.secondary",
					},
					ui.div {
						grow = 1,
						h = 4,
						rounded = ui.raw(2, "limit_card.rs bar"),
						bg = "$hover",
						overflow = "hidden",
						ui.div {
							class = "limit-bar",
							h_full = true,
							rounded = ui.raw(2, "limit_card.rs bar"),
							bg = "$zeron.tone.info.ink",
							w = function(): any
								local sp = span:get()
								local ms = left_ms() or 0
								local f = if sp then math.clamp(ms / sp.len, 0, 1) else 0
								return `{math.floor(f * 1000) / 10}%`
							end,
						},
					},
					ui.button {
						id = `limit-{props.child}-cancel`,
						label = locale.t("ds.threads.limit.cancel"),
						h = 26,
						px = ui.raw(12, "limit_card.rs button"),
						rounded = "$radius.full",
						bg = "#00000000",
						hover = { bg = "$hover" },
						color = "$text.primary",
						on_click = function()
							act("hold")
						end,
						ui.text { locale.t("ds.threads.limit.cancel"), wrap = false, weight = "$font.weight.medium" },
					},
				},
				ui.div { grow = 1 }
			),
			ui.button {
				id = `limit-{props.child}-act`,
				label = function(): string
					return locale.s(ACT[selected()] or ACT.wait)
				end,
				h = 26,
				px = ui.raw(12, "limit_card.rs button"),
				rounded = "$radius.full",
				bg = "$text.primary",
				color = "$background",
				hover = { opacity = 0.9 },
				on_click = function()
					act(selected())
				end,
				ui.text {
					function(): string
						return locale.s(ACT[selected()] or ACT.wait)
					end,
					wrap = false,
					weight = "$font.weight.medium",
				},
			},
		},
	}
end

function M.Started(props: { id: string, ref: string, title: string?, threads: Threads? }): Node
	local threads = props.threads
	local function title(): string
		local c = if threads then threads.child(props.ref) else nil
		local t = if c and c.title and c.title ~= "" then c.title else props.title
		return if t and t ~= "" then t else locale.s("ds.threads.thread")
	end
	local c0 = if threads then threads.child(props.ref) else nil
	local row: { [any]: any } = {
		id = props.id,
		class = "started-thread",
		w_full = true,
		h = ui.raw(28, "transcript.rs started_thread_line row"),
		min_w = 0,
		gap = ui.raw(6, "transcript.rs started_thread_line"),
		items = "center",
		text_size = ui.raw(11, "transcript.rs started_thread_line"),
		color = "$text.secondary",
		selectable = false,
	}
	for i, n in nodes(
		ui.div { shrink = 0, color = "$text.tertiary", aria_hidden = true, Icon { name = "chat-round-line", size = 12 } },
		ui.text { locale.s("ds.threads.started_thread"), shrink = 0, wrap = false },
		FadeText { title, literal = true, shrink = 1, min_w = 0, weight = "$font.weight.medium", color = "$text.primary" },
		if c0 and c0.route then M.RouteBadge { route = c0.route, id = `{props.id}-route` } else nil,
		ui.text { "·", literal = true, shrink = 0, aria_hidden = true },
		ui.button {
			id = `{props.id}-open`,
			class = "started-thread-open",
			label = function(): string
				return locale.s("ds.threads.open_thread", { title = title() })
			end,
			h = "auto" :: any,
			p = 0,
			bg = "#00000000",
			hover = { bg = "#00000000", color = "$text.primary" },
			active = { bg = "#00000000" },
			color = "$text.primary",
			cursor = "pointer",
			shrink = 0,
			on_click = function()
				if threads and threads.open_child then
					threads.open_child(props.ref)
				end
			end,
			ui.text { locale.s("ds.threads.open"), wrap = false, weight = "$font.weight.medium" },
		},
		ui.div { grow = 1, min_w = 8, h = 1, ml = ui.raw(4, "transcript.rs hairline gap"), bg = "$border.subtle", aria_hidden = true }
	) do
		row[i] = n
	end
	return ui.row(row)
end

function M.Sent(props: { id: string, now: () -> any, threads: Threads? }): Node
	local threads = props.threads
	local function to(): any
		local key = props.now().to or ""
		return if threads then threads.chat(key) else nil
	end
	local function lead(): string
		local b = props.now()
		if b.failed then
			return locale.s("ds.threads.couldnt_message")
		end
		return locale.s(if b.pending then "ds.threads.sending_to" else "ds.threads.message_to")
	end
	local function title(): string
		local c = to()
		return if c then c.title else props.now().to or ""
	end
	return ui.row {
		id = props.id,
		class = "sent-message",
		w_full = true,
		min_w = 0,
		opacity = function(): number
			return if props.now().pending and not props.now().failed then 0.65 else 1
		end,
		ui.show(function(): boolean
			return props.now().pending == true and not props.now().failed
		end, ui.div { class = "sent-pending", absolute = true, w = 0, h = 0, aria_hidden = true }),
		ui.button {
			label = function(): string
				return `{lead()} {title()}`
			end,
			h = "auto" :: any,
			p = 0,
			max_w = "80%",
			min_w = 0,
			gap = ui.raw(6, "Zeron's caption gap"),
			items = "center",
			bg = "#00000000",
			hover = { bg = "#00000000", color = "$text.primary" },
			active = { bg = "#00000000" },
			text_size = "$font.size.md",
			color = function(): string
				return if props.now().failed then "$danger" else "$text.secondary"
			end,
			selectable = false,
			cursor = "pointer",
			on_click = function()
				local c = to()
				if c and threads and threads.open_chat then
					threads.open_chat(c.id)
				end
			end,
			ui.text { lead, literal = true, shrink = 0, wrap = false },
			ui.row {
				gap = ui.raw(5, "Zeron's caption mark-to-name gap"),
				items = "center",
				min_w = 0,
				ui.presence(function(): string
					local c = to()
					return if c and c.mark then c.mark else ""
				end, function(m: string): Node
					return if m ~= "" then AgentMark.Mark { label = m, mark = m, size = 14, bare = true } else Icon { name = "chat-round-line", size = 14 }
				end, { shrink = 0, aria_hidden = true }),
				ui.text { title, literal = true, truncate = true, weight = "$font.weight.medium" },
			},
		},
	}
end

-- Zeron's `render_wake_row`.
function M.Wake(props: { id: string, ids: { string }, threads: Threads? }): Node
	local threads = props.threads
	local function kids(): { Child }
		local out: { Child } = {}
		for _, id in props.ids do
			local c = if threads then threads.child(id) else nil
			table.insert(out, c or ({ id = id } :: Child))
		end
		return out
	end
	local function failed(): boolean
		for _, c in kids() do
			if c.delegation == "failed" then
				return true
			end
		end
		return false
	end
	local function summary(): string
		local list = kids()
		local outcome = "finished"
		for _, c in list do
			if c.delegation == "failed" then
				outcome = "failed"
				break
			elseif c.delegation == "cancelled" or c.delegation == "interrupted" then
				outcome = "stopped"
			end
		end
		local word = locale.s(`ds.threads.wake.{outcome}`)
		if #list == 1 then
			return locale.s("ds.threads.wake.one", { title = list[1].title or locale.s("ds.threads.delegated_task"), outcome = word })
		end
		local titles: { string } = {}
		for _, c in list do
			table.insert(titles, c.title or locale.s("ds.threads.delegated_task"))
		end
		return locale.s("ds.threads.wake.many", { n = tostring(#list), outcome = word, titles = table.concat(titles, ", ") })
	end
	local first = kids()[1]
	local mark = if first then first.mark or first.agent else nil
	for _, c in kids() do
		if (c.mark or c.agent) ~= mark then
			mark = nil
		end
	end
	local row: { [any]: any } = {
		id = props.id,
		class = "delegate-wake",
		w_full = true,
		min_w = 0,
		gap = ui.raw(6, "transcript.rs render_wake_row"),
		items = "center",
		pb = ui.raw(8, "transcript.rs render_wake_row"),
		text_size = ui.raw(11, "transcript.rs render_wake_row"),
		color = function(): string
			return if failed() then "$danger" else "$text.secondary"
		end,
		selectable = false,
	}
	for i, n in nodes(
		ui.div {
			shrink = 0,
			color = "$text.tertiary",
			aria_hidden = true,
			if mark then AgentMark.Mark { label = mark, mark = mark, size = 12, bare = true } else Icon { name = "bot", size = 12 },
		},
		FadeText { summary, literal = true, shrink = 1, min_w = 0, weight = "$font.weight.medium" },
		if #props.ids == 1
			then ui.button {
				id = `{props.id}-open`,
				class = "delegate-wake-open",
				label = locale.t("ds.threads.open_subagent"),
				h = "auto" :: any,
				px = ui.raw(6, "transcript.rs render_wake_row"),
				py = 0,
				rounded = ui.raw(5, "transcript.rs render_wake_row"),
				bg = "$hover",
				hover = { color = "$text.primary" },
				color = "$text.primary",
				cursor = "pointer",
				shrink = 0,
				on_click = function()
					if threads and threads.open_child then
						threads.open_child(props.ids[1])
					end
				end,
				ui.text { locale.t("ds.threads.open_subagent"), wrap = false },
			}
			else nil,
		ui.div { grow = 1, min_w = 8, h = 1, ml = ui.raw(4, "transcript.rs hairline gap"), bg = "$border.subtle", aria_hidden = true }
	) do
		row[i] = n
	end
	return ui.row(row)
end

function M.From(props: { id: string, senders: { string }, clock: string?, threads: Threads? }): Node
	local threads = props.threads
	local names: { Node } = {}
	for i, from in props.senders do
		local function c(): any
			return if threads then threads.child(from) else nil
		end
		local function title(): string
			local x = c()
			local t = if x and x.title and x.title ~= "" then x.title else from
			return if i < #props.senders then `{t},` else t
		end
		local c0 = c()
		local mark = if c0 then c0.mark or c0.agent else nil
		table.insert(
			names,
			ui.button {
				id = `{props.id}-from-{from}`,
				label = function(): string
					return locale.s("ds.threads.from_label", { title = title() })
				end,
				h = "auto" :: any,
				p = 0,
				gap = ui.raw(6, "transcript.rs from row"),
				items = "center",
				min_w = 0,
				bg = "#00000000",
				hover = { bg = "#00000000", color = "$text.primary" },
				active = { bg = "#00000000" },
				color = "$text.primary",
				cursor = "pointer",
				on_click = function()
					if threads and threads.open_chat then
						threads.open_chat(from)
					end
				end,
				if mark then ui.div { shrink = 0, aria_hidden = true, AgentMark.Mark { label = mark, mark = mark, size = 12, bare = true } } else nil,
				ui.text { title, literal = true, truncate = true, weight = "$font.weight.medium" },
			}
		)
	end
	local row: { [any]: any } = {
		class = "peer-from",
		w_full = true,
		min_w = 0,
		gap = ui.raw(6, "transcript.rs from row"),
		items = "center",
		pb = ui.raw(8, "transcript.rs from row"),
		text_size = ui.raw(11, "transcript.rs from row"),
		color = "$text.secondary",
		selectable = false,
	}
	local parts = nodes(
		ui.div { shrink = 0, color = "$text.tertiary", aria_hidden = true, Icon { name = "chat-round-line", size = 12 } },
		ui.text { locale.s("ds.threads.from"), shrink = 0, wrap = false }
	)
	for _, n in names do
		table.insert(parts, n)
	end
	if props.clock then
		table.insert(parts, ui.text { props.clock, literal = true, shrink = 0, wrap = false, font = "$font.mono", text_size = ui.raw(10, "transcript.rs from clock"), color = "$text.tertiary" })
	end
	table.insert(parts, ui.div { grow = 1, min_w = 8, h = 1, ml = ui.raw(4, "transcript.rs hairline gap"), bg = "$border.subtle", aria_hidden = true })
	for i, n in parts do
		row[i] = n
	end
	return ui.row(row)
end

local CARD_EASE = "cubicBezier(0.2, 0.8, 0.2, 1)"
local STAGGER_MAX = 12
local BADGE: { [string]: { ink: string, wash: string } } = {
	done = { ink = "$success", wash = "$zeron.peer_card.done" },
	failed = { ink = "$danger", wash = "$zeron.peer_card.failed" },
	stopped = { ink = "$text.secondary", wash = "$zeron.peer_card.stopped" },
}

export type CardProps = {
	id: string,
	from: string,
	threads: Threads?,
	ids: () -> { string },
	text: (id: string) -> string,
	clock: (id: string) -> string?,
	open: State<boolean>,
	fresh: () -> boolean,
	woken: () -> boolean,
	found: (() -> boolean)?,
	body: (id: string) -> Node,
	lane: (id: string, hovered: () -> boolean) -> Node,
	hovered: () -> boolean,
}

local function flat(s: string): string
	return (string.gsub(string.gsub(s, "%s+", " "), "^%s*(.-)%s*$", "%1"))
end

local function live_dot(): Node
	return ui.div {
		class = "peer-card-live",
		w = 6,
		h = 6,
		shrink = 0,
		relative = true,
		aria_hidden = true,
		if motion.reduced_now()
			then nil
			else ui.div {
				absolute = true,
				left = 0,
				top = 0,
				w = 6,
				h = 6,
				rounded = "$radius.full",
				bg = "$accent",
				scale = ui.loop { from = 1, to = 3, duration = 1400, yoyo = false, fps = 30 },
				opacity = ui.loop { from = 0.55, to = 0, duration = 1400, yoyo = false, fps = 30 },
			},
		ui.div { absolute = true, left = 0, top = 0, w = 6, h = 6, rounded = "$radius.full", bg = "$accent" },
	}
end

local function badge(kind: string, pop: boolean): Node
	local tone = BADGE[kind]
	local reduced = motion.reduced_now()
	return ui.row {
		class = `peer-card-{kind}`,
		h = 16,
		px = ui.raw(6, "the mockup's peer card"),
		gap = ui.raw(3, "the mockup's peer card"),
		rounded = "$radius.lg",
		shrink = 0,
		items = "center",
		bg = tone.wash,
		color = tone.ink,
		opacity = if pop then 0 else 1,
		on_mount = if pop
			then function(el)
				anime.animate(el, if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else {
					opacity = { 0, 1 },
					scale = { { from = 0.4, to = 1.12, duration = 204, ease = "out(2)" }, { to = 1, duration = 136, ease = "inOut(2)" } },
					duration = 204,
				})
			end
			else nil,
		if kind == "done"
			then ui.div {
				shrink = 0,
				aria_hidden = true,
				opacity = if pop then 0 else 1,
				on_mount = if pop
					then function(el)
						anime.animate(el, if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, scale = { 0.3, 1 }, delay = 180, duration = 260, ease = "out(2)" })
					end
					else nil,
				Icon { name = "check", size = 9 },
			}
			else nil,
		ui.text { locale.s(`ds.threads.state.{kind}`), wrap = false, text_size = ui.raw(10, "the mockup's peer card"), weight = "$font.weight.semibold" },
	}
end

function M.Card(props: CardProps): Node
	local threads = props.threads
	local open = props.open
	local function child(): Child?
		return if threads then threads.child(props.from) else nil
	end
	local function title(): string
		local c = child()
		return if c and c.title and c.title ~= "" then c.title else props.from
	end
	local function model(): string
		local c = child()
		if not c or not c.model then
			return ""
		end
		return (if threads and threads.model_label then threads.model_label(c.harness, c.model) else nil) or c.model
	end
	local function standing(): string?
		local c = child()
		if not c then
			return nil
		end
		local d = c.delegation
		if d == "failed" then
			return "failed"
		elseif d == "cancelled" or d == "interrupted" then
			return "stopped"
		elseif d == "completed" or props.woken() then
			return "done"
		end
		return if c.status == "working" then "working" else nil
	end
	local function newest(): string
		local ids = props.ids()
		return ids[#ids] or props.id
	end
	local c0 = child()
	local mark = if c0 then c0.mark or c0.agent else nil
	local mounted = false
	local opened_at = -1e9
	local turn = ui.use_state(if open:peek() then 1 else 0)
	local natural = ui.use_state(0)
	local flash = ui.use_state(0)
	local sweep = ui.use_state(0)
	local card_w = ui.use_state(0)
	local over = ui.use_state(nil :: string?)
	local turning: any = nil
	local function set_open(on: boolean)
		if open:peek() == on then
			return
		end
		if on then
			opened_at = ui.now()
		end
		open:set(on)
		if turning then
			turning:pause()
			turning = nil
		end
		if motion.reduced_now() then
			turn:set(if on then 1 else 0)
			return
		end
		turning = anime.animate(turn, { value = { turn:peek(), if on then 1 else 0 }, duration = 220, ease = CARD_EASE })
	end
	local function flash_border()
		if motion.reduced_now() then
			return
		end
		flash:set(1)
		anime.animate(flash, { value = { 1, 0 }, delay = 350, duration = 650, ease = "out(2)" })
	end
	if props.found then
		local found = props.found
		ui.effect(function()
			if found() then
				set_open(true)
			end
		end)
	end
	local seen: string? = nil
	ui.effect(function()
		local id = newest()
		local was = seen
		seen = id
		if was == nil or was == id or not mounted then
			return
		end
		flash_border()
		if not open:peek() and not motion.reduced_now() then
			sweep:set(0)
			anime.animate(sweep, { value = { 0, 1 }, duration = 750, ease = "out(2)" })
		end
	end)
	local reduced = motion.reduced_now()
	local roll_in = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, y = { 12, 0 }, duration = 260, ease = CARD_EASE }
	local roll_out = if reduced then { opacity = 0, duration = motion.FADE } else { opacity = 0, y = -12, duration = 260, ease = CARD_EASE }
	local items = ui.computed(function(): { { id: string, k: number } }
		local out = {}
		for k, id in props.ids() do
			table.insert(out, { id = id, k = k })
		end
		return out
	end)

	local function item(it: { id: string, k: number }): Node
		local id, k = it.id, it.k
		local function last(): boolean
			return newest() == id
		end
		local staggered = ui.now() - opened_at < 100 and #props.ids() <= STAGGER_MAX
		local arriving = mounted and ui.now() - opened_at >= 100 and last()
		local moves = staggered or arriving
		return ui.column {
			id = `{props.id}-item-{id}`,
			class = "peer-card-item",
			relative = true,
			w_full = true,
			min_w = 0,
			pt = "$space.2",
			pl = ui.raw(30, "the mockup's peer card"),
			pr = "$space.3",
			opacity = if moves then 0 else 1,
			on_mount = if moves
				then function(el)
					anime.animate(el, if motion.reduced_now() then { opacity = { 0, 1 }, duration = motion.FADE } elseif staggered then { opacity = { 0, 1 }, y = { 6, 0 }, duration = 240, delay = 40 * (k - 1), ease = CARD_EASE } else { opacity = { 0, 1 }, y = { 6, 0 }, duration = 200, ease = CARD_EASE })
				end
				else nil,
			on_hover = function(inside: boolean)
				if inside then
					over:set(id)
				elseif over:peek() == id then
					over:set(nil)
				end
			end,
			on_focus = function()
				over:set(id)
			end,
			ui.div { absolute = true, left = 0, right = 0, top = 0, h = 1, bg = "$zeron.peer_card.rule", aria_hidden = true },
			ui.div {
				absolute = true,
				left = 11,
				top = 11,
				w = 8,
				h = 8,
				rounded = "$radius.full",
				border = 1.5,
				border_color = function(): string
					return if last() then "$accent" else "$text.tertiary"
				end,
				bg = function(): string
					return if last() then "$accent" else "#00000000"
				end,
				aria_hidden = true,
			},
			ui.row {
				w_full = true,
				min_w = 0,
				items = "start",
				gap = "$space.2",
				-- (The prose sets its own ink; an older one is dimmed toward the secondary.)
				ui.column {
					grow = 1,
					min_w = 0,
					opacity = function(): number
						return if last() then 1 else 0.72
					end,
					props.body(id),
				},
				ui.text {
					function(): string
						return props.clock(id) or ""
					end,
					literal = true,
					shrink = 0,
					wrap = false,
					pt = ui.raw(3, "the mockup's peer card"),
					font = "$font.mono",
					text_size = ui.raw(10, "the mockup's peer card"),
					color = "$text.tertiary",
				},
			},
			props.lane(id, function(): boolean
				return props.hovered() and over:get() == id
			end),
		}
	end

	local header = ui.button {
		id = `{props.id}-head`,
		class = "peer-card-head",
		label = function(): string
			return locale.s("ds.threads.card.toggle", { title = title() })
		end,
		expanded = function(): boolean
			return open:get()
		end,
		selectable = false,
		w_full = true,
		min_w = 0,
		h = 32,
		p = 0,
		pl = ui.raw(11, "the mockup's peer card"),
		pr = ui.raw(10, "the mockup's peer card"),
		gap = ui.raw(7, "the mockup's peer card"),
		items = "center",
		rounded = 0,
		bg = "#00000000",
		hover = { bg = "$hover" },
		active = { bg = "$hover" },
		text_size = "$font.size.xs",
		color = "$text.secondary",
		cursor = "pointer",
		on_click = function()
			set_open(not open:peek())
		end,
		on_press = function() end,
		on_release = function() end,
		ui.div {
			shrink = 0,
			aria_hidden = true,
			color = "$text.tertiary",
			rotate = motion.tween(ui.computed(function(): number
				return if open:get() then 90 else 0
			end), { duration = 220, ease = CARD_EASE }),
			Icon { name = "chevron-right", size = 10 },
		},
		if mark then ui.div { shrink = 0, aria_hidden = true, AgentMark.Mark { label = mark, mark = mark, size = 12, bare = true } } else nil,
		ui.button {
			id = `{props.id}-from-{props.from}`,
			label = function(): string
				return locale.s("ds.threads.from_label", { title = title() })
			end,
			h = "auto" :: any,
			p = 0,
			min_w = 0,
			shrink = 1,
			bg = "#00000000",
			hover = { bg = "#00000000" },
			active = { bg = "#00000000" },
			color = "$text.primary",
			cursor = "pointer",
			on_click = function()
				if threads and threads.open_chat then
					threads.open_chat(props.from)
				end
			end,
			ui.text { title, literal = true, truncate = true, wrap = false, weight = "$font.weight.medium" },
		},
		ui.text { model, literal = true, shrink = 0, wrap = false, color = "$text.tertiary" },
		ui.presence(standing, function(kind: string): Node
			if kind == "working" then
				return live_dot()
			end
			return badge(kind, mounted)
		end, { shrink = 0 }),
		ui.div { grow = 1, min_w = 0 },
		ui.presence(function(): string?
			local n = #props.ids()
			return if n > 1 then tostring(n) else nil
		end, function(n: string): Node
			return ui.text { locale.s("ds.threads.card.messages", { n = tonumber(n) }), wrap = false, color = "$text.tertiary" }
		end, { shrink = 0, enter = roll_in, exit = roll_out }),
		ui.text {
			function(): string
				return props.clock(newest()) or ""
			end,
			literal = true,
			shrink = 0,
			wrap = false,
			font = "$font.mono",
			text_size = ui.raw(10, "the mockup's peer card"),
			color = "$text.tertiary",
		},
	}

	local card = ui.column {
		id = props.id,
		class = "peer-card",
		relative = true,
		w_full = true,
		min_w = 0,
		rounded = "$radius.xl",
		border = 1,
		border_color = "$border",
		bg = "$zeron.peer_card.fill",
		overflow = "hidden",
		on_resize = function(size: { w: number, h: number })
			card_w:set(size.w)
		end,
		header,
		ui.show(
			function(): boolean
				return not open:get()
			end,
			ui.column {
				w_full = true,
				min_w = 0,
				pl = ui.raw(30, "the mockup's peer card"),
				pr = "$space.3",
				pb = ui.raw(10, "the mockup's peer card"),
				ui.presence(newest, function(id: string): Node
					return ui.text {
						function(): string
							return flat(props.text(id))
						end,
						class = "peer-card-preview",
						literal = true,
						wrap = true,
						lines = 2,
						truncate = true,
						w_full = true,
						min_w = 0,
						text_size = "$font.size.md",
						color = "$text.secondary",
					}
				end, {
					w_full = true,
					enter = if reduced then roll_in else { opacity = { 0, 1 }, y = { 8, 0 }, delay = 90, duration = 260, ease = CARD_EASE },
					exit = if reduced then roll_out else { opacity = 0, y = -10, duration = 200, ease = "in(2)" },
				}),
			}
		),
		ui.show(
			function(): boolean
				return open:get() or turn:get() > 0
			end,
			ui.column {
				w_full = true,
				min_w = 0,
				overflow = function(): string
					return if turn:get() < 1 then "hidden" else "visible"
				end,
				max_h = function(): number
					local t = turn:get()
					return if t < 1 then t * natural:get() else 1e6
				end,
				ui.column {
					w_full = true,
					min_w = 0,
					shrink = 0,
					on_resize = function(size: { w: number, h: number })
						natural:set(size.h)
					end,
					ui.each(items, item, {
						key = function(it: { id: string, k: number }): string
							return it.id
						end,
						w_full = true,
					}),
				},
			}
		),
		ui.div {
			absolute = true,
			top = 0,
			bottom = 0,
			w = function(): number
				return card_w:get() * 0.45
			end,
			left = function(): number
				local band = card_w:get() * 0.45
				return -band + sweep:get() * (card_w:get() + band)
			end,
			opacity = function(): number
				local s = sweep:get()
				return if s <= 0 or s >= 1 then 0 else math.sin(s * math.pi)
			end,
			bg = "$zeron.peer_card.sweep",
			rounded = "$radius.full",
			pointer_events = "none",
			aria_hidden = true,
		},
	}

	return ui.column {
		relative = true,
		w_full = true,
		min_w = 0,
		on_mount = function(el)
			mounted = true
			if not props.fresh() then
				return
			end
			if motion.reduced_now() then
				anime.animate(el, { opacity = { 0, 1 }, duration = motion.FADE })
				return
			end
			anime.animate(el, { opacity = { 0, 1 }, y = { 12, 0 }, scale = { 0.98, 1 }, duration = 280, ease = CARD_EASE })
			flash_border()
		end,
		card,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = "$radius.xl",
			border = 1,
			border_color = "$accent",
			outline = 3,
			outline_color = "$zeron.peer_card.ring",
			pointer_events = "none",
			aria_hidden = true,
			opacity = function(): number
				return flash:get()
			end,
		},
	}
end

function M.AutoContinue(): Node
	return ui.row {
		class = "auto-continue",
		w_full = true,
		justify = "end",
		gap = ui.raw(5, "transcript.rs auto continue"),
		items = "center",
		pb = ui.raw(4, "transcript.rs auto continue"),
		text_size = ui.raw(11, "transcript.rs auto continue"),
		color = "$text.secondary",
		selectable = false,
		ui.div { color = "$text.tertiary", aria_hidden = true, Icon { name = "refresh", size = 11 } },
		ui.text { locale.s("ds.threads.auto_continue"), wrap = false },
	}
end

function M.is_plan_write(t: any): boolean
	if t.plan == "write" then
		return true
	end
	if t.kind ~= "mcp" or type(t.target) ~= "string" then
		return false
	end
	local name = if t.target == "zeron.plan" then "plan" else string.match(t.target, "zeron__(.-)$")
	return name == "plan" and type(t.call) == "string" and string.find(t.call, '"action":%s*"write"') ~= nil
end

local function holds(b: any, id: string): boolean
	for _, t in b.tools or {} do
		if t.id == id then
			return true
		end
	end
	for _, inner in b.blocks or {} do
		if holds(inner, id) then
			return true
		end
	end
	return false
end
M.holds = holds

local function newest_in(blocks: { any }): string?
	for i = #blocks, 1, -1 do
		local b = blocks[i]
		local inner = if b.blocks then newest_in(b.blocks) else nil
		if inner then
			return inner
		end
		local tools = b.tools or {}
		for j = #tools, 1, -1 do
			if M.is_plan_write(tools[j]) then
				return tools[j].id
			end
		end
	end
	return nil
end

function M.newest_plan(messages: { any }): string?
	for i = #messages, 1, -1 do
		local m = messages[i]
		local found = newest_in(m.blocks or { { tools = m.tools } })
		if found then
			return found
		end
	end
	return nil
end

local MARK = 16
local MOVE_MS = 220

function M.Roll(props: { value: () -> string, id: string?, font: string?, text_size: any?, color: any?, weight: any? }): Node
	local reduced = motion.reduced_now()
	return ui.div {
		id = props.id,
		shrink = 0,
		overflow = "hidden",
		ui.presence(props.value, function(v: string): Node
			return ui.text { v, literal = true, wrap = false, tabular = true, font = props.font, text_size = props.text_size, color = props.color, weight = props.weight }
		end, {
			enter = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, y = { 10, 0 }, duration = 260, ease = CARD_EASE },
			exit = if reduced then { opacity = 0, duration = motion.FADE } else { opacity = 0, y = -10, duration = 260, ease = CARD_EASE },
		}),
	}
end

function M.Chip(props: { text: string | () -> string, id: string? }): Node
	return ui.row {
		id = props.id,
		class = "step-chip",
		shrink = 0,
		h = ui.raw(16, "the mockup's chip"),
		px = ui.raw(6, "the mockup's chip"),
		items = "center",
		rounded = ui.raw(8, "the mockup's chip"),
		bg = "$zeron.tone.warning.wash",
		ui.text { props.text, literal = true, wrap = false, text_size = ui.raw(10, "the mockup's chip"), weight = "$font.weight.semibold", color = "$zeron.tone.warning.ink" },
	}
end

function M.quiet(s: any): boolean
	return s.state == "working" and type(s.quiet_since) == "number"
end

local function quiet_minutes(since: number): number
	return math.max(1, math.floor(status.since(since) / 60))
end

function M.chips(s: any): { { k: string, text: string } }
	local out = {}
	if type(s.attempt) == "number" and s.attempt > 1 then
		table.insert(out, { k = "attempt", text = locale.s("ds.threads.chip.attempt", { n = s.attempt }) })
	end
	if s.state == "working" and type(s.background) == "table" and #s.background > 0 then
		table.insert(out, { k = "background", text = locale.s("ds.threads.chip.background") })
	end
	if M.quiet(s) then
		table.insert(out, { k = "quiet", text = locale.s("ds.threads.chip.quiet", { n = quiet_minutes(s.quiet_since) }) })
	end
	return out
end

function M.Chips(props: { chips: () -> { { k: string, text: string } } }): Node
	local reduced = motion.reduced_now()
	local list = ui.computed(props.chips)
	return ui.each(list, function(c: any): Node
		local k, last = c.k, c.text
		return M.Chip {
			text = function(): string
				for _, x in list:get() do
					if x.k == k then
						last = x.text
					end
				end
				return last
			end,
		}
	end, {
		key = function(c: any): string
			return c.k
		end,
		gap = ui.raw(4, "the mockup's chips"),
		shrink = 0,
		items = "center",
		enter = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, scale = { 0.6, 1 }, duration = MOVE_MS, ease = CARD_EASE },
		exit = { opacity = 0, duration = motion.FADE },
	})
end

function M.facts(turn: any): string?
	if type(turn) ~= "table" or type(turn.ms) ~= "number" then
		return nil
	end
	return locale.s("ds.threads.line.turn", { time = M.elapsed(turn.ms / 1000), tools = turn.tools or 0, failed = turn.failed or 0 })
end

function M.working_line(s: any): string?
	if s.state ~= "working" then
		return nil
	end
	if type(s.background) == "table" and #s.background > 0 then
		return locale.s("ds.threads.line.background", { what = table.concat(s.background, ", ") })
	end
	if type(s.quiet_since) == "number" then
		return locale.s("ds.threads.line.quiet", { n = quiet_minutes(s.quiet_since) })
	end
	if type(s.activity) == "string" and s.activity ~= "" then
		return s.activity
	end
	return nil
end

local function next_line(l: string): boolean
	return string.match(l, "^Read it with read_thread") ~= nil or string.match(l, "^Read them with read_thread") ~= nil
end

local STEP_LINE = '^Step "(.*)" %(%a*[Ss]tepId [^)]*%) (.-) on its thread (%S+)%.$'
local TASK_LINE = '^Delegated task %S+ %(".*"%) .-%.$'
local ENDED_AS = { ["finished a turn"] = "finished", failed = "failed", ["was cancelled"] = "cancelled", ["was interrupted"] = "interrupted" }

local function step_status(title: string, what: string): string
	return locale.s("ds.threads.handback", { title = (string.gsub(title, '\\"', '"')), state = ENDED_AS[what] or "other" })
end

-- (The engine parses the stored text: only what a person reads loses its lines.)
function M.handback_text(text: string, thread: string?): string
	local paras: { { string } } = {}
	local cur: { string } = {}
	for line in string.gmatch(text .. "\n", "(.-)\r?\n") do
		if string.match(line, "^%s*$") then
			if #cur > 0 then
				table.insert(paras, cur)
				cur = {}
			end
		else
			table.insert(cur, line)
		end
	end
	if #cur > 0 then
		table.insert(paras, cur)
	end
	while #paras > 0 and next_line(paras[#paras][1]) do
		table.remove(paras)
	end
	local out: { string } = {}
	local owner: string? = nil
	for _, p in paras do
		local kept: { string } = {}
		local machine = false
		for _, l in p do
			local title, what, of = string.match(l, STEP_LINE)
			if of then
				machine = true
				owner = of
				if thread == nil then
					table.insert(kept, step_status(title :: string, what :: string))
				end
			elseif string.match(l, TASK_LINE) then
				machine = false
				owner = ""
				table.insert(kept, l)
			elseif not (machine and string.match(l, "^Turn: .+%.$")) then
				table.insert(kept, l)
			end
		end
		if #kept > 0 and (thread == nil or owner == nil or owner == thread) then
			table.insert(out, table.concat(kept, "\n"))
		end
	end
	return table.concat(out, "\n\n")
end

local function first_lines(text: string, n: number): string
	local out: { string } = {}
	for line in string.gmatch(text .. "\n", "(.-)\n") do
		if #out == n then
			out[n] ..= " …"
			break
		end
		table.insert(out, line)
	end
	return table.concat(out, "\n")
end

-- (`attempt`: set when the text is an earlier attempt's, the current one having none yet.)
export type Handback = { id: string, text: string, thread: string, attempt: number? }

function M.step_handbacks(messages: { any }, card: any): { [string]: Handback }
	local of: { [string]: { step: string, attempt: number? } } = {}
	for _, s in card.steps do
		if s.chat then
			of[s.chat] = { step = s.id }
		end
		for i, e in (if type(s.earlier) == "table" then s.earlier else {}) do
			if type(e) == "table" and e.id and not of[e.id] then
				of[e.id] = { step = s.id, attempt = i }
			end
		end
	end
	local out: { [string]: Handback } = {}
	for _, x in messages do
		if x.role == "user" and x.from_chat and x.continued_on == nil and x.steps then
			for _, thread in x.steps do
				local o = of[thread]
				local had = if o then out[o.step] else nil
				if o and (o.attempt == nil or had == nil or had.attempt ~= nil) then
					out[o.step] = { id = x.id, text = x.text or "", thread = thread, attempt = o.attempt }
				end
			end
		end
	end
	return out
end

function M.step_said(s: any, hb: Handback?): string?
	if not hb or (s.state == "working" and hb.attempt == nil) then
		return nil
	end
	local text = M.handback_text(hb.text, hb.thread)
	return if text ~= "" then first_lines(text, 6) else nil
end

local function mark_kind(s: any): string
	local st = s.state
	if M.quiet(s) then
		return "quiet"
	elseif st == "readyForReview" then
		return "review"
	end
	return st or "planned"
end

local CHECK_INK = { review = "$text.secondary", done = "$success" }

local Check = ui.component(function(props: { ink: string, draw: boolean }): Node
	local drawn = ui.use_state(if props.draw then 0 else 1)
	if props.draw then
		anime.animate(drawn, { value = { 0, 1 }, delay = 120, duration = 260, ease = "out(2)" })
	end
	return ui.canvas {
		w = MARK,
		h = MARK,
		shrink = 0,
		aria_hidden = true,
		draw = function(c: any, w: number, h: number)
			local t = drawn:get()
			if t <= 0 then
				return
			end
			local k = math.min(w, h) / MARK
			c:stroke(ui.path():move(4 * k, 8.4 * k):line(6.6 * k, 11 * k):line(12 * k, 5.6 * k), props.ink, 1.8, { cap = "round", join = "round", draw = t })
		end,
	}
end)

local function mark_of(kind: string, draw: boolean): Node
	if kind == "working" then
		return status.StatusIcon { kind = "working", size = MARK }
	elseif kind == "quiet" then
		return status.Spinner { size = MARK, ink = "$zeron.tone.warning.ink" }
	elseif CHECK_INK[kind] then
		return Check { ink = CHECK_INK[kind], draw = draw and not motion.reduced_now() }
	elseif kind == "failed" then
		return status.StatusIcon { kind = "failed", size = MARK }
	elseif kind == "waitingOnYou" then
		return ui.div { shrink = 0, color = "$warning", Icon { name = "clock-circle", size = MARK } }
	elseif kind == "cancelled" then
		return ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "close-circle", size = MARK } }
	elseif kind == "skipped" then
		return ui.div { shrink = 0, color = "$text.tertiary", Icon { name = "status-done", size = MARK } }
	end
	return ui.div {
		w = MARK,
		h = MARK,
		shrink = 0,
		items = "center",
		justify = "center",
		opacity = 0.7,
		ui.div { w = ui.raw(11, "the mockup's dashed ring"), h = ui.raw(11, "the mockup's dashed ring"), rounded = "$radius.full", border = 1.5, border_color = "$text.tertiary", border_dash = { 2, 2 } },
	}
end

function M.StepMark(props: { kind: () -> string, class: ((kind: string) -> string)? }): Node
	local reduced = motion.reduced_now()
	local first = true
	return ui.presence(props.kind, function(kind: string): Node
		local draw = not first
		first = false
		return ui.div {
			class = if props.class then props.class(kind) else `step-mark step-mark-{kind}`,
			w = MARK,
			h = MARK,
			shrink = 0,
			items = "center",
			justify = "center",
			mark_of(kind, draw),
		}
	end, {
		w = MARK,
		h = MARK,
		shrink = 0,
		enter = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { from = 0, to = 1, duration = 160 }, scale = { 0.6, 1 }, duration = MOVE_MS, ease = CARD_EASE },
		exit = { opacity = 0, duration = if reduced then motion.FADE else 160 },
	})
end

export type PlanProps = {
	id: string,
	threads: Threads,
	card: (() -> any)?,
	handback: ((step: string) -> Handback?)?,
	body: ((text: string, key: string?) -> Node)?,
	found: ((key: string) -> boolean)?,
}

local RESULTS = ui.state("plan_card_results", {} :: { [string]: boolean })
local ENDED = { readyForReview = true, done = true, failed = true, cancelled = true, skipped = true }
local SWEPT = { readyForReview = true, done = true, failed = true }

local StepRow = ui.component(function(props: { plan: PlanProps, row: State<any>, tick: State<number> }): Node
	local plan, row = props.plan, props.row
	local threads = plan.threads
	local function s(): any
		return row:get()
	end
	local id = row:peek().id :: string
	local dk = (string.gsub(id, "[^%w%-]", "-"))
	local base = `{plan.id}-step-{dk}`
	local function child(): Child?
		local c = s().chat
		return if c then threads.child(c) else nil
	end
	local function handback(): Handback?
		return if plan.handback then plan.handback(id) else nil
	end
	local function said(): string?
		return M.step_said(s(), handback())
	end
	local function has_result(): boolean
		local x = s()
		return M.working_line(x) ~= nil or M.facts(x.turn) ~= nil or said() ~= nil or (type(x.earlier) == "table" and #x.earlier > 0)
	end
	local function found(): boolean
		local hb = handback()
		return hb ~= nil and plan.found ~= nil and plan.found(`{hb.id}:u`)
	end
	local function open(): boolean
		if not has_result() then
			return false
		end
		local v = RESULTS:get()[id]
		if v ~= nil then
			return v or found()
		end
		return s().state == "failed" or found()
	end
	local function toggle()
		if not has_result() then
			return
		end
		local all = table.clone(RESULTS:peek())
		all[id] = not open()
		RESULTS:set(all)
	end
	local function title(): string
		return s().title or ""
	end
	local function model(): string?
		local x = s()
		if not x.model then
			return nil
		end
		return (if threads.model_label then threads.model_label(x.harness, x.model) else nil) or x.model
	end
	local function corner(): string
		local x = s()
		local c = child()
		if x.state == "working" then
			local now = props.tick:get()
			local at = if type(x.turn_started_at) == "number" then x.turn_started_at elseif c then c.delegated_at else nil
			return if at then SessionStats.clock((now - at) * 1000) else ""
		end
		if type(x.turn) == "table" and type(x.turn.ms) == "number" then
			return M.elapsed(x.turn.ms / 1000)
		end
		return if c and c.delegated_at and c.delegated_end then M.elapsed(c.delegated_end - c.delegated_at) else ""
	end
	local sweep = ui.use_state(0)
	local sweep_bad = ui.use_state(false)
	local width = ui.use_state(0)
	local last = row:peek().state
	ui.effect(function()
		local st = s().state
		if st == last then
			return
		end
		local was = last
		last = st
		if SWEPT[st] and not ENDED[was] and not motion.reduced_now() then
			sweep_bad:set(st == "failed")
			sweep:set(0)
			anime.animate(sweep, { value = { 0, 1 }, duration = 650, ease = "out(2)" })
		end
	end)
	local reduced = motion.reduced_now()
	local opener = threads.open_child
	local line = ui.button {
		id = `{base}-row`,
		class = "plan-card-step-row",
		label = title,
		expanded = function(): boolean
			return open()
		end,
		selectable = false,
		relative = true,
		overflow = "hidden",
		w_full = true,
		min_w = 0,
		h = ui.raw(28, "the mockup's plan line"),
		p = 0,
		px = ui.raw(4, "the mockup's plan line"),
		gap = ui.raw(8, "the mockup's plan line"),
		items = "center",
		rounded = ui.raw(7, "the mockup's plan line"),
		bg = "#00000000",
		hover = { bg = "$hover" },
		active = { bg = "$hover" },
		text_size = ui.raw(13, "the mockup's plan line"),
		color = function(): string
			local st = s().state
			return if st == "failed" then "$danger" elseif st == "planned" then "$text.secondary" else "$text.primary"
		end,
		cursor = function(): string
			return if has_result() then "pointer" else "default"
		end,
		on_resize = function(size: { w: number, h: number })
			width:set(size.w)
		end,
		on_click = toggle,
		on_press = function() end,
		on_release = function() end,
		M.StepMark {
			kind = function(): string
				return mark_kind(s())
			end,
		},
		ui.text {
			function(): string
				return tostring(s().number or "")
			end,
			literal = true,
			w = ui.raw(14, "the mockup's step number"),
			shrink = 0,
			wrap = false,
			align = "right",
			font = "$font.mono",
			text_size = ui.raw(11, "the mockup's step number"),
			color = "$text.tertiary",
		},
		ui.row {
			grow = 1,
			min_w = 0,
			gap = ui.raw(6, "the mockup's plan line"),
			items = "center",
			FadeText { title, literal = true, min_w = 0, shrink = 1 },
			M.Chips {
				chips = function(): { any }
					return M.chips(s())
				end,
			},
		},
		ui.presence(function(): string?
			local x = s()
			return if x.state == "planned" then x.mark else nil
		end, function(mark: string): Node
			return AgentMark.Mark { label = mark, mark = mark, size = 12, bare = true }
		end, { shrink = 0 }),
		ui.show(function(): boolean
			return s().state == "planned"
		end, ui.text {
			function(): string
				return model() or ""
			end,
			literal = true,
			shrink = 0,
			wrap = false,
			text_size = ui.raw(12, "the mockup's model"),
			color = "$text.tertiary",
		}, ui.text {
			corner,
			id = `{base}-clock`,
			literal = true,
			shrink = 0,
			wrap = false,
			font = "$font.mono",
			text_size = ui.raw(11, "the mockup's plan clock"),
			color = "$text.tertiary",
		}),
		ui.show(
			function(): boolean
				return opener ~= nil and s().chat ~= nil
			end,
			ui.button {
				id = `{base}-open`,
				class = "plan-card-step-open",
				label = function(): string
					return locale.s("ds.threads.plan.open_thread", { title = title() })
				end,
				w = TILE,
				h = TILE,
				p = 0,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = ui.raw(5, "transcript.rs open tile"),
				bg = "#00000000",
				hover = { bg = "$hover" },
				color = "$text.tertiary",
				cursor = "pointer",
				on_click = function()
					local c = s().chat
					if c and opener then
						opener(c)
					end
				end,
				Icon { name = "arrow-up-right", size = 11 },
			}
		),
		ui.div {
			absolute = true,
			top = 0,
			bottom = 0,
			w = function(): number
				return width:get() * 0.45
			end,
			left = function(): number
				local band = width:get() * 0.45
				return -band + sweep:get() * (width:get() + band)
			end,
			opacity = function(): number
				local t = sweep:get()
				return if t <= 0 or t >= 1 then 0 else math.sin(t * math.pi)
			end,
			bg = function(): string
				return if sweep_bad:get() then "$zeron.tone.danger.wash" else "$zeron.tone.success.wash"
			end,
			rounded = "$radius.full",
			pointer_events = "none",
			aria_hidden = true,
		},
	}
	local function result(): Node
		local children: { Node } = {}
		table.insert(
			children,
			ui.show(function(): boolean
				return M.working_line(s()) ~= nil or M.facts(s().turn) ~= nil
			end, ui.text {
				function(): string
					local x = s()
					return M.working_line(x) or M.facts(x.turn) or ""
				end,
				id = `{base}-line`,
				literal = true,
				wrap = true,
				text_size = ui.raw(12.5, "the mockup's result line"),
				color = "$text.secondary",
			})
		)
		table.insert(
			children,
			ui.presence(function(): string?
				local hb = handback()
				return if hb and said() then hb.id else nil
			end, function(msg: string): Node
				local function text(): string
					return said() or ""
				end
				local hb = handback()
				local attempt = if hb then hb.attempt else nil
				return ui.column {
					id = `{base}-said`,
					w_full = true,
					min_w = 0,
					gap = ui.raw(2, "the mockup's result line"),
					if attempt
						then ui.text { locale.s("ds.threads.plan.from_attempt", { n = attempt }), id = `{base}-said-attempt`, wrap = false, text_size = ui.raw(11.5, "the mockup's result line"), color = "$text.tertiary" }
						else nil,
					if plan.body
						then plan.body(text(), `{msg}:u`)
						else ui.text { text, literal = true, wrap = true, text_size = ui.raw(12.5, "the mockup's result line"), color = "$text.secondary" },
				}
			end, { w_full = true, min_w = 0 })
		)
		table.insert(
			children,
			ui.each(ui.computed(function(): { any }
				local out = {}
				for i, e in s().earlier or {} do
					if type(e) == "table" and e.id then
						table.insert(out, { id = e.id, title = e.title, n = i })
					end
				end
				return out
			end), function(e: any): Node
				return ui.button {
					id = `{base}-earlier-{(string.gsub(e.id, "[^%w%-]", "-"))}`,
					class = "plan-card-earlier",
					label = locale.s("ds.threads.plan.earlier", { n = e.n, title = e.title or "" }),
					h = ui.raw(22, "the mockup's result line"),
					p = 0,
					gap = ui.raw(4, "the mockup's result line"),
					items = "center",
					bg = "#00000000",
					hover = { bg = "#00000000", underline = true },
					text_size = ui.raw(12, "the mockup's result line"),
					color = "$text.tertiary",
					cursor = "pointer",
					on_click = function()
						if opener then
							opener(e.id)
						end
					end,
					Icon { name = "arrow-up-right", size = 11 },
					ui.text { locale.s("ds.threads.plan.earlier", { n = e.n, title = e.title or "" }), literal = true, wrap = false, truncate = true, min_w = 0 },
				}
			end, {
				key = function(e: any): string
					return e.id
				end,
				w_full = true,
			})
		)
		local col: { [any]: any } = {
			id = `{base}-result`,
			class = "plan-card-result",
			w_full = true,
			min_w = 0,
			pl = ui.raw(42, "the mockup's result line"),
			pr = ui.raw(8, "the mockup's result line"),
			pb = ui.raw(6, "the mockup's result line"),
			gap = ui.raw(4, "the mockup's result line"),
		}
		for i, n in children do
			col[i] = n
		end
		return ui.column(col)
	end
	return ui.column {
		id = base,
		class = "plan-card-step",
		w_full = true,
		min_w = 0,
		line,
		ui.column {
			w_full = true,
			min_w = 0,
			overflow = "hidden",
			layout = "height",
			layout_clip = true,
			layout_duration = if reduced then 0 else MOVE_MS,
			layout_ease = CARD_EASE,
			ui.presence(function(): string?
				return if open() then "open" else nil
			end, result, {
				w_full = true,
				min_w = 0,
				enter = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, delay = 60, duration = 180 },
				exit = { opacity = 0, duration = if reduced then motion.FADE else 120 },
			}),
		},
	}
end)

function M.Plan(props: PlanProps): Node
	local threads = props.threads
	local card_now = ui.computed(function(): any
		return if props.card then props.card() elseif threads.plan then threads.plan() else nil
	end)
	local function now(): any
		return card_now:get()
	end
	local function approved(): boolean
		local p = now()
		return p ~= nil and not p.pending
	end
	local function finished(): boolean
		local p = now()
		return p ~= nil and not p.pending and p.total > 0 and p.done >= p.total
	end
	local steps = ui.computed(function(): { any }
		local p = now()
		return if p then p.steps else {}
	end)
	local tick = ui.use_state(os.time())
	local working = ui.computed(function(): boolean
		for _, s in steps:get() do
			if s.state == "working" then
				return true
			end
		end
		return false
	end)
	ui.effect(function()
		if working:get() then
			tick:set(os.time())
			ui.every(1000, function()
				tick:set(os.time())
			end)
		end
	end)
	local flash = ui.use_state(0)
	local was_pending = (now() or {}).pending == true
	ui.effect(function()
		local p = now()
		local pending = p ~= nil and p.pending == true
		if was_pending and not pending and p ~= nil and not motion.reduced_now() then
			flash:set(1)
			anime.animate(flash, { value = { 1, 0 }, delay = 270, duration = 630, ease = "out(2)" })
		end
		was_pending = pending
	end)
	local reduced = motion.reduced_now()
	local first_foot = true
	local card = ui.column {
		id = props.id,
		class = "plan-card",
		w_full = true,
		min_w = 0,
		pt = ui.raw(12, "the mockup's question card"),
		px = ui.raw(12, "the mockup's question card"),
		pb = ui.raw(8, "the mockup's question card"),
		rounded = ui.raw(12, "the mockup's question card"),
		border = 1,
		border_color = "$border",
		selectable = false,
		ui.row {
			w_full = true,
			min_w = 0,
			px = ui.raw(4, "the mockup's question card"),
			pb = ui.raw(6, "the mockup's question card"),
			gap = ui.raw(8, "the mockup's question card"),
			items = "center",
			ui.text {
				function(): string
					local p = now()
					local n = if p then p.count else 0
					return locale.s(if finished() then "ds.threads.plan.finished" else "ds.threads.plan.head", { n = n })
				end,
				id = `{props.id}-head`,
				grow = 1,
				min_w = 0,
				text_size = ui.raw(13.5, "the mockup's question card"),
				weight = "$font.weight.medium",
				wrap = false,
				truncate = true,
			},
			ui.show(
				approved,
				M.Roll {
					id = `{props.id}-progress`,
					value = function(): string
						local p = now()
						return if p then `{p.done}/{p.total}` else ""
					end,
					font = "$font.mono",
					text_size = ui.raw(12, "the mockup's plan count"),
					color = "$text.tertiary",
				}
			),
		},
		ui.presence(function(): string?
			return if approved() then "on" else nil
		end, function(): Node
			return ui.div {
				w_full = true,
				px = ui.raw(4, "the mockup's question card"),
				pb = ui.raw(6, "the mockup's question card"),
				ProgressBar {
					size = "thin",
					w_full = true,
					label = locale.t("chat.threads.plan.title"),
					tone = function(): string
						return if finished() then "success" else "accent"
					end :: any,
					value = function(): number?
						local p = now()
						return if p and p.total > 0 then p.done / p.total else 0
					end,
				},
			}
		end, { w_full = true, enter = { opacity = { 0, 1 }, duration = if reduced then motion.FADE else 200 } }),
		ui.show(function(): boolean
			local p = now()
			return p ~= nil and (p.overview or "") ~= ""
		end, ui.text {
			function(): string
				local p = now()
				return if p then p.overview or "" else ""
			end,
			literal = true,
			wrap = true,
			px = ui.raw(4, "the mockup's question card"),
			pb = ui.raw(8, "the mockup's question card"),
			text_size = ui.raw(12.5, "the mockup's question card"),
			color = "$text.secondary",
		}),
		ui.each(steps, function(_s: any, _i: number, row: State<any>): Node
			return StepRow { plan = props, row = row, tick = tick }
		end, {
			key = function(s: any): string
				return s.id or tostring(s.number)
			end,
			w_full = true,
		}),
		ui.presence(function(): string
			local p = now()
			return if p and p.pending then "pending" else "approved"
		end, function(which: string): Node
			local draw = not first_foot
			first_foot = false
			if which == "approved" then
				return ui.row {
					id = `{props.id}-approved`,
					w_full = true,
					justify = "end",
					px = ui.raw(4, "the mockup's approved line"),
					pt = ui.raw(6, "the mockup's approved line"),
					pb = ui.raw(2, "the mockup's approved line"),
					gap = ui.raw(6, "the mockup's approved line"),
					items = "center",
					text_size = ui.raw(12.5, "the mockup's approved line"),
					color = "$success",
					Check { ink = "$success", draw = draw and not reduced },
					ui.text { locale.s("ds.threads.plan.approved"), wrap = false },
				}
			end
			return ui.row {
				w_full = true,
				justify = "end",
				px = ui.raw(4, "the mockup's question foot"),
				pt = ui.raw(6, "the mockup's question foot"),
				pb = ui.raw(2, "the mockup's question foot"),
				gap = ui.raw(6, "the mockup's question foot"),
				items = "center",
				Button {
					locale.t("ds.threads.plan.edit"),
					id = `{props.id}-edit`,
					tone = "ghost",
					size = "xs",
					on_click = function()
						if threads.edit_plan then
							threads.edit_plan()
						end
					end,
				},
				Button {
					locale.t("ds.threads.plan.approve"),
					id = `{props.id}-approve`,
					tone = "accent",
					size = "xs",
					on_click = function()
						if threads.approve_plan then
							threads.approve_plan()
						end
					end,
				},
			}
		end, {
			w_full = true,
			enter = if reduced then { opacity = { 0, 1 }, duration = motion.FADE } else { opacity = { 0, 1 }, y = { 6, 0 }, duration = 200, ease = CARD_EASE },
			exit = { opacity = 0, duration = if reduced then motion.FADE else 140 },
		}),
	}
	return ui.column {
		relative = true,
		w_full = true,
		min_w = 0,
		card,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = ui.raw(12, "the mockup's question card"),
			border = 1,
			border_color = "$accent",
			outline = 3,
			outline_color = "$zeron.peer_card.ring",
			pointer_events = "none",
			aria_hidden = true,
			opacity = function(): number
				return flash:get()
			end,
		},
	}
end

local function range_of(numbers: { number }): string
	local sorted = table.clone(numbers)
	table.sort(sorted)
	local contiguous = true
	for i = 2, #sorted do
		if sorted[i] ~= sorted[i - 1] + 1 then
			contiguous = false
		end
	end
	if contiguous and #sorted > 1 then
		return locale.s("ds.threads.started.range", { from = sorted[1], to = sorted[#sorted] })
	end
	local words: { string } = {}
	for _, n in sorted do
		table.insert(words, tostring(n))
	end
	return table.concat(words, ", ")
end

function M.StepsStarted(props: { id: string, steps: () -> { string }, plan: () -> any }): Node
	local function lines(): { any }
		local p = props.plan()
		local out = {}
		for _, id in props.steps() do
			local s = if p and p.by_step then p.by_step[id] else nil
			if s then
				table.insert(out, s)
			end
		end
		return out
	end
	local function verb(): string
		local l = lines()
		if #l == 1 then
			return locale.s("ds.threads.started.one", { n = l[1].number or 0 })
		end
		return locale.s("ds.threads.started.many", { n = #props.steps() })
	end
	local function target(): string
		local l = lines()
		if #l == 1 then
			return l[1].title or ""
		end
		local numbers: { number } = {}
		for _, s in l do
			if s.number then
				table.insert(numbers, s.number)
			end
		end
		return range_of(numbers)
	end
	return ui.row {
		id = props.id,
		class = "plan-steps-started",
		w_full = true,
		min_w = 0,
		h = ui.raw(26, "the mockup's tool row"),
		gap = "$space.2",
		items = "center",
		text_size = "$font.size.sm",
		color = "$text.secondary",
		label = function(): string
			return `{verb()} {target()}`
		end,
		ui.div { shrink = 0, color = "$text.tertiary", aria_hidden = true, Icon { name = "side-chat", size = 14 } },
		ui.text { verb, literal = true, shrink = 0, wrap = false },
		FadeText { target, literal = true, min_w = 0, shrink = 1, color = "$text.primary" },
	}
end

return M
