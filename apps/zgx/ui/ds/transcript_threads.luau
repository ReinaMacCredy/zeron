-- The rows that tie a conversation to other chats, as Zeron's transcript
-- draws them (crates/ui/src/transcript.rs, limit_card.rs, route_badge.rs):
--
-- - a subagent's spawn card: bordered, its brand or model mark on a tile,
--   the model or agent type it was given, a spinner while it runs, the
--   danger's ink once it failed, an accent edge while its tab is open, the
--   open arrow (`Spawn`);
-- - a task delegated to another chat: its harness's mark, its title, role
--   and model, how it stands and for how long, the route's badge, the open
--   arrow, and under it a limit card when it hit its usage limit
--   (`Delegate`, `LimitCard`); a native spawn the engine routed is the same
--   chip, not opened (`NativeRoute`);
-- - "Started thread" for a thread an agent started (`Started`);
-- - "Message to [mark] Chat" for a message an agent sent another chat,
--   "Sending to" while it's on its way (dimmed), "Couldn't message" once it
--   failed (`Sent`);
-- - a delegated task's wake ("Delegated task “x” finished", Open subagent)
--   and a peer's message ("From [mark] thread", its time, a hairline), in a
--   person's message's place (`Wake`, `From`).
--
-- Each state reads in words, not only in a colour. Nothing here ticks at
-- rest: a spinner is there only while its run is, a countdown only while it
-- counts.
--
-- What they need of the chat they're in (`Threads`): a child chat as the
-- sidebar lists it, a chat a message names, a delegate's limit, whether a
-- child's tab is open, and what opens a child, a chat, a limit's choice.

local Icon = require("@self/icon")
local AgentMark = require("@self/agent_mark")
local FadeText = require("@self/fade_text")
local status = require("@self/status")
local locale = require("@self/locale")

local M = {}

-- What's given, without the nils between (a table's length can't be trusted past a hole).
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
	-- A child (or any listed chat) by id, as the sidebar lists it.
	child: (id: string) -> Child?,
	-- The chat a message names (its id, a prefix of it, or its title).
	chat: (key: string) -> { id: string, title: string, mark: string? }?,
	-- A delegate's usage limit, while it holds an offer (src/chat.rs `limits`).
	limit: ((id: string) -> any?)?,
	-- Whether a child's tab is open in this chat's panel.
	is_open: ((id: string) -> boolean)?,
	open_child: ((id: string) -> ())?,
	open_chat: ((id: string) -> ())?,
	-- A limit's choice, sent for the child ("switch", "wait", "hold").
	limit_op: ((id: string, op: string) -> ())?,
	-- The chat's own agent (a native route ran on it).
	agent: (() -> { label: string?, mark: string?, model: string? }?)?,
}

-- gpui's chip card: 30 high, its header 28, corner 9; the tile 18, corner 5.
local CARD, HEADER, TILE = 30, 28, 18
local TAG_SIZE = 11

local NAMES = { ["claude-code"] = "Claude Code", codex = "Codex", grok = "Grok", cursor = "Cursor", mock = "Mock", opencode = "OpenCode", devin = "Devin", pi = "Pi", hermes = "Hermes", antigravity = "Antigravity", muse = "Muse" }
function M.harness_name(harness: string?): string
	if not harness then
		return locale.s("ds.threads.agent")
	end
	return NAMES[harness] or harness
end

-- "4:12 PM" within a day, a weekday within a week, else "Nov 9" (Zeron's
-- `format_reset`, without the "resets").
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

