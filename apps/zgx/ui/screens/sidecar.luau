--!strict
-- The Live state column beside a session's transcript (crates/ui shell/summary.rs).

local Icon = require("@ds/icon")
local Mark = require("@ds/mark")
local Tooltip = require("@ds/tooltip")
local Menu = require("@ds/menu")
local ContextMenu = require("@ds/context_menu")
local control = require("@ds/control")
local motion = require("@ds/motion")
local locale = require("@ds/locale")
local status = require("@ds/status")
local sessions = require("@account/sessions")
local art = require("@app/screens/shell/art")
local store = require("@app/screens/chat/store")
local lanes = require("@app/screens/chat/lanes")
local Pills = require("@app/screens/chat/pills")
local Actions = require("@app/screens/panel/actions")
local home_api = require("@app/screens/panel/home_api")
local panel_state = require("@app/screens/panel/state")
local boundary = require("@app/screens/shell/boundary")
local tick_of_running = require("@app/screens/sidecar_tick")

local M = {}

M.WIDTH = 260
M.RAIL = 40
M.AUTO_RAIL_AT = 640
local TOP = 6
local POPOVER_W = 262
local CAP = 8
local RAIL_BUTTON = 32
local RAIL_GAP = 4
local LAYOUT = { duration = 300, ease = "cubicBezier(0.32, 0.72, 0, 1)" }
local FADE = { duration = 150, ease = "cubicBezier(0.25, 0.1, 0.25, 1)" }
local POPOVER_FADE = { duration = 160, ease = "cubicBezier(0.25, 0.1, 0.25, 1)" }
local POPOVER_SLIDE = { duration = 220, ease = "cubicBezier(0.32, 0.72, 0, 1)" }
local TOOLTIP_MS, RAIL_TOOLTIP_MS = 500, 260

M.collapsed = ui.persist("sidecar_collapsed", false)
local previous_open = ui.state("sidecar_previous_open", false)
local beside_all = ui.state("sidecar_beside_all", false)

local function icon_at(file: string, size: number): Node
	return ui.svg { src = `assets/sidecar/{file}.svg`, w = size, h = size, shrink = 0, aria_hidden = true, tint = "currentColor" }
end


