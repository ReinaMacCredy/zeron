local ds = require("@ds")
local Icon = require("@ds/icon")
local motion = require("@ds/motion")
local AgentFan = require("@ds/agent_fan")
local P = require("@ds/pills")
local FadeText = require("@ds/fade_text")
local Menu = require("@ds/menu")
local locale = require("@ds/locale")
local T = require("@ds/transcript")
local RowProps = require("@app/screens/chat/row_props")
local sessions = require("@account/sessions")
local store = require("@app/screens/chat/store")
local lanes = require("@app/screens/chat/lanes")
local Roster = require("@app/screens/chat/roster")
local Waiting = require("@app/screens/chat/waiting")
local typing = require("@app/screens/chat/typing")
local glass = require("@ds/glass")
local route = require("@app/screens/shell/route")

local M = {}

local ROW_H = 34
local LIST_MAX = 400
local SHELLS_W, DETAIL_W, LOG_H = 420, 600, 300
local PLACE = { side = "top", align = "start", offset = 8 }

local usage = ui.shared("chat.roster_usage", {} :: { [string]: any })
local shell_output = ui.shared("chat.shell_output", {} :: { [string]: any })
local published_subs = ui.shared("chat.subagents", {} :: { any })

local cards: { [any]: boolean } = {}
function M.dismiss(): boolean
	local any = false
	for card in cards do
		if card:peek() then
			card:set(false)
			any = true
		end
	end
	return any
end

local function tracked(open: any)
	cards[open] = true
	ui.on_cleanup(function()
		cards[open] = nil
	end)
end