-- "2m", "1h 5m" (Zeron's `limit_duration`).
function M.duration(ms: number): string
	local mins = math.max(0, math.floor(ms / 60000))
	local h, m = math.floor(mins / 60), mins % 60
	if h == 0 then
		return `{m}m`
	end
	return if m == 0 then `{h}h` else `{h}h {m}m`
end

-- "12s", "2m 3s", "1h 4m", "2d 3h" (Zeron's `format_elapsed`).
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

-- A route's badge words (Zeron's `badge_label`).
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

-- An agent's mark on the chip's quiet tile (Zeron's `agent_brand_icon`: the
-- model's maker first, else the harness, else the bot).
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

-- The card around a chip: bordered, an accent edge while its tab is open, a
-- press anywhere on it opening it.
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
	-- (Its open state as a node a test can find: an edge's colour isn't.)
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

-- A subagent's spawn. `now()` is its tool (ds/activity's): `subagent`
-- names its doc, how it stands and its title; `tag`, what it was given to
-- run on.
function M.Spawn(props: { now: () -> any, threads: Threads?, open_subagent: ((sub: any) -> ())?, agent: (() -> any)? }): Node
	local function sub(): any
		return props.now().subagent
	end
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
	local mark, icon = s0.agent, nil
	if not mark and props.agent then
		local a = props.agent()
		if a then
			mark, icon = a.mark, a.icon
		end
	end
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
			-- How it stands, in words: a spinner beside "Running", nothing
			-- ticking once it's done.
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

local DELEGATION_STATE = { running = "ds.threads.state.running", completed = "ds.threads.state.completed", failed = "ds.threads.state.failed", cancelled = "ds.threads.state.stopped", interrupted = "ds.threads.state.stopped" }

-- How a delegate stands, as Zeron's `delegate_state_text` words it:
-- "Running", "Completed in 2m 3s", "Hit usage limit".
function M.delegate_state(c: Child?, limited: boolean): string
	if limited then
		return locale.s("ds.threads.state.limited")
	end
	local status_key = if c then DELEGATION_STATE[c.delegation or ""] else nil
	local word = if status_key then locale.s(status_key) else locale.s("ds.threads.state.unavailable")
	if c and c.delegation and c.delegation ~= "running" and c.delegated_at and c.delegated_end then
		return locale.s("ds.threads.state.after", { state = word, time = M.elapsed(c.delegated_end - c.delegated_at) })
	end
	return word
end

-- The model's name as the chip says it: its last part.
local function model_word(model: string?): string?
	if not model or model == "" then
		return nil
	end
	return string.match(model, "([^/]+)$") or model
end

-- A delegated task: the chat it runs in (`ref`), its title as it's listed
-- now. A native route (`native`) is the same chip on the chat's own agent,
-- with nothing to open.
function M.Delegate(props: { id: string, ref: string?, title: string?, route: any?, native: boolean?, threads: Threads?, model_label: ((harness: string?, model: string?) -> string?)? }): Node
	local threads = props.threads
	local function child(): Child?
		return if props.ref and threads then threads.child(props.ref) else nil
	end
	local function limit(): any
		return if props.ref and threads and threads.limit then threads.limit(props.ref) else nil
	end
	local function failed(): boolean
		local c = child()
		return c ~= nil and c.delegation == "failed"
	end
	local function title(): string
		local c = child()
		local t = if c and c.title and c.title ~= "" then c.title else props.title
		return t or locale.s("ds.threads.delegated_task")
	end
	local agent = if props.native and threads and threads.agent then threads.agent() else nil
	local c0 = child()
	local harness = if c0 then c0.harness else nil
	local function name(): string
		if props.native then
			return locale.s("ds.threads.native", { agent = if agent and agent.label then agent.label else locale.s("ds.threads.agent") })
		end
		local c = child()
		return M.harness_name(if c then c.harness else harness)
	end
	local function meta(): string?
		local c = child()
		local route = props.route or (if c then c.route else nil)
		local role = if type(route) == "table" then route.role else nil
		local model = if props.native then (if agent then agent.model else nil) else (if c then c.model else nil)
		local said = if model and props.model_label then props.model_label(if c then c.harness else nil, model) else nil
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
	local open = if not props.native and props.ref and threads and threads.open_child
		then function()
			(threads.open_child :: (string) -> ())(props.ref :: string)
		end
		else nil
	local route = props.route or (if c0 then c0.route else nil)
	local mark = if props.native then (if agent then agent.mark else nil) else (if c0 then c0.mark or c0.agent else nil)
	local icon = if c0 then c0.agent_icon else nil
	local chip = card {
		id = props.id,
		label = function(): string
			if props.native then
				return `{name()} {title()}`
			end
			return `{name()} {title()} {state()}`
		end,
		open = function(): boolean
			return props.ref ~= nil and threads ~= nil and threads.is_open ~= nil and threads.is_open(props.ref)
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
	if props.native or not props.ref then
		return chip
	end
	return ui.column {
		w_full = true,
		gap = 0,
		chip,
		ui.presence(function(): string?
			local l = limit()
			return if l then `limit:{l.act_at or ""}` else nil
		end, function(): Node
			return M.LimitCard { child = props.ref :: string, limit = limit, threads = threads :: Threads }
		end, { w_full = true }),
	}
end

-- What a choice is called on the wire, and its words.
local ACT = { switch = "ds.threads.limit.switch_now", wait = "ds.threads.limit.wait_now" }
local VERB = { switch = "ds.threads.limit.switching_in", wait = "ds.threads.limit.waiting_in" }

-- The choices kept by child (a person's pick outlives the card made again).
local CHOSEN = ui.state("threads_limit_chosen", {} :: { [string]: string })

-- A delegate stopped on its usage limit, with an offer: switch to another
-- agent (keeping the transcript) or wait for the reset, the preselected
-- choice acting on its own when the countdown ends, Cancel holding it
-- (Zeron's `limit_card`).
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
	-- (The clock ticks only while it counts down.)
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
				color = "$bg",
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

-- "Started thread [title] [route] · Open", a hairline to the edge.
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

-- A message to another chat, at the reply's left: its state in words (and
-- dimmed while it's on its way), the chat's mark and title as listed now; a
-- click opens it.
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

-- A delegated task's wake, in its message's place: what finished, and Open
-- subagent for one task (Zeron's `render_wake_row`).
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

-- A peer's message's caption (thread tools): "From [mark] thread", each
-- sender a way over to it, its time, a hairline (Zeron's C3 rule).
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

-- "Auto-continue" over a turn the chat sent itself.
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

return M