local function short_elapsed(secs: number): string
	local s = math.max(math.floor(secs), 0)
	if s < 60 then
		return locale.s("sidecar.elapsed.s", { n = s })
	elseif s < 3600 then
		return locale.s("sidecar.elapsed.m", { n = s // 60 })
	elseif s < 86400 then
		return locale.s("sidecar.elapsed.h", { n = s // 3600 })
	end
	return locale.s("sidecar.elapsed.d", { n = s // 86400 })
end

-- "sha · 3 ahead of main". On the integration branch itself "N ahead of main" would be wrong: the sha alone.
function M.commit_line(sha: string?, branch: string?, comparison: any): string?
	local words: string? = nil
	if comparison then
		local base = string.match(comparison.base, "^[^/]+/(.+)$") or comparison.base
		local ahead, behind = comparison.ahead, comparison.behind
		if ahead == 0 and behind == 0 then
			words = locale.s("sidecar.commit.up_to_date")
		elseif branch ~= base then
			if ahead == 0 then
				words = locale.s("sidecar.commit.behind", { n = behind, base = base })
			elseif behind == 0 then
				words = locale.s("sidecar.commit.ahead", { n = ahead, base = base })
			else
				words = locale.s("sidecar.commit.both", { ahead = ahead, behind = behind, base = base })
			end
		end
	end
	if sha and words then
		return `{sha} · {words}`
	end
	return sha or words
end

function M.tilde(path: string, home: string?): string
	if home and path == home then
		return "~"
	elseif home and string.sub(path, 1, #home + 1) == home .. "/" then
		return "~" .. string.sub(path, #home + 1)
	end
	return path
end

function M.split_subagents(list: { any }): ({ any }, { any })
	local running, previous = {}, {}
	for i = #list, 1, -1 do
		local a = list[i]
		if a.status == "running" then
			table.insert(running, a)
		else
			table.insert(previous, a)
		end
	end
	return running, previous
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

local function device_of(id: string?): sessions.Device?
	for _, d in sessions.now:get().devices do
		if d.id == id then
			return d
		end
	end
	return nil
end

local function device_icon(d: sessions.Device?): { asset: string?, name: string? }
	if d and d.platform == "cloud" then
		return { name = "cloud" }
	elseif d and d.platform == "linux" and not d.here then
		return { name = "remote-server" }
	end
	return { asset = "astra-laptop" }
end

local function model_label(harness: string?, model: string): string
	for _, m in store.catalog:get().models or {} do
		if m.value == `{harness or ""}/{model}` and m.label then
			return m.label
		end
	end
	return model
end


local WORKING = "$zeron.sidecar.working"
local DONE = "$zeron.sidecar.done"
local FAILED = "$danger"
local FAINT = "$zeron.sidecar.faint"
local HEADING = "$zeron.sidecar.heading"

export type Entry = {
	key: string,
	icon: { asset: string?, name: string?, mark: string? },
	label: string,
	ink: string?,
	hint: string?,
	value: any,
	act: { [string]: any }?,
	trail: { icon: string, label: string, act: { [string]: any } }?,
	away: boolean?,
	session_menu: string?,
}

local function glyph_of(icon: { asset: string?, name: string?, mark: string? }): Node
	if icon.mark then
		return Mark { name = icon.mark :: any, size = 14, bare = true }
	elseif icon.asset then
		return icon_at(icon.asset, 14)
	end
	return Icon { name = (icon.name or "bot") :: any, size = 14 }
end

local function row(entry: State<Entry>, surface: string, data: any, after: (() -> ())?): Node
	local hovered = ui.use_state(false)
	local trail_focused = ui.use_state(false)
	local value = ui.computed(function(): any
		local v = entry:get().value
		if v and (v.since or v.expires) then
			data.tick:get()
			local now = os.time()
			if v.expires and now >= v.expires then
				return { kind = "dot", text = locale.s("sidecar.expired"), color = FAINT }
			elseif v.since then
				local words = short_elapsed(now - v.since)
				return { kind = v.kind, color = v.color, text = if v.ago then locale.s("sidecar.ago", { time = words }) else words }
			end
		end
		return v
	end)
	local function clickable(): boolean
		return entry:get().act ~= nil
	end
	-- (A row that does something now and then, an action while its list loads, stays one.)
	local button = entry:peek().act ~= nil or string.sub(entry:peek().key, 1, 7) == "action-"
	local function act(what: { [string]: any }?)
		if what then
			data.run(what)
		end
		if after then
			after()
		end
	end
	local node = ui.row {
		id = `sidecar-{surface}-{entry:peek().key}`,
		class = "sidecar-row",
		role = if button then "button" else "group",
		label = function(): string
			return entry:get().label
		end,
		value_text = function(): string?
			local v = value:get()
			return if v then v.text else nil
		end,
		focusable = button,
		w_full = true,
		h = ui.raw(28, "a Sidecar row"),
		shrink = 0,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		rounded = "$radius.md",
		cursor = function(): string
			return if clickable() then "pointer" else "default"
		end,
		color = "$text.primary",
		hover = if button then { bg = "$hover" } else nil,
		focus = control.ring,
		on_hover = function(inside: boolean)
			hovered:set(inside)
		end,
		on_click = function()
			if entry:peek().act then
				act(entry:peek().act)
			end
		end,
		on_key = function(e: any): boolean?
			if (e.key == "Enter" or e.key == " ") and not (e.cmd or e.ctrl or e.alt) and entry:peek().act then
				act(entry:peek().act)
				return true
			end
			return nil
		end,
		tooltip = Tooltip {
			function(): string
				local e = entry:get()
				if e.hint then
					return e.hint
				end
				local v = value:get()
				return if v and v.text then `{e.label}: {v.text}` else e.label
			end,
		},
		tooltip_delay = TOOLTIP_MS,
		tooltip_placement = { side = "left", align = "center", offset = 8 },
		ui.row {
			w = 14,
			h = 14,
			shrink = 0,
			items = "center",
			justify = "center",
			color = function(): string
				return if clickable() and hovered:get() then "$text.primary" else "$text.secondary"
			end,
			ui.presence(function(): string
				local i = entry:get().icon
				return `{i.mark or ""}|{i.asset or ""}|{i.name or ""}`
			end, function(_: string): Node
				return glyph_of(entry:peek().icon)
			end),
		},
		ui.text {
			function(): string
				return entry:get().label
			end,
			literal = true,
			grow = 1,
			min_w = 0,
			truncate = true,
			text_size = "$font.size.md",
			color = function(): string
				return entry:get().ink or "$text.primary"
			end,
		},
		if entry:peek().key == "branch" then data.branch_slot(surface) else nil,
		ui.show(function(): boolean
			return entry:get().away == true
		end, ui.div {
			shrink = 0,
			role = "img",
			label = locale.t("panel.home.away"),
			tooltip = Tooltip { locale.t("panel.home.away") },
			color = "$warning",
			Icon { name = "wifi-off", size = 14 },
		}),
		ui.show(function(): boolean
			return value:get() ~= nil
		end, ui.row {
			shrink = 0,
			max_w = ui.raw(120, "a Sidecar value at most"),
			gap = ui.raw(5, "a Sidecar value's dot"),
			items = "center",
			ui.show(function(): boolean
				local v = value:get()
				return v ~= nil and v.kind == "dot"
			end, ui.div {
				class = "sidecar-dot",
				w = 6,
				h = 6,
				shrink = 0,
				rounded = "$radius.full",
				bg = function(): string
					local v = value:get()
					return if v and v.color then v.color else FAINT
				end,
			}),
			ui.text {
				function(): string
					local v = value:get()
					return if v then v.text or "" else ""
				end,
				class = "sidecar-value",
				literal = true,
				min_w = 0,
				truncate = true,
				text_size = function(): string
					local v = value:get()
					return if v and v.kind == "open" then "$font.size.xs" else "$font.size.sm"
				end,
				weight = function(): string
					local v = value:get()
					return if v and v.kind == "open" then "$font.weight.medium" else "$font.weight.regular"
				end,
				color = function(): string
					local v = value:get()
					if v and v.kind == "open" then
						return "$accent"
					end
					if v and v.kind == "tone" then
						return v.color
					end
					return FAINT
				end,
			},
		}),
		ui.show(function(): boolean
			return entry:get().trail ~= nil
		end, ui.button {
			class = "sidecar-trail",
			w = 22,
			h = 22,
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = "$radius.sm",
			cursor = "pointer",
			bg = "#00000000",
			color = "$text.secondary",
			role = "button",
			label = function(): string
				local t = entry:get().trail
				return if t then t.label else ""
			end,
			tooltip = Tooltip {
				function(): string
					local t = entry:get().trail
					return if t then t.label else ""
				end,
			},
			opacity = function(): number
				return if hovered:get() or trail_focused:get() then 1 else 0
			end,
			on_focus = function()
				trail_focused:set(true)
			end,
			on_blur = function()
				trail_focused:set(false)
			end,
			hover = { bg = "$hover" },
			focus = control.ring,
			on_click = function()
				local t = entry:peek().trail
				if t then
					act(t.act)
				end
			end,
			ui.presence(function(): string
				local t = entry:get().trail
				return if t then t.icon else ""
			end, function(name: string): Node
				return if name == "" then ui.div {} elseif name == "stop" then icon_at("stop", 14) else Icon { name = name :: any, size = 14 }
			end),
		}),
	}
	if not entry:peek().session_menu then
		return node
	end
	return ContextMenu {
		items = function(): { any }?
			local sidebar: any = boundary.load("@app/screens/shell/sidebar")
			local id = entry:peek().session_menu
			return if sidebar and sidebar.session_menu and id then sidebar.session_menu(id) else nil
		end,
		open = ui.use_state(false),
		ring = "$radius.md",
		w_full = true,
		node,
	}
end

local function rows(list: () -> { Entry }, surface: string, data: any, after: (() -> ())?): Node
	return ui.each(ui.computed(list), function(_e: Entry, _i: State<number>, e: State<Entry>): Node
		return row(e, surface, data, after)
	end, {
		key = function(e: Entry): string
			return e.key
		end,
		w_full = true,
	})
end


type Data = {
	run: (act: { [string]: any }) -> (),
	tick: State<number>,
	refresh: () -> (),
	session: () -> string?,
	workspace: () -> { Entry },
	beside: () -> { Entry },
	previous: () -> { Entry },
	foot: () -> { Entry },
	failed: () -> number,
	running: () -> number,
	actions: () -> { Entry },
	actions_shown: () -> boolean,
	slot_session: State<any>,
	dialogs: Node,
}

local function make_data(props: { session: () -> string?, open_panel: () -> () }): Data
	local function session(): string?
		return props.session()
	end
	local function open_chat(): sessions.Chat?
		return chat_of(session())
	end
	local chat = ui.computed(open_chat)

	local tick = ui.state("sidecar_tick", 0)

	local comparison = ui.state("sidecar_comparison", {} :: { [string]: any })
	local asked: { [string]: string } = {}
	local function git_on(c: sessions.Chat?): boolean
		local p = if c then project_of(c.project) else nil
		return p ~= nil and p.git == true
	end
	ui.effect(function()
		local c = chat:get()
		if not c or not git_on(c) or c.work_path then
			return
		end
		local key = `{c.live_branch or ""}@{c.head or ""}`
		local cwd = c.cwd
		if asked[c.id] == key or not cwd then
			return
		end
		asked[c.id] = key
		local id = c.id
		home_api.comparison(cwd, c.device):after(function(found: any)
			if asked[id] == key then
				comparison:set_key(id, found or false)
			end
		end, function()
			if asked[id] == key then
				comparison:set_key(id, false)
			end
		end)
	end)
	local workspace = ui.computed(function(): { Entry }
		local c = chat:get()
		local d = device_of(if c then c.device else nil)
		local home = if d and d.here then sessions.now:get().home else nil
		local path = if c then c.work_path or c.cwd else nil
		local shown = if path then M.tilde(path, home) else locale.s("sidecar.no_workspace")
		local out: { Entry } = {
			{
				key = "machine",
				icon = device_icon(d),
				label = if d then d.name else locale.s("sidecar.local"),
				away = d ~= nil and not d.here and d.online == false,
			},
			{
				key = "path",
				icon = { asset = "astra-folder" },
				label = shown,
				hint = shown,
			},
		}
		if git_on(c) then
			local branch = if c then c.live_branch else nil
			table.insert(out, {
				key = "branch",
				icon = { asset = "astra-git-branch" },
				label = if branch and branch ~= "" then branch else locale.s("sidecar.no_branch"),
				ink = "$zeron.sidecar.branch",
			})
			local sha = if c and c.head and not c.work_path then string.sub(c.head, 1, 7) else nil
			local compared = if c and not c.work_path then comparison:get()[c.id] else nil
			local line = M.commit_line(sha, branch, compared or nil)
			if line then
				table.insert(out, {
					key = "commit",
					icon = { asset = "lucide-git-commit-horizontal" },
					label = line,
					ink = HEADING,
				})
			end
		end
		return out
	end)

	local function open_tabs(): { [string]: boolean }
		local out: { [string]: boolean } = {}
		local sid = session()
		if not sid then
			return out
		end
		for _, t in panel_state.of(`chat:{sid}`).tabs do
			if (t.kind == "subagent" or t.kind == "side_chat") and t.chat then
				out[t.chat] = true
			end
		end
		return out
	end
	local subs = ui.computed(function(): { any }
		local s = store.session:get()
		if not s or s.id ~= session() then
			return {}
		end
		return lanes.subagents()
	end)
	local split = ui.computed(function(): { running: { any }, previous: { any } }
		local running, previous = M.split_subagents(subs:get())
		return { running = running, previous = previous }
	end)

	local function agent_entry(a: any, opened: { [string]: boolean }, harness: string?, mark: string?): Entry
		local parts = { a.title, locale.s("sidecar.kind.agent") }
		if a.model then
			table.insert(parts, model_label(harness, a.model))
		elseif a.kind then
			table.insert(parts, a.kind)
		end
		local value: any
		if opened[a.ref] then
			value = { kind = "open", text = locale.s("sidecar.open") }
		elseif a.status == "running" then
			value = if a.at then { kind = "dot", since = a.at, color = WORKING } else { kind = "dot", text = locale.s("ds.status.working"), color = WORKING }
		elseif a.status == "failed" then
			value = { kind = "dot", text = locale.s("ds.status.failed"), color = FAILED }
		elseif a.status == "stopped" then
			value = { kind = "dot", text = locale.s("sidecar.stopped"), color = FAINT }
		elseif a.status == "done" then
			value = { kind = "dot", text = locale.s("ds.status.done"), color = DONE }
		end
		local agent = a.agent or mark
		return {
			key = `agent-{a.ref}`,
			icon = if agent then { mark = agent } else { name = "bot" },
			label = a.title,
			hint = table.concat(parts, " · "),
			value = value,
			act = { what = "subagent", sub = { ref = a.ref, title = a.title, status = a.status } },
		}
	end

	local function children_now(): { any }
		local refs: { [string]: boolean } = {}
		for _, a in subs:get() do
			refs[a.ref] = true
		end
		local out: { any } = {}
		for _, k in sessions.children(session()) do
			if not refs[k.id] then
				table.insert(out, k)
			end
		end
		table.sort(out, function(a: any, b: any): boolean
			if a.created ~= b.created then
				return (a.created or 0) < (b.created or 0)
			end
			return a.id < b.id
		end)
		return out
	end
	local children = ui.computed(children_now)

	local function child_entry(k: any, opened: { [string]: boolean }): Entry
		local kind = k.kind or "side_chat"
		local kind_word = locale.s(`sidecar.kind.{kind}`)
		local title = if k.title and k.title ~= "" then k.title else kind_word
		local value: any
		if opened[k.id] then
			value = { kind = "open", text = locale.s("sidecar.open") }
		elseif k.status == "working" or k.status == "connecting" or k.status == "waiting" then
			value = { kind = "dot", text = status.KINDS[k.status :: any].label, color = WORKING }
		elseif k.status == "input" or k.status == "approval" or k.status == "plan" then
			value = { kind = "dot", text = status.KINDS[k.status :: any].label, color = "$zeron.sidecar.asking" }
		elseif k.status == "failed" or k.status == "limited" then
			value = { kind = "dot", text = status.KINDS[k.status :: any].label, color = FAILED }
		else
			value = { kind = "dot", since = k.updated or k.created or os.time(), ago = true, color = FAINT }
		end
		if not opened[k.id] then
			value.expires = k.expires
		end
		local icon: any = { name = "bot" }
		if kind == "fork" then
			icon = { asset = "fork" }
		elseif kind == "side_chat" then
			icon = { asset = "chat-round-line" }
		elseif k.agent then
			icon = { mark = k.agent }
		end
		return {
			key = `child-{k.id}`,
			icon = icon,
			label = title,
			hint = `{title} · {kind_word}`,
			value = value,
			act = { what = "child", id = k.id, title = title },
			session_menu = k.id,
		}
	end

	local function shells_now(): { any }
		local s = store.session:get()
		if not s or s.id ~= session() then
			return {}
		end
		local out: { any } = {}
		for _, t in (s :: any).tasks or {} do
			if t.task_kind == nil or t.task_kind == "shell" then
				table.insert(out, t)
			end
		end
		return out
	end
	local shells = ui.computed(shells_now)
	-- What Stop all stops: the agents that can stop and the shells, where
	-- shells can be (the shipping app's `stop_all_background`), for this chat.
	local function stoppable(): { agents: { any }, shells: { any } }
		local s = store.session:get()
		if not s or s.id ~= session() then
			return { agents = {}, shells = {} }
		end
		return Pills.stoppable_of(Pills.window_runs(), s)
	end

	local function stop_task(id: string)
		local sid = session()
		if sid then
			store.stop_task(sid, id):after(function() end, function() end)
		end
	end

	local beside = ui.computed(function(): { Entry }
		local opened = open_tabs()
		local c = chat:get()
		local harness = if c then c.harness else nil
		local mark = if c then c.agent else nil
		local out: { Entry } = {}
		for i, a in split:get().running do
			if i > CAP then
				break
			end
			table.insert(out, agent_entry(a, opened, harness, mark))
		end
		local list = children:get()
		local all = beside_all:get()
		local shown = if all then #list else math.min(#list, CAP)
		for i = 1, shown do
			table.insert(out, child_entry(list[i], opened))
		end
		local hidden = #list - CAP
		if hidden > 0 then
			table.insert(out, {
				key = "children-more",
				icon = { name = "more-horizontal" },
				label = if all then locale.s("sidecar.show_fewer") else locale.s("sidecar.more", { n = hidden }),
				act = { what = "more" },
			})
		end
		for i, t in shells:get() do
			if i > CAP then
				break
			end
			table.insert(out, {
				key = `shell-{t.id}`,
				icon = { asset = "astra-terminal" },
				label = t.detail or t.name or "",
				value = if t.since then { kind = "dot", since = t.since, color = WORKING } else { kind = "dot", text = locale.s("ds.status.working"), color = WORKING },
				trail = if (store.session:get() :: any).shells_stoppable ~= false then { icon = "stop", label = locale.s("sidecar.stop"), act = { what = "stop_task", id = t.id } } else nil,
			})
		end
		return out
	end)

	local tick_of, ticking = 0, ""
	ui.effect(function()
		local started: { number } = {}
		for _, a in split:get().running do
			if a.at then
				table.insert(started, a.at)
			end
		end
		for _, t in shells:get() do
			if t.since then
				table.insert(started, t.since)
			end
		end
		table.sort(started)
		local key = table.concat(started, ",")
		if key == ticking then
			return
		end
		ticking = key
		tick_of += 1
		local mine = tick_of
		local function arm()
			local soonest = tick_of_running.soonest(started, os.time())
			if not soonest then
				return
			end
			ui.after(soonest * 1000, function()
				if mine == tick_of then
					tick:set(tick:peek() + 1)
					arm()
				end
			end)
		end
		arm()
	end)
	ui.on_cleanup(function()
		tick_of += 1
	end)

	local previous = ui.computed(function(): { Entry }
		if not previous_open:get() then
			return {}
		end
		local opened = open_tabs()
		local c = chat:get()
		local out: { Entry } = {}
		for _, a in split:get().previous do
			table.insert(out, agent_entry(a, opened, if c then c.harness else nil, if c then c.agent else nil))
		end
		return out
	end)
	local foot = ui.computed(function(): { Entry }
		local can = stoppable()
		if #can.agents == 0 and #can.shells == 0 then
			return {}
		end
		return {
			{
				key = "stop-all",
				icon = { asset = "stop" },
				label = locale.s("sidecar.stop_all"),
				act = { what = "stop_all" },
			},
		}
	end)
	local failed = ui.computed(function(): number
		local n = 0
		for _, a in split:get().previous do
			if a.status == "failed" then
				n += 1
			end
		end
		return n
	end)
	local running = ui.computed(function(): number
		local n = #split:get().running
		for _, k in children:get() do
			if k.status == "working" then
				n += 1
			end
		end
		return n
	end)

	local bound = Actions.bind(session, function(title: string, run: { [string]: any })
		local sid = session()
		if sid then
			panel_state.open(`chat:{sid}`, "terminal", { title = title, run = run })
			props.open_panel()
		end
	end)
	local function run_value(a: any): any
		local sid = session()
		if not sid then
			return nil
		end
		local tab: any = nil
		for _, t in panel_state.of(`chat:{sid}`).tabs do
			if t.kind == "terminal" and t.run and t.run.action == a.id then
				tab = t
			end
		end
		if not tab then
			return nil
		end
		local run = panel_state.runs:get()[tab.id]
		if not run or run.unknown then
			return nil
		elseif run.code == nil then
			return { kind = "tone", text = locale.s("sidecar.run.running"), color = WORKING }
		elseif run.code == 0 then
			return { kind = "tone", text = locale.s("sidecar.run.done"), color = DONE }
		end
		return { kind = "tone", text = locale.s("sidecar.run.failed"), color = "$danger" }
	end
	local actions = ui.computed(function(): { Entry }
		local out: { Entry } = {}
		local s = bound.now()
		if (s == nil or s.state == "loading") and #bound.list:get() == 0 then
			table.insert(out, {
				key = "actions-loading",
				icon = { name = "refresh" },
				label = locale.s("panel.home.actions_loading"),
			})
			return out
		end
		if s and s.state == "failed" then
			table.insert(out, {
				key = "actions-unavailable",
				icon = { name = "info-circle" },
				label = s.message or "",
				hint = s.message or "",
			})
			table.insert(out, {
				key = "actions-retry",
				icon = { name = "refresh" },
				label = locale.s("panel.home.retry"),
				act = { what = "retry" },
			})
		end
		for _, a in bound.list:get() do
			local name = if a.setup then locale.s("panel.home.setup_suffix", { name = a.name }) else a.name
			table.insert(out, {
				key = `action-{a.id}`,
				icon = { name = Actions.icon(a.icon) },
				label = name,
				value = run_value(a),
				act = if bound.can_run() then { what = "run_action", action = a } else nil,
				trail = { icon = "settings-minimalistic", label = locale.s("panel.home.edit_action"), act = { what = "edit_action", action = a } },
			})
		end
		for _, d in bound.imports:get() do
			table.insert(out, {
				key = `import-{d.name}`,
				icon = { name = Actions.icon(d.icon) },
				label = d.name,
				value = { kind = "text", text = locale.s("panel.home.import") },
				act = { what = "edit_action", action = { name = d.name, command = d.command, icon = d.icon, setup = d.setup } },
			})
		end
		local issue = if s and s.snapshot then s.snapshot.issue else nil
		if issue then
			table.insert(out, {
				key = "actions-issue",
				icon = { name = "info-circle" },
				label = issue,
				hint = issue,
			})
		end
		table.insert(out, {
			key = "add-action",
			icon = { asset = "astra-plus" },
			label = locale.s("panel.home.add_action"),
			act = { what = "edit_action" },
		})
		return out
	end)

	local slot_session = ui.state("sidecar_slot_session", nil :: any)
	ui.effect(function()
		local c = chat:get()
		local now = if c and (c.live_branch or "") ~= "" then { session = c.id } else nil
		if ui.json.encode(slot_session:peek()) ~= ui.json.encode(now) then
			slot_session:set(now)
		end
	end)

	local function run(act: { [string]: any })
		local sid = session()
		local what = act.what
		if what == "subagent" and sid then
			lanes.open_subagent(sid, act.sub)
		elseif what == "child" and sid then
			lanes.open(sid, { kind = "side_chat", id = act.id, title = act.title })
		elseif what == "more" then
			beside_all:set(not beside_all:peek())
		elseif what == "stop_task" then
			stop_task(act.id)
		elseif what == "stop_all" then
			local s = store.session:peek()
			if s and s.id == sid then
				Pills.stop_all(Pills.window_runs(), s)
			end
		elseif what == "retry" then
			bound.retry()
		elseif what == "run_action" then
			bound.run(act.action)
		elseif what == "edit_action" then
			bound.edit(act.action)
		end
	end

	local function branch_slot(surface: string): Node
		return ui.row {
			id = `sidecar-{surface}-branch-slot`,
			shrink = 0,
			items = "center",
			provide = { panel_branch = slot_session },
			ui.presence(function(): string
				return session() or ""
			end, function(sid: string): Node
				return if sid == "" then ui.div {} else ui.slot("zeron.panel.branch", { direction = "row", items = "center" })
			end, { direction = "row", items = "center" }),
		}
	end

	return {
		run = run,
		tick = tick,
		refresh = function()
			tick:set(tick:peek() + 1)
		end,
		branch_slot = branch_slot,
		session = session,
		workspace = function(): { Entry }
			return workspace:get()
		end,
		beside = function(): { Entry }
			return beside:get()
		end,
		previous = function(): { Entry }
			return previous:get()
		end,
		foot = function(): { Entry }
			return foot:get()
		end,
		failed = function(): number
			return failed:get()
		end,
		running = function(): number
			return running:get()
		end,
		previous_count = function(): number
			return #split:get().previous
		end,
		actions = function(): { Entry }
			return actions:get()
		end,
		actions_shown = function(): boolean
			local s = bound.now()
			return bound.place() ~= nil and not (s and s.state == "unsupported")
		end,
		slot_session = slot_session,
		dialogs = bound.dialogs,
	} :: any
end


local PARTS = {
	workspace = { heading = "sidecar.workspace", icon = "astra-laptop" },
	beside = { heading = "sidecar.beside", icon = "lucide-message-square" },
	actions = { heading = "sidecar.actions", icon = "lucide-box" },
}

local function running_words(n: number): string
	return if n == 1 then locale.s("sidecar.running_one") else locale.s("sidecar.running", { n = n })
end

local function session_id(): string?
	local s = store.session:get()
	return if s then s.id else nil
end

local function heading(part: string, count: (() -> number)?, fan: Node?): Node
	return ui.row {
		w_full = true,
		h = ui.raw(19, "a Sidecar heading"),
		px = "$space.2",
		items = "center",
		justify = "between",
		ui.row {
			grow = 1,
			basis = 0,
			min_w = 0,
			gap = "$space.2",
			items = "center",
			pr = "$space.2",
			ui.text { locale.t(PARTS[part].heading), min_w = 0, truncate = true, text_size = "$font.size.sm", color = HEADING },
			fan,
		},
		if count
			then ui.show(function(): boolean
				return count() > 0
			end, ui.text {
				function(): string
					return if fan then locale.s("ds.pills.shells_running", { n = count() }) else running_words(count())
				end,
				class = "sidecar-running",
				literal = true,
				shrink = 0,
				text_size = "$font.size.xs",
				color = HEADING,
			})
			else nil,
	}
end

local function workspace_section(data: any, surface: string, after: (() -> ())?): Node
	return ui.column {
		id = `sidecar-{surface}-section-workspace`,
		role = "group",
		label = locale.t("sidecar.workspace"),
		w_full = true,
		shrink = 0,
		pt = "$space.2",
		pb = "$space.1",
		heading("workspace"),
		rows(data.workspace, surface, data, after),
		ui.presence(function(): string
			return data.session() or ""
		end, function(sid: string): Node
			if sid == "" then
				return ui.div {}
			end
			return ui.column {
				id = `sidecar-{surface}-details`,
				w_full = true,
				provide = { panel_details = data.slot_session },
				ui.slot("zeron.panel.details", { w_full = true }),
			}
		end, { w_full = true }),
	}
end

local function fold_row(data: any, surface: string): Node
	return ui.show(function(): boolean
		return data.previous_count() > 0
	end, ui.row {
		id = `sidecar-{surface}-fold`,
		role = "button",
		label = function(): string
			return locale.s("sidecar.previous", { n = data.previous_count() })
		end,
		expanded = function(): boolean
			return previous_open:get()
		end,
		focusable = true,
		cursor = "pointer",
		w_full = true,
		h = ui.raw(28, "a Sidecar row"),
		shrink = 0,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		rounded = "$radius.md",
		hover = { bg = "$hover" },
		focus = control.ring,
		on_click = function()
			previous_open:set(not previous_open:peek())
		end,
		on_key = function(e: any): boolean?
			if e.key == "Enter" or e.key == " " then
				previous_open:set(not previous_open:peek())
				return true
			end
			return nil
		end,
		ui.text {
			function(): string
				return locale.s("sidecar.previous", { n = data.previous_count() })
			end,
			literal = true,
			shrink = 0,
			text_size = "$font.size.sm",
			color = FAINT,
		},
		ui.div { grow = 1, h = 1, bg = "$border.subtle" },
		ui.show(function(): boolean
			return data.failed() > 0
		end, ui.text {
			function(): string
				return locale.s("sidecar.failed", { n = data.failed() })
			end,
			literal = true,
			shrink = 0,
			text_size = "$font.size.sm",
			color = "$danger",
		}),
		ui.div {
			shrink = 0,
			color = FAINT,
			rotate = function(): number
				return if previous_open:get() then 180 else 0
			end,
			icon_at("lucide-chevron-down", 12),
		},
	})
end

local function beside_section(data: any, surface: string, after: (() -> ())?): Node
	local function nothing(): boolean
		return #data.beside() == 0 and data.previous_count() == 0
	end
	return ui.column {
		id = `sidecar-{surface}-section-beside`,
		role = "group",
		label = function(): string
			local n = data.running()
			local head = locale.s("sidecar.beside")
			return if n > 0 then `{head} · {running_words(n)}` else head
		end,
		w_full = true,
		shrink = 0,
		pt = "$space.2",
		pb = "$space.1",
		heading("beside", data.running, ui.show(function(): boolean
			return Pills.fan_of(session_id(), nil) ~= nil
		end, Pills.Fan {
			name = `sidecar-{surface}-fan`,
			chat = session_id,
			size = 16,
			tiles = function(): { any }
				return Pills.fan_of(session_id(), nil) or {}
			end,
			place = { side = "left", align = "start", offset = 8 },
		})),
		rows(data.beside, surface, data, after),
		ui.show(nothing, ui.row {
			id = `sidecar-{surface}-empty`,
			w_full = true,
			h = ui.raw(28, "a Sidecar row"),
			px = "$space.2",
			items = "center",
			ui.text { locale.t("sidecar.nothing"), text_size = "$font.size.sm", color = FAINT },
		}),
		fold_row(data, surface),
		rows(data.previous, surface, data, after),
		rows(data.foot, surface, data, after),
	}
end

local function actions_section(data: any, surface: string, after: (() -> ())?): Node
	return ui.show(data.actions_shown, ui.column {
		id = `sidecar-{surface}-section-actions`,
		role = "group",
		label = locale.t("sidecar.actions"),
		w_full = true,
		shrink = 0,
		pt = "$space.2",
		pb = "$space.1",
		heading("actions"),
		rows(data.actions, surface, data, after),
	})
end

local function section(part: string, data: any, surface: string, after: (() -> ())?): Node
	if part == "workspace" then
		return workspace_section(data, surface, after)
	elseif part == "beside" then
		return beside_section(data, surface, after)
	end
	return actions_section(data, surface, after)
end


export type Props = {
	session: () -> string?,
	panel_open: () -> boolean,
	full_w: () -> number,
	close_panel: () -> (),
	open_panel: () -> (),
	root_ref: any?,
}

local function panel(data: any, railed: () -> boolean, set_collapsed: (boolean) -> (), take_focus: () -> boolean, fold_ref: any): Node
	local collapse_over = ui.use_state(false)
	return ui.column {
		id = "sidecar-panel",
		absolute = true,
		left = 0,
		top = ui.raw(TOP, "under the bar, as Zeron's column"),
		bottom = 0,
		w = M.WIDTH,
		pr = "$space.2",
		opacity = motion.tween(ui.computed(function(): number
			return if railed() then 0 else 1
		end), FADE),
		x = motion.tween(ui.computed(function(): number
			return if railed() then 18 else 0
		end), LAYOUT),
		pointer_events = function(): any
			return if railed() then "none" else "auto"
		end,
		ui.row {
			w_full = true,
			h = 24,
			shrink = 0,
			pl = "$space.3",
			pr = ui.raw(10, "the chevrons' place"),
			items = "center",
			justify = "between",
			ui.text { locale.t("sidecar.live_state"), text_size = "$font.size.md", weight = "$font.weight.semibold", color = "$text.primary" },
			ui.button {
				id = "sidecar-collapse",
				ref = fold_ref,
				role = "button",
				label = locale.t("sidecar.fold"),
				on_mount = function(el: Element)
					if take_focus() then
						el:focus()
					end
				end,
				w = 20,
				h = 20,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.sm",
				cursor = "pointer",
				bg = "#00000000",
				hover = { bg = "$hover" },
				focus = control.ring,
				color = function(): string
					return if collapse_over:get() then "$text.primary" else FAINT
				end,
				on_hover = function(inside: boolean)
					collapse_over:set(inside)
				end,
				on_click = function()
					set_collapsed(true)
				end,
				icon_at("astra-chevrons-right", 12),
			},
		},
		ui.scroll {
			id = "sidecar-scroll",
			w_full = true,
			grow = 1,
			min_h = 0,
			pt = ui.raw(2, "under the header"),
			px = "$space.1",
			pb = "$space.1",
			overscroll = "contain",
			fade = { 12, 12 },
			fade_overflow = true,
			section("workspace", data, "panel"),
			section("beside", data, "panel"),
			section("actions", data, "panel"),
		},
	}
end

local function rail_button(id: string, label: () -> string, glyph: Node, on: () -> boolean, on_click: () -> (), extra: Node?): Node
	return ui.button {
		id = id,
		role = "button",
		label = label,
		relative = true,
		w = RAIL_BUTTON,
		h = RAIL_BUTTON,
		shrink = 0,
		items = "center",
		justify = "center",
		rounded = "$radius.md",
		cursor = "pointer",
		bg = function(): string
			return if on() then "$hover" else "#00000000"
		end,
		color = function(): string
			return if on() then "$text.primary" else "$text.secondary"
		end,
		hover = { bg = "$hover" },
		focus = control.ring,
		tooltip = Tooltip { label },
		tooltip_delay = RAIL_TOOLTIP_MS,
		tooltip_placement = { side = "left", align = "center", offset = 6 },
		on_click = on_click,
		glyph,
		extra,
	}
end

local function rail(data: any, railed: () -> boolean, up: State<string?>, expand: () -> ()): Node
	local function popover_for(part: string, index: number, label: () -> string, extra: Node?): Node
		-- (A state, not a function of `up`: a press on its own button then closes it, once, rather than closing and opening it again.)
		local open = ui.use_state(up:peek() == part)
		ui.effect(function()
			if up:get() ~= part and open:peek() then
				open:set(false)
			end
		end)
		ui.effect(function()
			local on = open:get()
			if on and up:peek() ~= part then
				up:set(part)
			elseif not on and up:peek() == part then
				up:set(nil)
			end
		end)
		local trigger = rail_button(`sidecar-rail-{part}`, label, icon_at(PARTS[part].icon, 15), function(): boolean
			return open:get()
		end, ui.toggle(open), extra)
		local shown = ui.use_state(0)
		-- (A scroll is 240 tall unless it's told: the card is as tall as its rows, up to the window.)
		local content_h = ui.use_state(0)
		return ui.popover(Menu.panel {
			id = `sidecar-popover-{part}`,
			class = "sidecar-popover",
			open = open,
			anchor = trigger,
			label = label,
			autofocus = false,
			placement = { side = "left", align = "start", offset = 8 },
			w = POPOVER_W,
			rounded = ui.raw(12, "as Zeron's card"),
			opacity = motion.tween(shown, POPOVER_FADE),
			x = motion.tween(ui.computed(function(): number
				return 10 * (1 - shown:get())
			end), POPOVER_SLIDE),
			y = -6,
			build = function(): Node
				shown:set(0)
				ui.after(0, function()
					shown:set(1)
				end)
				return ui.scroll {
					id = "sidecar-popover-scroll",
					w_full = true,
					h = function(): number
						return math.min(content_h:get(), math.max(art.window:get().h - (48 + (index + 1) * (RAIL_BUTTON + RAIL_GAP) - 6) - 16, 120))
					end,
					px = "$space.1",
					overscroll = "contain",
					ui.column {
						w_full = true,
						pb = "$space.1",
						on_resize = function(size: any)
							content_h:set(size.h)
						end,
						section(part, data, "popover", function()
							up:set(nil)
						end),
					},
				}
			end,
		})
	end
	return ui.column {
		id = "sidecar-rail",
		absolute = true,
		top = ui.raw(TOP, "under the bar, as Zeron's column"),
		right = (M.RAIL - RAIL_BUTTON) / 2,
		w = RAIL_BUTTON,
		gap = "$space.1",
		opacity = motion.tween(ui.computed(function(): number
			return if railed() then 1 else 0
		end), FADE),
		x = motion.tween(ui.computed(function(): number
			return if railed() then 0 else 12
		end), LAYOUT),
		pointer_events = function(): any
			return if railed() then "auto" else "none"
		end,
		rail_button("sidecar-expand", function(): string
			return locale.s("sidecar.expand")
		end, icon_at("lucide-chevrons-left", 15), function(): boolean
			return false
		end, expand),
		popover_for("workspace", 0, function(): string
			return locale.s("sidecar.workspace")
		end),
		popover_for("beside", 1, function(): string
			local n = data.running()
			local head = locale.s("sidecar.beside")
			return if n > 0 then `{head} · {running_words(n)}` else head
		end, ui.show(function(): boolean
			return data.running() > 0
		end, ui.row {
			id = "sidecar-rail-badge",
			absolute = true,
			top = 1,
			right = 0,
			min_w = 14,
			h = 14,
			px = ui.raw(3, "the badge's room"),
			rounded = "$radius.full",
			items = "center",
			justify = "center",
			bg = "$accent",
			pointer_events = "none",
			ui.text {
				function(): string
					return tostring(data.running())
				end,
				literal = true,
				text_size = ui.raw(9.5, "the badge's count"),
				weight = "$font.weight.semibold",
				color = "$accent.text",
			},
		})),
		ui.show(data.actions_shown, popover_for("actions", 2, function(): string
			return locale.s("sidecar.actions")
		end)),
	}
end

function M.width_for(railed: boolean): number
	return if railed then M.RAIL else M.WIDTH
end

function M.Column(props: Props): Node
	-- (Here, not as the module loads: an effect made under the first column's load would go with that column.)
	ui.effect(function()
		local kind = ui.theme.kind()
		local light = kind == "light" or kind == "high-contrast-light"
		ui.theme.tokens { zeron = { sidecar = {
			branch = if light then "#7f22fe" else "#c4b4ff",
			working = "#2b7fff",
			done = "#00bc7d",
			asking = if light then "#4f39f6" else "#a3b3ff",
			faint = if light then "#797980" else "#858589",
			heading = if light then "#626269" else "#a3a3a3",
		} } }
	end)
	local railed_now = ui.computed(function(): boolean
		return M.collapsed:get() or props.panel_open() or props.full_w() <= M.AUTO_RAIL_AT
	end)
	local function railed(): boolean
		return railed_now:get()
	end
	local up = ui.use_state(nil :: string?)
	local expanding = false
	local fold_ref = ui.ref()
	ui.effect(function()
		if not railed() and up:peek() ~= nil then
			up:set(nil)
		end
	end)
	local panel_built = ui.use_state(not railed_now:peek())
	local rail_built = ui.use_state(railed_now:peek())
	local flip_seq = 0
	ui.effect(function()
		local r = railed()
		flip_seq += 1
		local mine = flip_seq
		if r then
			rail_built:set(true)
		else
			panel_built:set(true)
		end
		ui.after(motion.ms(FADE.duration), function()
			if mine == flip_seq then
				if r then
					panel_built:set(false)
				else
					rail_built:set(false)
				end
			end
		end)
	end)
	local data = make_data { session = props.session, open_panel = props.open_panel }
	ui.effect(function()
		local sid = props.session()
		if panel_state.sidecar_holds:peek() ~= sid then
			panel_state.sidecar_holds:set(sid)
		end
	end)
	ui.on_cleanup(function()
		panel_state.sidecar_holds:set(nil)
	end)
	return ui.div {
		ref = props.root_ref,
		id = "sidecar",
		role = "group",
		label = locale.t("sidecar.live_state"),
		relative = true,
		shrink = 0,
		h_full = true,
		w = motion.tween(ui.computed(function(): number
			return M.width_for(railed())
		end), LAYOUT),
		overflow = "hidden",
		pointer_events = "auto",
		on_hover = function(inside: boolean)
			if inside then
				data.refresh()
			end
		end,
		ui.presence(function(): string
			return if panel_built:get() then "on" else "off"
		end, function(which: string): Node
			return if which == "on"
				then panel(data, railed, function(on: boolean)
					M.collapsed:set(on)
				end, function(): boolean
						local was = expanding
						expanding = false
						return was
					end, fold_ref)
				else ui.div {}
		end, { absolute = true, left = 0, top = 0, bottom = 0, w = M.WIDTH, pointer_events = "none" }),
		ui.presence(function(): string
			return if rail_built:get() then "on" else "off"
		end, function(which: string): Node
			return if which == "on"
				then rail(data, railed, up, function()
					up:set(nil)
					if props.panel_open() then
						props.close_panel()
					end
					M.collapsed:set(false)
					if props.full_w() <= M.AUTO_RAIL_AT then
						return
					end
					-- (Expand goes as the rail does; the keyboard stays here, not at the window's next Tab stop.)
					local fold = if panel_built:peek() then fold_ref:peek() else nil
					if fold then
						fold:focus()
					else
						expanding = true
					end
				end)
				else ui.div {}
		end, { absolute = true, right = 0, top = 0, bottom = 0, w = M.RAIL, pointer_events = "none" }),
		data.dialogs,
	}
end

return M