-- The roster and the shells menu take no focus (a popover would keep the composer's keys from it), so they close on a press outside as gpui's on_mouse_down_out.
local floating: { [any]: boolean } = {}
local press_inside = false
local pressed_out = false

local function press_in()
	press_inside = true
	ui.after(0, function()
		press_inside = false
	end)
end

function M.press_outside()
	if press_inside then
		press_inside = false
		return
	end
	pressed_out = true
	ui.after(50, function()
		pressed_out = false
	end)
	for card in floating do
		if card:peek() then
			card:set(false)
		end
	end
end

local function keys_back(open: any, keys: () -> ())
	local was = open:peek()
	ui.effect(function()
		local now = open:get()
		if was and not now then
			ui.after(0, function()
				if not pressed_out then
					keys()
				end
			end)
		end
		was = now
	end)
end

local function typed(e: any): boolean
	return utf8.len(e.key or "") == 1 and not (e.cmd or e.ctrl or e.meta or e.editing)
end

function M.window_bind(): any
	return {
		id = function(): string?
			local s = store.session:get()
			return if s then s.id else nil
		end,
		session = function(): any
			return store.session:get()
		end,
		subs = function(): { any }
			return published_subs:get()
		end,
	}
end

-- The host holds one set a window, and a closing card's cleanup runs after the next card has said what it wants.
local function wants(send: ({ any }) -> ()): () -> (({ [string]: any }) -> ())
	local by: { [any]: { [string]: any } } = {}
	local sent = ""
	local function flush()
		local all: { [string]: any } = {}
		for _, items in by do
			for key, item in items do
				all[key] = item
			end
		end
		local keys: { string } = {}
		for key in all do
			table.insert(keys, key)
		end
		table.sort(keys)
		local joined = table.concat(keys, "\0")
		if joined ~= sent then
			sent = joined
			local out: { any } = {}
			for _, key in keys do
				table.insert(out, all[key])
			end
			send(out)
		end
	end
	return function()
		local me = {}
		ui.on_cleanup(function()
			by[me] = nil
			flush()
		end)
		return function(items: { [string]: any })
			by[me] = items
			flush()
		end
	end
end

local want_docs = wants(function(docs: { any })
	store.roster_watch(docs)
end)
local want_shells = wants(function(shells: { any })
	store.shell_watch(shells)
end)

local function chat_row(id: string?): any
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

function M.runs_of(bind: any): any
	return ui.computed(function(): any
		local id = bind.id()
		local s = bind.session()
		local row = chat_row(id)
		return Roster.runs {
			chat = id,
			subs = bind.subs(),
			children = sessions.children(id),
			usage = usage:get(),
			turn_at = if s then s.turn_at else nil,
			harness = if s then s.harness else nil,
			agent = if row then (row :: any).mark or row.agent else nil,
			icon = if row then (row :: any).agent_icon else nil,
			reasoning = if s then s.effort else nil,
		}
	end)
end

-- The same table until they change, so a list of every chat can read it without following that chat's session.
local window = M.window_bind()
local window_runs = M.runs_of(window)
function M.window_runs(): any
	return window_runs:get()
end
local window_id = ui.computed(window.id)
local window_busy = ui.computed(function(): boolean
	return Roster.working(window_runs:get().listed) > 0
end)
local fan_seen: any = { id = nil, tiles = {}, sig = "" }
local window_fan = ui.computed(function(): any
	local id = window.id()
	local tiles = Roster.tiles(window_runs:get().listed)
	local parts = { id or "" }
	for _, t in tiles do
		table.insert(parts, `{t.key}:{t.asking}:{t.count}`)
	end
	local sig = table.concat(parts, "\0")
	if sig ~= fan_seen.sig then
		fan_seen = { id = id, tiles = tiles, sig = sig }
	end
	return fan_seen
end)

function M.fan_of(id: string?, row_fan: { any }?): { any }?
	local open = window_fan:get()
	if id and open.id == id then
		return if #open.tiles > 0 then open.tiles else row_fan
	end
	return row_fan
end

local function stop_run(r: any)
	local id = r.chat
	if not id then
		return
	end
	local function fail()
		ds.toast.show { title = locale.s("chat.composer.couldnt_stop_task"), tone = "danger" }
	end
	local p = if r.delegated then store.cancel_delegate(id, r.id) else store.stop_task(id, r.id)
	if p then
		p:after(function() end, fail)
	else
		fail()
	end
end

M.stoppable_of = Roster.stoppable

function M.stop_all(runs: any, session: any)
	local all = M.stoppable_of(runs, session)
	for _, r in all.agents do
		stop_run(r)
	end
	for _, t in all.shells do
		local p = if session then store.stop_task(session.id, t.id) else nil
		if p then
			p:after(function() end, function() end)
		end
	end
end

local function open_row(r: any)
	local id = r.chat
	if not id then
		return
	end
	if r.child then
		lanes.open(id, { kind = "side_chat", id = r.doc, title = r.title })
	else
		lanes.open_subagent(id, { ref = r.doc, title = r.title, status = if r.frozen then "done" else "running" })
	end
end

local REASONING = { minimal = true, low = true, medium = true, high = true, xhigh = true, xHigh = true, max = true, ultra = true, ultracode = true, ultrathink = true }

local function reasoning_word(level: string): string
	return if REASONING[level] then locale.s(`ds.pills.reasoning.{string.lower(level)}`) else level
end

local function run_row(r: any, clock: any, on_open: (any) -> ()): Node
	return ui.row {
		class = "pills-roster-row",
		h = ROW_H,
		w_full = true,
		gap = "$space.2",
		items = "center",
		pl = ui.raw(12 + 16 * r.depth, "background.rs indent by depth"),
		pr = ui.raw(12, "background.rs row"),
		cursor = "pointer",
		hover = { bg = "$hover" },
		role = "button",
		label = r.title,
		on_click = function()
			on_open(r)
		end,
		P.Glyph { state = function()
			return r.state
		end, frame = clock.frame },
		FadeText { r.title, literal = true, grow = 1, basis = 0, min_w = 0, text_size = ui.raw(13, "background.rs row"), color = "$text.primary" },
		if r.tag then P.ModelTag { tag = r.tag, mark = r.mark, icon = r.icon } else nil,
		if r.reasoning then P.Chip { key = locale.t("ds.pills.chip.thinking"), value = reasoning_word(r.reasoning) } else nil,
		if r.tokens then P.Chip { key = locale.t("ds.pills.chip.tokens"), value = Roster.compact(r.tokens) } else nil,
		ui.text {
			function(): string
				return Roster.state_text(r, clock.now:get())
			end,
			shrink = 0,
			tabular = true,
			wrap = false,
			text_size = "$font.size.xs",
			color = function(): string
				return if r.asking then "$warning" else "$zeron.composer.faint"
			end,
		},
		ui.div {
			w = 20,
			h = 20,
			shrink = 0,
			if r.stoppable
				then P.Stop { label = locale.s("ds.composer_dock.stop_named", { name = r.title }), on_click = function()
					stop_run(r)
				end }
				else nil,
		},
	}
end

local detail_holder: any = nil

local Detail = ui.component(function(props: { runs: any, run_id: string, clock: any, on_back: () -> (), on_done: () -> () }): Node
	local on_back, on_done, clock = props.on_back, props.on_done, props.clock
	local last: any = nil
	local run = ui.computed(function(): any
		for _, r in props.runs:get().listed do
			if r.id == props.run_id then
				last = r
				return r
			end
		end
		return last
	end)
	local slot = "pill-detail"
	local view = lanes.view(slot)
	local first = run:peek()
	local me = {}
	detail_holder = me
	lanes.watch(slot, first.doc, first.frozen == true, first.chat)
	ui.on_cleanup(function()
		if detail_holder == me then
			detail_holder = nil
			lanes.watch(slot, nil)
		end
	end)
	local hide = ui.use_state(false)
	local messages = ui.computed(function(): { any }
		local all = view.messages:get()
		if not hide:get() then
			return all
		end
		local out: { any } = {}
		for _, m in all do
			local row = table.clone(m)
			local blocks: { any } = {}
			for _, b in m.blocks or {} do
				if b.kind == "tools" then
					local kept: { any } = {}
					for _, t in b.tools or {} do
						if t.kind ~= "thought" then
							table.insert(kept, t)
						end
					end
					if #kept > 0 then
						local group = table.clone(b)
						group.tools = kept
						local rest = if group.summary then string.match(group.summary, "^Thought process · (.+)$") else nil
						if rest then
							group.summary = string.upper(string.sub(rest, 1, 1)) .. string.sub(rest, 2)
						end
						table.insert(blocks, group)
					end
				else
					table.insert(blocks, b)
				end
			end
			row.blocks = blocks
			table.insert(out, row)
		end
		return out
	end)
	local tools = ui.computed(function(): number
		local n = 0
		for _, m in view.messages:get() do
			for _, t in m.tools or {} do
				if t.kind ~= "thought" then
					n += 1
				end
			end
		end
		return n
	end)
	local function link(words: any, ink: string, on_click: () -> (), id: string?): Node
		return ui.button {
			bg = "#00000000",
			id = id,
			label = words,
			color = ink,
			text_size = ui.raw(12, "background.rs detail_link"),
			hover = { opacity = 0.75 },
			on_click = on_click,
			ui.text { words, wrap = false },
		}
	end
	local function copy_text(): string
		local lines: { string } = {}
		for _, m in view.messages:peek() do
			if m.text and m.text ~= "" then
				table.insert(lines, m.text)
			end
			for _, t in m.tools or {} do
				table.insert(lines, `{t.verb or ""} {t.target or ""}`)
			end
		end
		return table.concat(lines, "\n")
	end
	local offset = ui.use_state(T.PAST_END)
	local keep = ui.use_state(nil :: any)
	return ui.column {
		id = "pills-detail",
		w_full = true,
		p = ui.raw(6, "background.rs detail card"),
		ui.row {
			gap = "$space.2",
			items = "center",
			pr = "$space.1",
			pb = ui.raw(6, "background.rs detail_head"),
			ui.button {
				bg = "#00000000",
				id = "pills-detail-back",
				label = locale.t("ds.pills.back"),
				w = 24,
				h = 24,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.sm",
				color = "$text.secondary",
				hover = { bg = "$hover" },
				on_click = on_back,
				Icon { name = "alt-arrow-left", size = 12 },
			},
			P.Glyph { state = function()
				return run:get().state
			end, frame = clock.frame },
			FadeText { function(): string
				return run:get().title
			end, literal = true, grow = 1, basis = 0, min_w = 0, text_size = ui.raw(13, "background.rs detail_head"), color = "$text.primary" },
			ui.text {
				function(): string
					return Roster.state_text(run:get(), clock.now:get())
				end,
				shrink = 0,
				wrap = false,
				tabular = true,
				text_size = ui.raw(12, "background.rs detail_head"),
				color = "$text.secondary",
			},
			ui.div {
				min_w = 20,
				h = 20,
				shrink = 0,
				ui.show(function(): boolean
					return run:get().stoppable == true
				end, P.Stop { id = "pills-detail-stop", label = locale.s("ds.composer_dock.stop_named", { name = first.title }), on_click = function()
					stop_run(run:peek())
				end }),
			},
		},
		ui.row {
			wrap = true,
			gap = ui.raw(6, "background.rs detail chips"),
			pl = ui.raw(36, "background.rs detail chips"),
			pb = "$space.2",
			if first.tag then P.ModelTag { tag = first.tag, mark = first.mark, icon = first.icon } else nil,
			if first.reasoning then P.Chip { key = locale.t("ds.pills.chip.thinking"), value = reasoning_word(first.reasoning) } else nil,
			ui.show(function(): boolean
				return run:get().tokens ~= nil
			end, P.Chip { key = locale.t("ds.pills.chip.tokens"), value = function(): string
				return Roster.compact(run:get().tokens or 0)
			end }),
			P.Chip { key = locale.t("ds.pills.chip.tools"), value = function(): string
				return tostring(tools:get())
			end },
		},
		ui.column {
			w_full = true,
			h = LOG_H,
			border = 1,
			border_sides = { 1, 0, 0, 0 },
			border_color = "$border",
			T.Transcript(RowProps.add({
				name = "pills.detail",
				measure = function(): number
					return DETAIL_W
				end,
				messages = messages,
				draft = view.draft,
				offset = offset,
				keep = keep,
			}, function(): string?
				return first.doc
			end, function(): any
				return view.session:get()
			end, function(): any
				return view.session:peek()
			end)),
		},
		ui.row {
			gap = ui.raw(14, "background.rs detail footer"),
			items = "center",
			px = "$space.2",
			pt = "$space.2",
			pb = ui.raw(2, "background.rs detail footer"),
			link(function(): string
				return locale.s(if hide:get() then "ds.pills.show_thinking" else "ds.pills.hide_thinking")
			end, "$text.secondary", function()
				hide:set(not hide:peek())
			end, "pills-detail-thinking"),
			ui.div { grow = 1 },
			link(locale.t("ds.pills.copy"), "$accent", function()
				ui.clipboard.write(copy_text())
			end, "pills-detail-copy"),
			link(locale.t("ds.pills.open_pane"), "$accent", function()
				on_done()
				open_row(run:peek())
			end, "pills-detail-open"),
		},
	}
end)

M.RosterBody = ui.component(function(props: { bind: any, clock: any?, on_close: () -> () }): Node
	local runs = M.runs_of(props.bind)
	local detail = ui.use_state(nil :: string?)
	local clock = props.clock or P.clock(function(): boolean
		return Roster.working(runs:get().listed) > 0
	end)
	local say_docs = want_docs()
	ui.effect(function()
		local docs: { [string]: string } = {}
		for _, r in runs:get().listed do
			if not r.child then
				docs[r.doc] = r.doc
			end
		end
		say_docs(docs)
	end)
	ui.effect(function()
		local id = detail:get()
		if id then
			for _, r in runs:get().listed do
				if r.id == id then
					return
				end
			end
			detail:set(nil)
		end
	end)
	local lines = ui.computed(function(): { any }
		return Roster.lines(runs:get().listed)
	end)
	local function stoppable(): { any }
		local out: { any } = {}
		for _, r in runs:get().agents do
			if r.stoppable then
				table.insert(out, r)
			end
		end
		return out
	end
	local on_open = function(r: any)
		if r.child then
			props.on_close()
			open_row(r)
		else
			detail:set(r.id)
		end
	end
	local offset = ui.use_state(0)
	local function total(): number
		return #lines:get() * ROW_H
	end
	local list = ui.column {
		id = "pills-roster",
		w_full = true,
		pb = ui.raw(4, "background.rs roster"),
		ui.row {
			h = 36,
			gap = "$space.2",
			items = "center",
			pl = ui.raw(12, "background.rs roster header"),
			pr = "$space.2",
			text_size = ui.raw(13, "background.rs roster header"),
			color = "$text.primary",
			FadeText {
				function(): string
					return Roster.heading(runs:get().listed)
				end,
				grow = 1,
				basis = 0,
				min_w = 0,
			},
			ui.show(function(): boolean
				return #stoppable() > 0
			end, ui.button {
				bg = "#00000000",
				id = "pills-roster-stop-all",
				label = locale.t("ds.pills.stop_all"),
				color = "$text.secondary",
				hover = { color = "$text.primary" },
				on_click = function()
					for _, r in stoppable() do
						stop_run(r)
					end
				end,
				ui.text { locale.t("ds.pills.stop_all"), wrap = false },
			}),
			ui.button {
				bg = "#00000000",
				id = "pills-roster-close",
				label = locale.t("ds.pills.close"),
				w = 20,
				h = 20,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.sm",
				color = "$text.secondary",
				hover = { bg = "$hover", color = "$text.primary" },
				on_click = props.on_close,
				Icon { name = "close", size = 12 },
			},
		},
		ui.div {
			w_full = true,
			h = function(): number
				return math.min(total(), LIST_MAX)
			end,
			ui.list {
				w_full = true,
				h_full = true,
				data = lines,
				key = function(l: any): string
					return l.key
				end,
				heights = function(): { number }
					local out: { number } = {}
					for i in lines:get() do
						out[i] = ROW_H
					end
					return out
				end,
				offset = offset,
				overscroll = "contain",
				role = "list",
				row = function(l: any): Node
					if l.kind == "previous" then
						return ui.row {
							h = ROW_H,
							pl = ui.raw(12, "background.rs row"),
							items = "center",
							ui.text { locale.s("ds.pills.previous", { n = l.n }), text_size = "$font.size.xs", color = "$text.secondary", wrap = false },
						}
					end
					return run_row(l.run, clock, on_open)
				end,
			},
		},
	}
	return ui.presence(function(): string
		return detail:get() or ""
	end, function(id: string): Node
		if id == "" then
			return list
		end
		for _, r in runs:peek().listed do
			if r.id == id then
				return Detail {
					runs = runs,
					run_id = id,
					clock = clock,
					on_back = function()
						detail:set(nil)
					end,
					on_done = props.on_close,
				}
			end
		end
		return list
	end, { w_full = true })
end)

function M.RosterFor(props: { anchor: Node, open: any, bind: any, place: any?, w: number? }): Node
	tracked(props.open)
	keys_back(props.open, typing.take)
	return ui.popover(Menu.panel {
		class = "pills-roster-panel",
		open = props.open,
		anchor = props.anchor,
		autofocus = false,
		w = props.w or 480,
		placement = props.place or PLACE,
		on_key = function(e: any): boolean
			if typed(e) then
				props.open:set(false)
			end
			return false
		end,
		build = function(): Node
			return P.while_open(props.open, function(): Node
				return M.RosterBody {
					bind = props.bind,
					on_close = function()
						props.open:set(false)
					end,
				}
			end)
		end,
	})
end

local function fan_words(tiles: { any }): string
	local n, k = 0, 0
	for _, t in tiles do
		n += t.count or 1
		k += if t.asking then 1 else 0
	end
	return Roster.fan_label(n, k)
end

M.Fan = ui.component(function(props: { chat: string | () -> string?, tiles: () -> { any }, words: (() -> { any })?, lead: ((number) -> Node)?, size: number?, place: any?, name: string? }): Node
	local open = ui.use_state(false)
	local function chat_of(): string?
		return if type(props.chat) == "function" then props.chat() else props.chat
	end
	local seen = window_id:peek()
	ui.effect(function()
		local now = window_id:get()
		if now ~= seen then
			seen = now
			if open:peek() then
				open:set(false)
			end
		end
	end)
	ui.effect(function()
		if not window_busy:get() and open:peek() then
			open:set(false)
		end
	end)
	return M.RosterFor {
		anchor = AgentFan {
			id = props.name,
			class = "agent-fan",
			tiles = props.tiles,
			lead = props.lead,
			size = props.size,
			label = function(): string
				return fan_words(if props.words then props.words() else props.tiles())
			end,
			on_click = function()
				local chat = chat_of()
				if window_id:peek() ~= chat then
					if chat then
						route.open_session(chat)
					end
				elseif open:peek() or window_busy:peek() then
					open:set(not open:peek())
				end
			end,
		},
		open = open,
		bind = window,
		place = props.place,
	}
end)

local function shell_line(task: string): string?
	local look = shell_output:get()[task]
	local lines = if look then look.lines else nil
	for i = #(lines or {}), 1, -1 do
		local trimmed = string.match(lines[i], "^%s*(.-)%s*$") or ""
		if trimmed ~= "" then
			return trimmed
		end
	end
	return nil
end

local ShellDetail = ui.component(function(props: { task: any, stopping: any, can_stop: () -> boolean, clock: any, on_stop: (string) -> (), on_back: () -> () }): Node
	local task, stopping, on_stop, on_back, now = props.task, props.stopping, props.on_stop, props.on_back, props.clock.now
	local offset = ui.use_state(0)
	local lines = ui.computed(function(): { string }
		local look = shell_output:get()[task.id]
		return if look then look.lines else {}
	end)
	local LINE = 16
	local pinned = ui.use_state(true)
	local function most(): number
		return math.max(0, #lines:peek() * LINE - LOG_H)
	end
	ui.effect(function()
		local _ = lines:get()
		if pinned:peek() then
			ui.after(0, function()
				offset:set(most())
			end)
		end
	end)
	ui.effect(function()
		local at = offset:get()
		local near = at >= most() - 4
		if pinned:peek() ~= near then
			pinned:set(near)
		end
	end)
	return ui.column {
		id = "pills-shell-detail",
		w_full = true,
		p = ui.raw(2, "background.rs shell detail"),
		ui.row {
			gap = "$space.2",
			items = "center",
			pr = "$space.1",
			pb = "$space.2",
			ui.button {
				bg = "#00000000",
				id = "pills-detail-back",
				label = locale.t("ds.pills.back"),
				w = 24,
				h = 24,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.sm",
				color = "$text.secondary",
				hover = { bg = "$hover" },
				on_click = on_back,
				Icon { name = "alt-arrow-left", size = 12 },
			},
			FadeText { task.detail or task.name, literal = true, grow = 1, basis = 0, min_w = 0, font = "$font.mono", text_size = ui.raw(13, "background.rs detail_head"), color = "$text.primary" },
			ui.text {
				function(): string
					return Roster.elapsed(now:get() - (task.since or now:get()))
				end,
				shrink = 0,
				tabular = true,
				wrap = false,
				text_size = ui.raw(12, "background.rs detail_head"),
				color = "$text.secondary",
			},
			M.ShellStop(task, stopping, props.can_stop, on_stop),
		},
		ui.div {
			relative = true,
			w_full = true,
			h = LOG_H,
			rounded = "$radius.md",
			bg = "$zeron.secondary.bg",
			ui.show(function(): boolean
				return #lines:get() == 0
			end, ui.div {
				px = "$space.2",
				py = "$space.2",
				ui.text { locale.t("ds.pills.no_output"), id = "pills-shell-empty", text_size = ui.raw(10.5, "background.rs log"), font = "$font.mono", color = "$text.secondary" },
			}),
			ui.list {
				w_full = true,
				h_full = true,
				data = lines,
				heights = function(): { number }
					local out: { number } = {}
					for i in lines:get() do
						out[i] = LINE
					end
					return out
				end,
				offset = offset,
				overscroll = "contain",
				row = function(line: string): Node
					return ui.text { line, px = "$space.2", h = LINE, wrap = false, font = "$font.mono", text_size = ui.raw(10.5, "background.rs log"), color = "$text.secondary" }
				end,
			},
			ui.show(function(): boolean
				return not pinned:get()
			end, ui.button {
				id = "pills-shell-jump",
				label = locale.t("ds.pills.jump"),
				absolute = true,
				right = 10,
				bottom = 8,
				gap = "$space.1",
				items = "center",
				px = "$space.2",
				py = ui.raw(3, "background.rs jump"),
				rounded = "$radius.full",
				border = 1,
				border_color = "$border",
				bg = "$zeron.menu.solid",
				text_size = ui.raw(11, "background.rs jump"),
				color = "$text.secondary",
				hover = { color = "$text.primary" },
				on_click = function()
					offset:set(most())
				end,
				ui.text { locale.t("ds.pills.jump"), wrap = false },
				Icon { name = "alt-arrow-down", size = 10 },
			}),
		},
		ui.row {
			items = "center",
			px = "$space.2",
			pt = "$space.2",
			pb = ui.raw(2, "background.rs detail footer"),
			text_size = ui.raw(12, "background.rs detail footer"),
			FadeText { if task.detail then task.name else "", literal = true, grow = 1, basis = 0, min_w = 0, color = "$text.secondary" },
			ui.button {
				bg = "#00000000",
				id = "pills-detail-copy",
				label = locale.t("ds.pills.copy"),
				color = "$accent",
				hover = { opacity = 0.75 },
				on_click = function()
					ui.clipboard.write(table.concat(lines:peek(), "\n"))
				end,
				ui.text { locale.t("ds.pills.copy"), wrap = false },
			},
		},
	}
end)

function M.ShellStop(task: any, stopping: any, can_stop: () -> boolean, on_stop: (string) -> ()): Node
	return ui.show(can_stop, ui.show(function(): boolean
		return stopping:get()[task.id] == true
	end, ui.text { locale.t("ds.pills.stopping"), id = `pills-stopping-{task.id}`, shrink = 0, wrap = false, text_size = ui.raw(11.5, "background.rs stopping"), color = "$warning" }, P.Stop {
		label = locale.s("ds.composer_dock.stop_named", { name = task.name }),
		on_click = function()
			on_stop(task.id)
		end,
	}))
end

local ShellsBody = ui.component(function(props: { bind: any, tasks: any, detail: any, clock: any }): Node
	local bind, tasks, detail, clock = props.bind, props.tasks, props.detail, props.clock
	local stopping = ui.use_state({} :: { [string]: boolean })
	local function can_stop(): boolean
		local s = bind.session()
		return not s or s.shells_stoppable ~= false
	end
	local say_shells = want_shells()
	local session = bind.id()
	ui.effect(function()
		local shells: { [string]: any } = {}
		for _, t in tasks:get() do
			shells[t.id] = { session = session, task = t.id }
		end
		say_shells(shells)
	end)
	ui.effect(function()
		local live: { [string]: boolean } = {}
		for _, t in tasks:get() do
			live[t.id] = true
		end
		if detail:peek() and not live[detail:peek()] then
			detail:set(nil)
		end
		local now = stopping:peek()
		local kept: { [string]: boolean } = {}
		local moved = false
		for id in now do
			if live[id] then
				kept[id] = true
			else
				moved = true
			end
		end
		if moved then
			stopping:set(kept)
		end
	end)
	local function on_stop(task: string)
		local id = session
		if not id then
			return
		end
		local now = table.clone(stopping:peek())
		now[task] = true
		stopping:set(now)
		local function give_back()
			local after = table.clone(stopping:peek())
			after[task] = nil
			stopping:set(after)
		end
		store.stop_task(id, task):after(function() end, function()
			give_back()
			ds.toast.show { title = locale.s("chat.composer.couldnt_stop_task"), tone = "danger" }
		end)
		ui.after(15000, give_back)
	end
	local tick = clock.now
	local list = ui.column {
		id = "pills-shells",
		w_full = true,
		ui.row {
			px = "$space.2",
			pt = ui.raw(6, "background.rs shells heading"),
			pb = "$space.1",
			text_size = ui.raw(11, "background.rs shells heading"),
			color = "$text.secondary",
			ui.text { locale.t("ds.pills.shells_title"), grow = 1, wrap = false },
			ui.text { function(): string
				return locale.s("ds.pills.shells_running", { n = #tasks:get() })
			end, wrap = false },
		},
		ui.each(tasks, function(task: any): Node
			return ui.column {
				class = "pills-shell-row",
				gap = "$space.1",
				px = "$space.2",
				py = ui.raw(7, "background.rs shell row"),
				rounded = "$zeron.composer.tray.row_radius",
				cursor = "pointer",
				hover = { bg = "$hover" },
				role = "button",
				label = task.detail or task.name,
				on_click = function()
					detail:set(task.id)
				end,
				ui.row {
					gap = "$space.2",
					items = "center",
					P.Dots { frame = clock.frame },
					FadeText { task.detail or task.name, literal = true, grow = 1, basis = 0, min_w = 0, font = "$font.mono", text_size = ui.raw(11.5, "background.rs shell row"), color = "$text.primary" },
					ui.text {
						function(): string
							return Roster.elapsed(tick:get() - (task.since or tick:get()))
						end,
						shrink = 0,
						tabular = true,
						wrap = false,
						text_size = ui.raw(11.5, "background.rs shell row"),
						color = "$text.secondary",
					},
					ui.div { min_w = 20, h = 20, shrink = 0, M.ShellStop(task, stopping, can_stop, on_stop) },
				},
				if task.detail
					then FadeText { task.name, literal = true, pl = ui.raw(20, "background.rs shell row"), text_size = ui.raw(11.5, "background.rs shell row"), color = "$text.secondary" }
					else nil,
				ui.show(function(): boolean
					return shell_line(task.id) ~= nil
				end, FadeText {
					function(): string
						return shell_line(task.id) or ""
					end,
					literal = true,
					pl = ui.raw(20, "background.rs shell row"),
					font = "$font.mono",
					text_size = ui.raw(10.5, "background.rs shell row"),
					color = "$text.secondary",
				}),
			}
		end, {
			key = function(t: any): string
				return t.id
			end,
		}),
	}
	return ui.presence(function(): string
		return detail:get() or ""
	end, function(id: string): Node
		if id == "" then
			return list
		end
		for _, t in tasks:peek() do
			if t.id == id then
				return ShellDetail {
					task = t,
					stopping = stopping,
					can_stop = can_stop,
					clock = clock,
					on_stop = on_stop,
					on_back = function()
						detail:set(nil)
					end,
				}
			end
		end
		return list
	end, { w_full = true })
end)

M.Row = ui.component(function(props: { bind: any, tasks: any, waiting: boolean?, id: string?, keys: (() -> ())? }): Node
	local bind = props.bind
	local keys = props.keys or typing.take
	local runs = M.runs_of(bind)
	local shells = ui.computed(function(): { any }
		local out: { any } = {}
		for _, t in props.tasks:get() do
			if t.task_kind == nil or t.task_kind == "shell" then
				table.insert(out, t)
			end
		end
		return out
	end)
	local clock = P.clock(function(): boolean
		return Roster.working(runs:get().listed) > 0 or #shells:get() > 0
	end)
	local roster_open = ui.use_state(false)
	local shells_open = ui.use_state(false)
	local waiting_open = ui.use_state(false)
	local shell_detail = ui.use_state(nil :: string?)
	ui.effect(function()
		if not shells_open:get() then
			shell_detail:set(nil)
		end
	end)
	tracked(roster_open)
	tracked(shells_open)
	tracked(waiting_open)
	floating[roster_open] = true
	floating[shells_open] = true
	ui.on_cleanup(function()
		floating[roster_open] = nil
		floating[shells_open] = nil
	end)
	keys_back(waiting_open, keys)
	local function toggle(open: any)
		open:set(not open:peek())
		ui.after(0, keys)
	end
	-- Read as a card opens, no on_resize: every pane's row is resized at each frame of a dragged edge.
	local row_ref = ui.ref()
	local width = ui.use_state(640)
	ui.effect(function()
		if shells_open:get() or waiting_open:get() then
			ui.after(0, function()
				local w = row_ref.width
				if w and w > 0 and width:peek() ~= w then
					width:set(w)
				end
			end)
		end
	end)
	local function only(open: any)
		for _, other in { roster_open, shells_open, waiting_open } do
			if other ~= open and other:peek() then
				other:set(false)
			end
		end
	end
	ui.effect(function()
		if roster_open:get() then
			only(roster_open)
		end
	end)
	ui.effect(function()
		if shells_open:get() then
			only(shells_open)
		end
	end)
	ui.effect(function()
		if waiting_open:get() then
			only(waiting_open)
		end
	end)
	local seen = bind.id()
	ui.effect(function()
		local now = bind.id()
		if now ~= seen then
			seen = now
			only({})
		end
	end)
	local function working(): boolean
		return Roster.working(runs:get().listed) > 0
	end
	ui.effect(function()
		if not working() and roster_open:peek() then
			roster_open:set(false)
		end
	end)
	ui.effect(function()
		if #shells:get() == 0 and shells_open:peek() then
			shells_open:set(false)
		end
	end)
	local enter = if motion.reduced_now() then { opacity = { 0, 1 }, duration = 120 } else { opacity = { 0, 1 }, y = { 4, 0 }, duration = 150 }
	local exit = { opacity = 0, duration = 120 }
	local function fan_tiles(): { any }
		return Roster.tiles(runs:get().listed)
	end
	local agents_pill = P.Pill {
		id = "pills-agents",
		class = "pills-pill",
		label = function(): string
			local rows = runs:get().listed
			return Roster.fan_label(Roster.working(rows), Roster.asking(rows))
		end,
		active = function(): boolean
			return roster_open:get()
		end,
		expanded = function(): boolean
			return roster_open:get()
		end,
		shrink = 100,
		min_w = function(): number
			local title = Roster.label(runs:get().listed)
			return if title then 140 else 96
		end,
		pl = ui.raw(7, "the mockup's pill with a fan"),
		gap = ui.raw(8, "the mockup's pill with a fan"),
		on_press = press_in,
		on_click = function()
			toggle(roster_open)
		end,
		AgentFan { tiles = fan_tiles, size = 20 },
		P.Dots { frame = clock.frame },
		ui.show(function(): boolean
			return (Roster.label(runs:get().listed)) ~= nil
		end, FadeText {
			function(): string
				return (Roster.label(runs:get().listed)) or ""
			end,
			literal = true,
			min_w = 0,
			shrink = 1,
		}),
		ui.text {
			function(): string
				local _, tail = Roster.label(runs:get().listed)
				return tail
			end,
			id = "pills-agents-words",
			shrink = 1,
			min_w = 0,
			truncate = true,
		},
		ui.show(function(): boolean
			return Roster.asking(runs:get().listed) > 0
		end, ui.text {
			function(): string
				return "· " .. locale.s("ds.pills.asking", { n = Roster.asking(runs:get().listed) })
			end,
			id = "pills-agents-asking",
			shrink = 1,
			min_w = 0,
			truncate = true,
			color = "$warning",
		}),
		ui.show(function(): boolean
			return Roster.done(runs:get().listed) ~= nil
		end, ui.text {
			function(): string
				return "· " .. (Roster.done(runs:get().listed) or "")
			end,
			id = "pills-agents-done",
			shrink = 1,
			min_w = 0,
			truncate = true,
			color = "$success",
		}),
	}
	local shells_pill = P.Pill {
		id = "pills-shells-pill",
		class = "pills-pill",
		label = function(): string
			return locale.s("ds.pills.shells", { n = #shells:get() })
		end,
		active = function(): boolean
			return shells_open:get()
		end,
		expanded = function(): boolean
			return shells_open:get()
		end,
		on_press = press_in,
		on_click = function()
			toggle(shells_open)
		end,
		P.Dots { frame = clock.frame },
		ui.text { ">_", ml = ui.raw(4, "background.rs pill_leading"), font = "$font.mono", text_size = ui.raw(10.5, "background.rs shells pill"), color = "$text.secondary", wrap = false },
		ui.text { function(): string
			return locale.s("ds.pills.shells", { n = #shells:get() })
		end, id = "pills-shells-count", wrap = false },
	}
	local function pill_slot(shown: () -> string, build: () -> Node, gives: boolean?): Node
		return ui.presence(shown, function(which: string): Node
			if which == "" then
				return ui.div { w = 0, h = 0 }
			end
			return build()
		end, { enter = enter, exit = exit, shrink = if gives then 100 else 0, min_w = if gives then 0 else nil })
	end
	local function card(open: any, class: string, inset: any, place: { [string]: any }, body: () -> Node): Node
		local opts: { [string]: any } = { absolute = true, left = 0, bottom = P.HEIGHT + PLACE.offset, enter = enter, exit = exit }
		for k, v in place do
			opts[k] = v
		end
		return ui.presence(function(): string?
			return if open:get() then "on" else nil
		end, function(): Node
			return ui.column(glass.style({
				class = class,
				w_full = true,
				p = inset,
				pointer_events = "auto",
				on_press = press_in,
				on_key = function(e: any): boolean
					if typed(e) then
						keys()
					end
					return false
				end,
				body(),
			}, "surface"))
		end, opts)
	end
	local children: { any } = {
		id = props.id or "composer-pills",
		class = "composer-pills",
		relative = true,
		w_full = true,
		h = P.HEIGHT,
		gap = ui.raw(6, "background.rs render_corner"),
		items = "end",
		pointer_events = "none",
		on_key = function(e: any): boolean
			if e.key == "Escape" and M.dismiss() then
				ui.after(0, keys)
				return true
			end
			if typed(e) then
				keys()
			end
			return false
		end,
		ref = row_ref,
		pill_slot(function(): string
			return if working() then "agents" else ""
		end, function(): Node
			return agents_pill
		end, true),
		pill_slot(function(): string
			return if #shells:get() > 0 then "shells" else ""
		end, function(): Node
			return shells_pill
		end),
		card(roster_open, "pills-roster-panel", 0, { right = 0 }, function(): Node
			return M.RosterBody {
				bind = bind,
				clock = clock,
				on_close = function()
					roster_open:set(false)
					ui.after(0, keys)
				end,
			}
		end),
		card(shells_open, "pills-shells-panel", ui.raw(4, "the shells menu's inset"), {
			w = function(): number
				return math.min(if shell_detail:get() then DETAIL_W else SHELLS_W, width:get())
			end,
		}, function(): Node
			return ShellsBody { bind = bind, tasks = shells, detail = shell_detail, clock = clock }
		end),
	}
	if props.waiting then
		table.insert(children, Waiting.Pill {
			open = waiting_open,
			width = function(): number
				return width:get()
			end,
			elsewhere_of = bind.id,
		})
	end
	return ui.row(children)
end)

return M
