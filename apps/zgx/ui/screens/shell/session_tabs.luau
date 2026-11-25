--

local ds = require("@ds")
local ContextMenu = require("@ds/context_menu")
local SessionRow = require("@ds/session_row")
local motion = require("@ds/motion")
local TabStrip = require("@ds/tab_strip")
local Tooltip = require("@ds/tooltip")
local status = require("@ds/status")
local look = require("@app/screens/shell/project_look")
local route = require("@app/screens/shell/route")
local sessions = require("@account/sessions")
local splits = require("@app/screens/shell/split")
local frame = require("@app/screens/shell/frame")
local art = require("@app/screens/shell/art")
local top_bar = require("@app/screens/shell/top_bar")
local chat_shared = require("@app/screens/chat/shared")
local AgentFan = require("@ds/agent_fan")
local bots = require("@app/screens/bots/store")
local Pills = require("@app/screens/chat/pills")
require("@app/screens/shell/tab_rules")

local M = {}

local TAB_H, SLOT, TILE = 30, 24, 20
local MARK = 14
local TAB_MAX, TAB_MIN, TAB_GAP = 176, 128, 4
local PIN_W, PIN_RULE, PIN_MARK = 36, 9, 16
local TAB_RADIUS = ui.raw(10, "a tab's corner in the notch")
local TITLE_SIZE = ui.raw(12, "Zeron's tab title, at any text size")
local BAR, LIFT = 42, 30
local DROP = (BAR - TAB_H) / 2
local PIN_MS = 240
local WALL_SWEEP = 280
local NO_WALL = 100000
local THUMB_W, THUMB_H = 150, 92
local LIFT_MORPH_MS = 160
local SLIDE = { duration = 150, ease = "out(3)" }
local FADE, FADE_RATE = 36, 12
-- A tab's stacked tiles, as a fanned hand of cards: at most this many, and for each
-- place how far along it stands and how far it's turned. Alone, a tile stands square.
local STACK = 3
local FAN = { { x = 0, turn = -3 }, { x = 6, turn = 3 }, { x = 10, turn = 7 } }
local DRAFT = "draft"

local rows = splits.tree.steady(function(): { [string]: any }
	local out: { [string]: any } = {}
	for _, c in sessions.now:get().chats do
		out[c.id] = {
			id = c.id,
			title = c.title,
			status = c.status,
			agent = c.agent,
			mark = (c :: any).mark,
			harness = c.harness,
			agent_icon = (c :: any).agent_icon,
			project = c.project,
			branch = c.branch,
			tree = (c :: any).tree,
			pr = (c :: any).pr,
			fan = (c :: any).fan,
			settle = (c :: any).settle,
			last_message = (c :: any).last_message,
		}
	end
	return out
end)

local function chat_of(id: string): any
	return rows:get()[id]
end
local function project_of(c: any): any
	if not c or not c.project then
		return nil
	end
	for _, p in sessions.projects:get() do
		if p.id == c.project then
			return p
		end
	end
	return nil
end

-- The ones focused, latest first.
local recent = ui.persist("shell_session_tabs_recent", {} :: { string })

-- The tabs with a chat still there, by key. (`splits` keeps them.)
local shown = ui.computed(function(): { string }
	local live = rows:get()
	local out: { string } = {}
	for _, key in splits.tabs:get() do
		for _, id in splits.panes(key) do
			if live[id] then
				table.insert(out, key)
				break
			end
		end
	end
	return out
end)
M.shown = shown
M.recent = recent
M.DRAFT = DRAFT

local function tab_of(key: string): string
	return if key == "" then DRAFT else `{DRAFT}:{key}`
end
local function draft_in(tab: string?): string?
	if tab == DRAFT then
		return ""
	end
	return if tab and string.sub(tab, 1, #DRAFT + 1) == `{DRAFT}:` then string.sub(tab, #DRAFT + 2) else nil
end
M.tab_of = tab_of
M.draft_in = draft_in
local draft_put_away = ui.state("shell_session_tabs_drafts_away", {} :: { [string]: boolean })
local drafts = ui.computed(function(): { string }
	local home = route.chat:get().name == "home"
	local active = route.draft:get()
	local away = draft_put_away:get()
	local out: { string } = {}
	for _, key in chat_shared.new_drafts() do
		if not away[key] or (home and key == active) then
			table.insert(out, key)
		end
	end
	if home and not table.find(out, active) then
		table.insert(out, active)
		table.sort(out, chat_shared.new_before)
	end
	return out
end)
M.drafts = drafts
local strip = ui.computed(function(): { string }
	local out = table.clone(shown:get())
	for _, key in drafts:get() do
		table.insert(out, tab_of(key))
	end
	return out
end)
ui.effect(function()
	if route.chat:get().name ~= "home" then
		return
	end
	local key = route.draft:get()
	local away = draft_put_away:peek()
	if away[key] then
		local left = table.clone(away)
		left[key] = nil
		draft_put_away:set(left)
	end
end)

local function selected_key(): string?
	local name = route.current:get().name
	if name == "session" then
		return splits.current_key()
	end
	return if name == "home" then tab_of(route.draft:get()) else nil
end

type Drag = { key: string, w: number, order: { string }, grab: number?, out: boolean }
local drag = ui.state("shell_session_tab_drag", nil :: Drag?)
local rider_x = ui.state("shell_session_tab_rider", 0)
local slot_w = ui.state("shell_session_tab_slot", 0)
-- (The rider isn't hovered: the pointer's its own.)
local rider_hover = ui.state("shell_session_tab_rider_hover", false)
local landed = false
-- (Frozen for a drag, or the strip would shrink as the slot closes.)
local tab_w = ui.state("shell_session_tab_w", TAB_MAX)
local SHADE_PARAMS = { "layerParam0", "layerParam1", "layerParam2", "layerParam3", "layerParam4", "layerParam5", "layerParam6", "layerParam7" }
local PAD = 8
-- (A dragged tab carries its own fill, so the card rises into none.)
local carved = ui.computed(function(): string?
	local key = selected_key()
	local d = drag:get()
	return if key and not (d and d.key == key) then key else nil
end)

local pins = ui.computed(function(): number
	local n = 0
	for _, key in shown:get() do
		if splits.is_pinned(key) then
			n += 1
		end
	end
	return n
end)
M.pins = pins

local function width_of(key: string): number
	return if splits.is_pinned(key) then PIN_W else tab_w:get()
end

-- A tab's panes, as many as show (`STACK`), the one focused last first: the ones
-- focused in that order, then the rest as they stand in the split.
local function stack_of(key: string): { string }
	local peers = splits.panes(key)
	local out: { string } = {}
	for _, r in recent:get() do
		if #out < STACK and table.find(peers, r) and chat_of(r) then
			table.insert(out, r)
		end
	end
	for _, peer in peers do
		if #out < STACK and not table.find(out, peer) and chat_of(peer) then
			table.insert(out, peer)
		end
	end
	return out
end

local function tab_state(key: string): string?
	local best: string? = nil
	for _, id in splits.panes(key) do
		local c = chat_of(id)
		local kind = if c then c.status else nil
		if kind and SessionRow.URGENCY[kind] and (not best or SessionRow.URGENCY[kind] < SessionRow.URGENCY[best]) then
			best = kind
		end
	end
	return best
end

-- What a tab shows: of its chats, the one focused last.
local function face(key: string): string
	return stack_of(key)[1] or splits.panes(key)[1] or key
end
M.face = face

-- A chat a drag carries, for the tabs: not a tab of them being moved.
local function carried(payload: any): string?
	if type(payload) ~= "table" or payload.session_tab then
		return nil
	end
	return splits.dragged(payload)
end

-- A chat focused goes first among the focused. (Its tab: `splits`.)
ui.effect(function()
	local s = route.session:get()
	if not s then
		return
	end
	local was = recent:peek()
	if was[1] ~= s.id then
		local now = { s.id }
		for _, r in was do
			if r ~= s.id and #now < 64 then
				table.insert(now, r)
			end
		end
		recent:set(now)
	end
end)

local function neighbour(key: string): string?
	local list = strip:peek()
	local at = table.find(list, key)
	if not at then
		return nil
	end
	return list[at + 1] or list[at - 1]
end

local function go_to(key: string?)
	local draft = draft_in(key)
	if draft then
		route.open_draft(draft)
	elseif not key then
		route.new_chat()
	else
		route.open_session(face(key))
	end
end

local function close(key: string)
	if splits.is_pinned(key) then
		return
	end
	local was_showing = route.current:peek().name == "session" and splits.current_key() == key
	local next_key = if was_showing then neighbour(key) else nil
	splits.close_tabs({ key })
	if was_showing then
		go_to(next_key)
	end
end

-- A draft written in is put away (its sidebar row stays); an empty one just goes.
local function close_draft(key: string)
	local showing = route.current:peek().name == "home" and route.draft:peek() == key
	local next_tab = if showing then neighbour(tab_of(key)) else nil
	if showing and not next_tab then
		return
	end
	if chat_shared.has_words(key) then
		local away = table.clone(draft_put_away:peek())
		away[key] = true
		draft_put_away:set(away)
	end
	if next_tab then
		go_to(next_tab)
	end
end

local function discard_draft(key: string)
	chat_shared.discard_draft_of(key)
	if draft_put_away:peek()[key] then
		local away = table.clone(draft_put_away:peek())
		away[key] = nil
		draft_put_away:set(away)
	end
end

function M.carried_in_strip(): boolean
	local d = drag:get()
	return d ~= nil and not d.out
end

function M.close_showing(): boolean
	local name = route.current:peek().name
	if name == "home" then
		close_draft(route.draft:peek())
		return true
	end
	local key = splits.current_key()
	if name ~= "session" or not key then
		return false
	end
	close(key)
	return true
end

-- Those tabs closed at once, `keep` gone to if the one showing was among them.
-- (Each of `close` in turn would open every chat on the way.)
local function close_many(list: { string }, keep: string)
	if #list == 0 then
		return
	end
	local was_showing = route.current:peek().name == "session" and table.find(list, splits.current_key() or "") ~= nil
	splits.close_tabs(list)
	if was_showing then
		go_to(keep)
	end
end

-- The tabs other than this one: all of them, or those to one side.
local function others(id: string, side: string?): { string }
	local list = shown:peek()
	local me = table.find(list, id) or 0
	local out: { string } = {}
	for i, other in list do
		if other ~= id and not splits.is_pinned(other) and (side == nil or (side == "left" and i < me) or (side == "right" and i > me)) then
			table.insert(out, other)
		end
	end
	return out
end

local function idle_tabs(): { string }
	local showing = selected_key()
	local out: { string } = {}
	for _, key in shown:peek() do
		local panes = splits.panes(key)
		local c = rows:peek()[panes[1] or ""]
		if key ~= showing and #panes == 1 and c and not c.status and not splits.is_pinned(key) then
			table.insert(out, key)
		end
	end
	return out
end

local tile_enter = { opacity = { 0, 1 }, scale = { 0.6, 1 }, duration = 220, ease = "outBack(1.6)" }
local tile_exit = { opacity = 0, scale = 0.6, duration = 120, ease = "inQuad" }

-- The tile for a pane's place in the stack: its project's. The focus moving
-- swaps each place's pane, the new one springing up there.
local function pane_tile(id: string, place: number): Node
	return ui.presence(function(): string
		local chat = stack_of(id)[place]
		if not chat then
			return ""
		end
		-- (A chat with no project shows the side chat's glyph: a real swap.)
		return if project_of(chat_of(chat)) then chat else `{chat}\0loose`
	end, function(which: string): Node
		local chat = string.match(which, "^[^%z]*") or ""
		if which ~= chat then
			return ds.Tile { size = TILE, inside = ds.Icon { name = "side-chat", size = 14 } }
		end
		local function now(): any
			return project_of(chat_of(chat))
		end
		return ds.Tile {
			size = TILE,
			label = function(): string?
				local p = now()
				return if p then p.name else "Chat"
			end,
			hue = function(): string?
				local p = now()
				return if p then look.of(p).hue else nil
			end,
			src = function(): string?
				local p = now()
				return if p then look.of(p).src else nil
			end,
			emoji = function(): string?
				local p = now()
				return if p then look.of(p).emoji else nil
			end,
			project = function(): string?
				local p = now()
				return if p then p.id else nil
			end,
		}
	end, { w = TILE, h = TILE, shrink = 0, enter = tile_enter, exit = tile_exit })
end

-- A tab's tiles: the pane focused last's in front at the far left, turned a little,
-- its shadow on the others fanned out behind it to the right. (A tab of one pane
-- has its tile alone and square: a turned node is a layer of its own.)
local function tile(id: string): Node
	local function count(): number
		return #stack_of(id)
	end
	local function stacked(): boolean
		return count() >= 2
	end
	local function place(at: number): Node
		local fan = FAN[at]
		return ui.div {
			id = `session-tab-{id}-tile-{at}`,
			absolute = true,
			top = 0,
			left = fan.x,
			w = TILE,
			h = TILE,
			rounded = "$radius.sm",
			rotate = motion.tween(ui.computed(function(): number
				return if stacked() then fan.turn else 0
			end), SLIDE),
			-- (Solid under its wash, so what's behind it doesn't show through; the
			-- one in front's shadow falls on those behind.)
			bg = function(): string
				return if stacked() then "$zeron.menu.solid" else "#00000000"
			end,
			shadow = function(): any
				return if stacked() and at == 1 then "$shadow.sm" else false
			end,
			pane_tile(id, at),
		}
	end
	local shown_places: { any } = {
		relative = true,
		shrink = 0,
		h = TILE,
		w = motion.tween(ui.computed(function(): number
			return TILE + FAN[math.clamp(count(), 1, STACK)].x
		end), SLIDE),
	}
	-- (Those behind first: each paints under the one after it.)
	for at = STACK, 2, -1 do
		table.insert(shown_places, ui.show(function(): boolean
			return count() >= at
		end, place(at)))
	end
	table.insert(shown_places, place(1))
	return ui.div(shown_places)
end

local function project_tile(chat: string): (number) -> Node
	return function(size: number): Node
		local function now(): any
			return project_of(chat_of(chat))
		end
		if not now() then
			return ds.Tile { size = size, inside = ds.Icon { name = "side-chat", size = 14 } }
		end
		return ds.Tile {
			size = size,
			label = function(): string?
				local p = now()
				return if p then p.name else "Chat"
			end,
			hue = function(): string?
				local p = now()
				return if p then look.of(p).hue else nil
			end,
			src = function(): string?
				local p = now()
				return if p then look.of(p).src else nil
			end,
			emoji = function(): string?
				local p = now()
				return if p then look.of(p).emoji else nil
			end,
		}
	end
end

local function agents_of(id: string): { any }
	local panes = splits.panes(id)
	if #panes ~= 1 then
		return {}
	end
	local c = chat_of(panes[1])
	return Pills.fan_of(panes[1], if c then c.fan else nil) or {}
end

local function own_of(id: string): any
	local c = chat_of(face(id))
	return if c and (c.agent or c.mark) then c else nil
end

-- Model maker first, as the roster marks its agents.
local function own_mark(id: string, size: number?): Node
	return SessionRow.mark(function(): any
		local c = own_of(id) or {}
		return { agent = c.mark or c.agent, harness = c.harness, agent_icon = c.agent_icon }
	end, function(): boolean
		return true
	end, size)
end

local function foreign_of(id: string): { any }
	return Pills.foreign(agents_of(id), own_of(id))
end

local function fan_tiles(id: string): { any }
	local own = own_of(id)
	local front = { key = "lead", lead = true, asking = false }
	local out: { any } = { front }
	local others = if own then foreign_of(id) else agents_of(id)
	if own then
		for _, t in agents_of(id) do
			if not table.find(others, t) then
				front.asking = front.asking or t.asking == true
			end
		end
	end
	for _, t in others do
		if #out >= AgentFan.MAX then
			break
		end
		table.insert(out, t)
	end
	return out
end

local function agents_lead(id: string, bare: boolean): Node
	local chat = face(id)
	local tile_of = project_tile(chat)
	return Pills.Fan {
		name = `session-tab-{id}-agents`,
		chat = chat,
		size = if bare then MARK else TILE,
		bare = bare,
		lead = if bare
			then function(size: number): Node
				return own_mark(id, size)
			end
			else tile_of,
		words = function(): { any }
			return agents_of(id)
		end,
		tiles = function(): { any }
			return fan_tiles(id)
		end,
		place = { side = "bottom", align = "start", offset = 6 },
	}
end

local function title(id: string): string
	local c = chat_of(face(id))
	local bot = if c then bots.of_chat(c.id) else nil
	return if bot then bot.name elseif c then c.title else ""
end

local function lead(id: string, hovered: any): Node
	local function chat(): any
		return chat_of(face(id))
	end
	return ui.presence(function(): string
		if #stack_of(id) >= 2 then
			return "stack"
		end
		if #agents_of(id) > 0 then
			if not own_of(id) then
				return "agents"
			elseif #foreign_of(id) > 0 then
				return "fan"
			end
			return if hovered:get() then "grip" else "own"
		end
		local c = chat()
		if c and (c.status == "working" or c.status == "connecting") then
			return c.status
		end
		return if hovered:get() then "grip" else "mark"
	end, function(which: string): Node
		if which == "stack" then
			return tile(id)
		elseif which == "agents" or which == "fan" then
			return agents_lead(id, which == "fan")
		elseif which == "own" then
			return own_mark(id)
		elseif which == "grip" then
			return ui.div { color = "$text.secondary", ds.Icon { name = "drag-handle", size = 14 } }
		elseif which == "mark" then
			return SessionRow.mark(function(): any
				return chat() or {}
			end, function(): boolean
				return true
			end)
		end
		return ui.div { id = `session-tab-{id}-state`, SessionRow.state(which :: any) }
	end, {
		shrink = 0,
		-- (Stacked, as wide as its fan: the title starts after it, not under it.)
		w = function(): number
			local n = #stack_of(id)
			if n >= 2 then
				return TILE + FAN[math.clamp(n, 1, STACK)].x
			end
			if #agents_of(id) == 0 then
				return 16
			end
			local own = own_of(id)
			if own and #foreign_of(id) == 0 then
				return 16
			end
			return AgentFan.width(#fan_tiles(id), if own then MARK else TILE, own ~= nil)
		end,
		h = 16,
		items = "center",
		justify = "center",
	})
end

local function state_mark(kind: string, size: number, compact: boolean?): Node
	if kind ~= "waiting" then
		return status.StatusDot { kind = kind :: any, size = size }
	end
	local track = if compact then size + 2 else 9
	local run = ui.div {
		relative = true,
		w = ui.raw(track, "a run's track"),
		h = ui.raw(2, "a run's track"),
		shrink = 0,
		rounded = "$radius.full",
		overflow = "hidden",
		ui.div { absolute = true, left = 0, right = 0, top = 0, bottom = 0, bg = "$zeron.status.working", opacity = 0.28 },
		status.looping({
			absolute = true,
			left = 0,
			top = 0,
			bottom = 0,
			w = ui.raw(4, "what runs along the track"),
			rounded = "$radius.full",
			bg = "$zeron.status.working",
		}, function(el: any): any
			return anime.animate(el, { x = { -4, track }, duration = 1100, ease = "inOut(2)", loop = true })
		end, 1100),
	}
	if compact then
		return run
	end
	return ui.row {
		shrink = 0,
		items = "end",
		gap = ui.raw(1, "the prompt and its run"),
		pb = ui.raw(1, "the run on the prompt's line"),
		color = "$zeron.status.working",
		ds.Icon { name = "chevron-right", size = 10 },
		run,
	}
end
M.state_mark = state_mark

local function pen(shows: () -> boolean): Node
	return ui.show(shows, ui.div { shrink = 0, color = "$warning", ds.Icon { name = "pen", size = 13 } })
end

local function close_button(on_click: () -> ()): Node
	return ds.IconButton {
		label = ui.t("shell.tabs.close"),
		tooltip = false,
		size = "sm",
		tone = "quiet",
		w = SLOT,
		h = SLOT,
		rounded = "$radius.sm",
		on_click = on_click,
		ds.Icon { name = "close", size = 14 },
	}
end

local function settle_button(id: string, hovered: any): Node
	local function chat(): any
		local panes = splits.panes(id)
		return if #panes == 1 then chat_of(panes[1]) else nil
	end
	return ui.show(function(): boolean
		return hovered:get() and chat() ~= nil
	end, ui.presence(function(): string
		local c = chat()
		return if c and require("@app/screens/shell/sidebar").is_settled(c) then "unsettle" else "settle"
	end, function(which: string): Node
		local settled = which == "unsettle"
		return ds.IconButton {
			id = `session-tab-{id}-settle`,
			label = ui.t(if settled then "ds.sidebar.unsettle_thread" else "ds.sidebar.settle_thread"),
			tooltip = false,
			size = "sm",
			tone = "quiet",
			w = SLOT,
			h = SLOT,
			rounded = "$radius.sm",
			on_click = function()
				local c = chat()
				if c then
					require("@app/screens/shell/sidebar").settle(c.id, not settled)
				end
			end,
			ds.Icon { name = if settled then "lucide-undo-2" else "lucide-check", size = 14 },
		}
	end, { shrink = 0 }))
end

local function pin_button(id: string, hovered: any): Node
	return ui.show(function(): boolean
		return hovered:get() and #splits.panes(id) == 1
	end, ds.IconButton {
		id = `session-tab-{id}-pin`,
		label = ui.t("shell.tabs.pin"),
		tooltip = false,
		size = "sm",
		tone = "quiet",
		w = SLOT,
		h = SLOT,
		rounded = "$radius.sm",
		on_click = function()
			splits.pin(id, true)
		end,
		ds.Icon { name = "pin", size = 14 },
	})
end

local function corner(id: string, hovered: any, active: () -> boolean): Node
	return ui.row {
		w = SLOT,
		h = SLOT,
		shrink = 0,
		items = "center",
		justify = "center",
		ui.presence(function(): string
			if hovered:get() then
				return "close"
			end
			local state = tab_state(id)
			if #splits.panes(id) >= 2 then
				return state or (if active() then "close" else "")
			end
			if active() then
				return "close"
			end
			return if state and state ~= "working" and state ~= "connecting" then `dot:{state}` else ""
		end, function(which: string): Node
			if which == "" then
				return ui.div {}
			elseif which == "close" then
				return close_button(function()
					close(id)
				end)
			end
			local dot = string.match(which, "^dot:(.+)$")
			if dot then
				return ui.div { id = `session-tab-{id}-state`, state_mark(dot, 6) }
			end
			return ui.div { id = `session-tab-{id}-state`, SessionRow.state(which :: any) }
		end, { shrink = 0 }),
	}
end

local function tree_badge(id: string): Node
	local function tree(): any
		local panes = splits.panes(id)
		local c = if #panes == 1 then chat_of(panes[1]) else nil
		return if c and not c.pr then c.tree else nil
	end
	return ui.presence(function(): string
		local t = tree()
		return if t then `{t.kind}:{t.commits}:{t.dirty}` else ""
	end, function(key: string): Node
		if key == "" then
			return ui.div {}
		end
		local kind, commits, dirty = string.match(key, "^(%a+):(%d+):(%d+)$")
		local ahead = kind == "ahead"
		local c = chat_of(face(id))
		local base = (c and c.branch) or ui.t("shell.tabs.tree.base"):get()
		local tip = if ahead
			then ui.t("shell.tabs.tree.ahead", { n = tonumber(commits), base = base }):get()
			else ui.t("shell.tabs.tree.merged", { dirty = tonumber(dirty), base = base }):get()
		local hue = if ahead then "$text.secondary" else "$zeron.pr.merged"
		return ui.row {
			id = `session-tab-{id}-tree`,
			shrink = 0,
			relative = true,
			h = 16,
			ml = ui.raw(8, "Zeron's tab's gaps"),
			px = "$space.1",
			gap = ui.raw(3, "Zeron's badge's gap"),
			items = "center",
			rounded = "$radius.sm",
			color = hue,
			tooltip = tip,
			label = tip,
			ui.div { absolute = true, left = 0, right = 0, top = 0, bottom = 0, rounded = "$radius.sm", bg = hue, opacity = 0.08 },
			ds.Icon { name = if ahead then "arrow-up" else "check", size = 10 },
			if ahead then ui.text { commits, literal = true, tabular = true, font = "$font.mono", text_size = ui.raw(10, "Zeron's badge") } else nil,
			if not ahead and tonumber(dirty) ~= 0 then ui.div { w = 5, h = 5, rounded = "$radius.full", shrink = 0, bg = "$warning" } else nil,
		}
	end, { shrink = 0 })
end

local function face_row(id: string, hovered: any, active: () -> boolean): { any }
	return {
		lead(id, hovered),
		pen(function(): boolean
			return #splits.panes(id) == 1 and chat_shared.draft_of(face(id)).text ~= ""
		end),
		-- (Together: an empty badge would take a gap from the title.)
		ui.row {
			grow = 1,
			min_w = 0,
			items = "center",
			ds.FadeText {
				function(): string
					return title(id)
				end,
				grow = 1,
				min_w = 0,
				text_size = TITLE_SIZE,
			},
			tree_badge(id),
		},
		settle_button(id, hovered),
		pin_button(id, hovered),
		corner(id, hovered, active),
	}
end

local hover_key = ui.state("shell_session_tab_hover", nil :: string?)
local function hover(key: string, inside: boolean)
	if inside then
		hover_key:set(key)
	elseif hover_key:peek() == key then
		hover_key:set(nil)
	end
end

local function tab_box(props: { [any]: any }, active: () -> boolean, washed: () -> boolean, tile: (() -> number)?): { [any]: any }
	props.h = TAB_H
	props.shrink = 0
	props.relative = true
	props.pl = props.pl or ui.raw(8, "a tab's start")
	props.pr = props.pr or ui.raw(4, "a tab's end")
	props.gap = ui.raw(8, "Zeron's tab's gaps")
	props.items = "center"
	props.cursor = "pointer"
	props.rounded = TAB_RADIUS
	props.color = function(): string
		return if active() then "$text.primary" else frame.strip_token("ink")
	end
	table.insert(props, 1, ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		rounded = TAB_RADIUS,
		border = 1,
		bg = function(): string
			return frame.row_token(if active() then "selected" else "hover")
		end,
		border_color = function(): string
			return if active() then frame.row_token("rim") else "#00000000"
		end,
		opacity = ui.tween(ui.computed(function(): number
			return if (washed() and not active()) or (active() and carved:get() == nil) then 1 else 0
		end), { duration = math.max(motion.ms(120), 1), ease = "out(3)" }),
	})
	if tile then
		table.insert(props, 1, ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = TAB_RADIUS,
			bg = function(): string
				return frame.row_token("tile")
			end,
			opacity = function(): number
				return if active() and carved:get() ~= nil then 0 else tile()
			end,
		})
	end
	return props
end

local function unpin_button(id: string): Node
	return ds.IconButton {
		id = `session-tab-{id}-unpin`,
		label = ui.t("shell.tabs.unpin"),
		tooltip = false,
		size = "sm",
		tone = "quiet",
		w = 16,
		h = 16,
		rounded = "$radius.full",
		bg = "$zeron.menu.solid",
		on_click = function()
			splits.pin(id, false)
		end,
		ui.div {
			relative = true,
			w = 10,
			h = 10,
			ds.Icon { name = "pin", size = 10 },
			ui.div {
				absolute = true,
				left = -1,
				right = -1,
				top = ui.raw(4.25, "across the pin"),
				h = ui.raw(1.5, "the strike's stroke"),
				rounded = "$radius.full",
				bg = "$text.primary",
				rotate = 45,
			},
		},
	}
end

local function tile_face(id: string, hovered: any): Node
	return ui.row {
		id = `session-tab-{id}-tile`,
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		items = "center",
		justify = "center",
		own_mark(id, PIN_MARK),
		ui.presence(function(): string
			return if hovered:get() then "unpin" else tab_state(id) or ""
		end, function(which: string): Node
			if which == "" then
				return ui.div {}
			elseif which == "unpin" then
				return ui.div { absolute = true, right = ui.raw(-4, "past the tile's corner"), top = ui.raw(-4, "past the tile's corner"), unpin_button(id) }
			end
			return ui.div { id = `session-tab-{id}-state`, absolute = true, left = 0, right = 0, bottom = ui.raw(2, "under the mark"), items = "center", state_mark(which, 5, true) }
		end, { absolute = true, left = 0, right = 0, top = 0, bottom = 0 }),
	}
end

local function menu_items(id: string): { any }
	local pinned = splits.is_pinned(id)
	local items: { any } = {}
	if pinned or splits.can_pin(id) then
		table.insert(items, { label = ui.t(if pinned then "shell.tabs.unpin" else "shell.tabs.pin"):get(), icon = "pin", on_select = function()
			splits.pin(id, not pinned)
		end })
		table.insert(items, { separator = true })
	end
	if not pinned then
		table.insert(items, { label = ui.t("shell.tabs.close"):get(), icon = "close-thin", on_select = function()
			close(id)
		end })
	end
	if #others(id) > 0 then
		table.insert(items, { label = ui.t("shell.tabs.close_others"):get(), on_select = function()
			close_many(others(id), id)
		end })
	end
	if #others(id, "right") > 0 then
		table.insert(items, { label = ui.t("shell.tabs.close_right"):get(), on_select = function()
			close_many(others(id, "right"), id)
		end })
	end
	if #others(id, "left") > 0 then
		table.insert(items, { label = ui.t("shell.tabs.close_left"):get(), on_select = function()
			close_many(others(id, "left"), id)
		end })
	end
	local idle = idle_tabs()
	if #idle > 0 then
		table.insert(items, { label = ui.t("shell.tabs.close_idle"):get(), on_select = function()
			close_many(idle, id)
		end })
	end
	return items
end
M.menu_items = menu_items

type Lift = { x: number, y: number, gx: number, gy: number, top: number, w: number }
local function lifted_card(id: string, at: State<Lift?>, t: State<number>): Node
	local function lerp(a: number, b: number): number
		return a + (b - a) * t:get()
	end
	local function line(part: number): Node
		return ui.div { h = 4, w = `{part * 100}%`, rounded = ui.raw(2, "Zeron's skeleton line"), bg = "$hover" }
	end
	return ui.column {
		absolute = true,
		left = function(): number
			local a = at:get()
			return ui.raw(if a then lerp(0, a.gx - THUMB_W / 2) else 0, "Zeron's lifted tab")
		end,
		top = function(): number
			local a = at:get()
			return ui.raw(if a then lerp(a.top - (a.y - a.gy), a.gy - 12) else 0, "Zeron's lifted tab")
		end,
		w = function(): number
			local a = at:get()
			return ui.raw(lerp(if a then a.w else THUMB_W, THUMB_W), "Zeron's lifted tab")
		end,
		h = function(): number
			return ui.raw(lerp(TAB_H, THUMB_H), "Zeron's lifted tab")
		end,
		opacity = t,
		overflow = "hidden",
		rounded = function(): number
			return ui.raw(lerp(8, 10), "Zeron's lifted tab")
		end,
		bg = "$surface.raised",
		border = 1,
		border_color = "$border.strong",
		shadow = "$shadow.lg",
		px = ui.raw(10, "Zeron's card's sides"),
		ui.row {
			h = function(): number
				return ui.raw(lerp(TAB_H - 2, 24), "Zeron's lifted tab")
			end,
			shrink = 0,
			gap = "$space.2",
			items = "center",
			color = "$text.primary",
			own_mark(id, 14),
			ds.FadeText {
				title(id),
				grow = 1,
				min_w = 0,
				text_size = function(): number
					return ui.raw(lerp(12, 10), "Zeron's lifted tab title")
				end,
			},
		},
		ui.column { gap = ui.raw(6, "Zeron's lines' gap"), opacity = t, line(0.9), line(0.7), line(0.82), line(0.5) },
	}
end

local function leave_strip()
	local d = drag:peek()
	if not d or d.out then
		return
	end
	local again = table.clone(d)
	again.out = true
	drag:set(again)
	anime.animate(slot_w, { value = 0, duration = motion.DRAG_SLIDE.duration, ease = motion.DRAG_SLIDE.ease })
end

local function zone_start(pinned: number): number
	return pinned * (PIN_W + TAB_GAP) + (if pinned > 0 then PIN_RULE else 0)
end
local function slot_left(at: number): number
	local k = pins:get()
	if at <= k then
		return PAD + (at - 1) * (PIN_W + TAB_GAP)
	end
	return PAD + zone_start(k) + (at - 1 - k) * (tab_w:get() + TAB_GAP)
end

local Tab = ui.component(function(props: { chat: string, motion: TabStrip.Motion }): Node
	local id = props.chat
	local hovered = ui.use_state(false)
	-- A chat held over it, that dropping would bring in.
	local held = ui.use_state(false)
	local own = ui.ref()
	local function active(): boolean
		return selected_key() == id
	end
	local function riding(): boolean
		local d = drag:get()
		return d ~= nil and d.key == id
	end
	local lift_at = ui.use_state(nil :: Lift?)
	local lift_t = ui.use_state(0)
	local lifting: any, lifted = nil, false
	ui.effect(function()
		local d = drag:get()
		local out = d ~= nil and d.key == id and d.out
		if out == lifted then
			return
		end
		lifted = out
		if lifting then
			lifting:cancel()
		end
		lifting = anime.animate(lift_t, motion.params({ value = if out then 1 else 0, duration = LIFT_MORPH_MS, ease = motion.EASE_OUT }))
	end)
	local function pinned(): boolean
		return splits.is_pinned(id)
	end
	local pin_t = ui.use_state(if table.find(splits.pinned:peek(), id) then 1 else 0)
	local folding: any, folded = nil, pin_t:peek() == 1
	ui.effect(function()
		local on = pinned()
		if on == folded then
			return
		end
		folded = on
		if folding then
			folding:cancel()
		end
		folding = anime.animate(pin_t, motion.params({ value = if on then 1 else 0, duration = PIN_MS, ease = motion.EASE_OUT }))
	end)
	local function fold(from: number, to: number): number
		return from + (to - from) * pin_t:get()
	end
	local row: { [any]: any } = tab_box({
		id = `session-tab-{id}`,
		ref = own,
		role = "tab",
		label = function(): string
			return title(id)
		end,
		selected = active,
		w = function(): number
			return if riding() then slot_w:get() else fold(tab_w:get(), PIN_W)
		end,
		opacity = function(): number
			return if riding() then 0 else 1
		end,
		overflow = function(): string
			local t = pin_t:get()
			return if riding() or (t > 0 and t < 1) then "hidden" else "visible"
		end,
		on_hover = function(inside: boolean)
			hovered:set(inside)
			hover(id, inside)
		end,
		on_click = function()
			route.open_session(face(id))
		end,
		on_aux_click = function(e: any)
			if e.button == "middle" then
				close(id)
			end
		end,
		on_drag = function(): any
			local w = if table.find(splits.pinned:peek(), id) then PIN_W else tab_w:peek()
			local list = shown:peek()
			local el = own:peek()
			local r = if el then ui.window_rect(el) else nil
			lift_at:set({ x = 0, y = 0, gx = 0, gy = 0, top = if r then r.y else 0, w = w })
			rider_x:set(slot_left(table.find(list, id) or 1))
			slot_w:set(w)
			landed = false
			drag:set({ key = id, w = w, order = list, grab = nil, out = false })
			-- (Zeron's: a tab carried into the page splits it, as a sidebar row does.)
			splits.dragging:set(true)
			splits.carried:set(face(id))
			return { session_tab = id, session = face(id) }
		end,
		drag_preview = function(): Node
			local a = lift_at:peek()
			return ui.div {
				w = if a then a.w else THUMB_W,
				h = TAB_H,
				lifted_card(id, lift_at, lift_t),
			}
		end,
		drag_preview_in_window = true,
		on_drag_move = function(_: any, e: any)
			local a = lift_at:peek()
			if a then
				lift_at:set({ x = e.x, y = e.y, gx = e.grab_x, gy = e.grab_y, top = a.top, w = a.w })
			end
			-- (Past the strip before its band shows to hear it go.)
			if e.y > ds.TitleBar.middle() + BAR / 2 + LIFT then
				leave_strip()
			end
		end,
		on_drag_end = function()
			local d = drag:peek()
			drag:set(nil)
			splits.dragging:set(false)
			splits.carried:set("")
			if d and not landed then
				splits.set_order(d.order)
			end
		end,
		on_drag_over = function(payload: any)
			if type(payload) == "table" and payload.session_tab then
				return
			end
			local chat = carried(payload)
			if chat then
				local can = splits.can_join(chat, id)
				if held:peek() ~= can then
					held:set(can)
				end
			end
		end,
		on_drag_leave = function()
			held:set(false)
		end,
		on_drop = function(payload: any)
			held:set(false)
			local chat = carried(payload)
			if chat then
				splits.join_tab(chat, id)
			end
		end,
		on_drag_spring = function(payload: any): boolean
			local chat = carried(payload)
			if not chat or payload.split_pane or not held:peek() or active() then
				return false
			end
			route.open_session(face(id))
			return true
		end,
	}, active, function(): boolean
		return hovered:get() or held:get()
	end, function(): number
		return pin_t:get()
	end)
	-- (Its slot closes to nothing: no padding left holding it open.)
	row.pl = function(): any
		return ui.raw(if riding() then 0 else fold(8, 0), "a tab's start")
	end
	row.pr = function(): any
		return ui.raw(if riding() then 0 else fold(4, 0), "a tab's end")
	end
	local parts: { [any]: any } = {
		grow = 1,
		min_w = 0,
		h_full = true,
		gap = ui.raw(8, "Zeron's tab's gaps"),
		items = "center",
		opacity = function(): number
			return 1 - math.min(pin_t:get() * 2, 1)
		end,
	}
	for _, part in face_row(id, hovered, active) do
		table.insert(parts, part)
	end
	table.insert(row, ui.show(function(): boolean
		return pin_t:get() < 1
	end, ui.row(parts)))
	table.insert(row, ui.show(function(): boolean
		return pin_t:get() > 0
	end, ui.div {
		absolute = true,
		tooltip = Tooltip { function(): string
			return title(id)
		end },
		tooltip_placement = { side = "bottom", align = "center", offset = 6 },
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		opacity = function(): number
			return math.clamp(pin_t:get() * 2 - 1, 0, 1)
		end,
		tile_face(id, hovered),
	}))
	return ContextMenu {
		label = ui.t("panel.tab.actions"),
		ring = "$radius.lg",
		shrink = 0,
		ml = function(): any
			local k = pins:get()
			return ui.raw(if k > 0 and shown:get()[k + 1] == id then PIN_RULE else 0, "the rule's room after the pinned")
		end,
		items = function(): { any }
			return menu_items(id)
		end,
		ui.row(props.motion.apply(row, id, function(): number
			return width_of(id)
		end)),
	}
end)

-- Those tabs closed at once, chats' and drafts' (one written in put away), `keep` gone to if the one showing was among them.
local function close_tabs(tabs: { string }, keep: string)
	local showing = selected_key()
	local chats: { string } = {}
	local away = table.clone(draft_put_away:peek())
	for _, tab in tabs do
		local d = draft_in(tab)
		if not d then
			table.insert(chats, tab)
		elseif chat_shared.has_words(d) then
			away[d] = true
		end
	end
	draft_put_away:set(away)
	if #chats > 0 then
		splits.close_tabs(chats)
	end
	if showing and table.find(tabs, showing) then
		go_to(keep)
	end
end

local function draft_menu(key: string): { any }
	local tab = tab_of(key)
	local list = strip:peek()
	local at = table.find(list, tab) or 0
	local function others_of(side: string?): { string }
		local out: { string } = {}
		for i, other in list do
			if other ~= tab and not splits.is_pinned(other) and (side == nil or (side == "left" and i < at) or (side == "right" and i > at)) then
				table.insert(out, other)
			end
		end
		return out
	end
	local items: { any } = {
		{ label = ui.t("shell.tabs.close"):get(), icon = "close-thin", on_select = function()
			close_draft(key)
		end },
	}
	if #others_of(nil) > 0 then
		table.insert(items, { label = ui.t("shell.tabs.close_others"):get(), on_select = function()
			close_tabs(others_of(nil), tab)
		end })
	end
	if #others_of("right") > 0 then
		table.insert(items, { label = ui.t("shell.tabs.close_right"):get(), on_select = function()
			close_tabs(others_of("right"), tab)
		end })
	end
	if #others_of("left") > 0 then
		table.insert(items, { label = ui.t("shell.tabs.close_left"):get(), on_select = function()
			close_tabs(others_of("left"), tab)
		end })
	end
	if chat_shared.has_words(key) then
		table.insert(items, { label = ui.t("ds.sidebar.discard_draft"):get(), icon = "trash-bin-minimalistic", on_select = function()
			discard_draft(key)
		end })
	end
	return items
end

local DraftTab = ui.component(function(props: { key: string }): Node
	local key = props.key
	local tab = tab_of(key)
	local hovered = ui.use_state(false)
	local function active(): boolean
		return selected_key() == tab
	end
	local function close()
		close_draft(key)
	end
	return ContextMenu {
		label = ui.t("panel.tab.actions"),
		ring = "$radius.lg",
		shrink = 0,
		items = function(): { any }
			return draft_menu(key)
		end,
		ui.row(tab_box({
			id = if key == "" then "session-tab-draft" else `session-tab-draft-{key}`,
			role = "tab",
			label = ui.t("shell.tabs.new_session"),
			selected = active,
			w = function(): number
				return tab_w:get()
			end,
			on_hover = function(inside: boolean)
				hovered:set(inside)
				hover(tab, inside)
			end,
			on_click = function()
				route.open_draft(key)
			end,
			on_aux_click = function(e: any)
				if e.button == "middle" then
					close()
				end
			end,
			ui.div { shrink = 0, color = "$warning", ds.Icon { name = "pen", size = 13 } },
			ds.FadeText { ui.t("shell.tabs.new_session"), grow = 1, min_w = 0, text_size = TITLE_SIZE },
			ui.row {
				w = SLOT,
				h = SLOT,
				shrink = 0,
				items = "center",
				justify = "center",
				ui.show(function(): boolean
					return hovered:get() or active()
				end, close_button(close)),
			},
		}, active, function(): boolean
			return hovered:get()
		end)),
	}
end)

function M.Strip(props: { window_x: () -> number, start: () -> number, right: () -> number, card_right: () -> number, hidden: (() -> boolean)? }): Node
	-- Each edge fades in full as soon as tabs run a little past it: a band only
	-- as wide as the overflow (`fade_overflow`) is a hard edge while that's a
	-- few points. Over the last few, as the end comes to rest, it goes.
	local offset = ui.state("shell_session_tabs_offset", 0)
	local view, content = ui.ref(), ui.ref()
	local function bands(): { number }
		local at = offset:get()
		local past = (content.width or 0) - (view.width or 0) - at
		return { math.clamp(at * FADE_RATE, 0, FADE), math.clamp(past * FADE_RATE, 0, FADE) }
	end
	local function slots(): number
		return #strip:get()
	end
	local function slot_w_at(at: number): number
		return if at <= pins:get() then PIN_W else tab_w:get()
	end
	ui.effect(function()
		local room = view.width or 0
		local k = pins:get()
		local count = math.max(slots() - k, 1)
		if drag:get() or room <= 0 then
			return
		end
		local w = math.clamp((room - PAD * 2 - zone_start(k) - TAB_GAP * (count - 1)) / count, TAB_MIN, TAB_MAX)
		if tab_w:peek() ~= w then
			tab_w:set(w)
		end
	end)
	local function showing_at(): number?
		local key = selected_key()
		return if key then table.find(strip:get(), key) else nil
	end
	-- The tab showing, always clear of the overflow and its fade: as it's opened,
	-- as tabs come and go, and as the strip's room changes (the panel opening).
	-- (A person's own scrolling is left alone: this doesn't follow the offset.)
	ui.effect(function()
		local at_slot = showing_at()
		local room = view.width or 0
		local n = slots()
		if not at_slot or room <= 0 or drag:peek() then
			return
		end
		local w = slot_w_at(at_slot)
		local total = slot_left(n) + slot_w_at(n) + PAD
		local left = slot_left(at_slot)
		local right = left + w
		local at = offset:peek()
		local want = at
		if w + 2 * FADE >= room then
			want = left - (room - w) / 2
		elseif left - FADE < at then
			want = left - FADE
		elseif right + FADE > at + room then
			want = right + FADE - room
		end
		-- (No further than what's laid out, which the scroll would clamp it to: its width is followed, so it goes on as the tabs widen.)
		want = math.clamp(want, 0, math.max(0, math.min(total, content.width or 0) - room))
		if math.abs(want - at) > 0.5 then
			offset:set(want)
		end
	end)
	local start = props.start
	local carve_l, carve_r = ui.state("shell_session_tab_carve_l", 0), ui.state("shell_session_tab_carve_r", 0)
	local carve_placed = false
	ui.effect(function()
		local key = carved:get()
		local at = if key then table.find(strip:get(), key) else nil
		local l, r
		if at then
			l = slot_left(at)
			r = l + slot_w_at(at)
		else
			l = (carve_l:peek() + carve_r:peek()) / 2
			r = l
		end
		if not carve_placed then
			carve_placed = at ~= nil
			carve_l:set(l)
			carve_r:set(r)
			return
		end
		local ms = math.max(motion.ms(PIN_MS), 1)
		anime.animate(carve_l, { value = l, duration = ms, ease = motion.DRAG_SLIDE.ease })
		anime.animate(carve_r, { value = r, duration = ms, ease = motion.DRAG_SLIDE.ease })
	end)
	local cut_params: { [any]: { [string]: number } } = {}
	ui.effect(function()
		local room = view.width or 0
		local hidden = props.hidden ~= nil and props.hidden()
		local a = start() - (1 - top_bar.reveal()) * WALL_SWEEP
		local b = start() + room
		local left = not hidden and room > 0 and a - frame.NOTCH_FLARE > art.card_left:get()
		local right = not hidden and room > 0 and b + frame.NOTCH_FLARE < props.card_right()
		local x0, x1 = if left then a else -NO_WALL, if right then b else NO_WALL
		local was = art.notch:peek()
		if was.x0 ~= x0 or was.x1 ~= x1 or was.left ~= left or was.right ~= right then
			art.notch:set({ x0 = x0, x1 = x1, left = left, right = right })
		end
		local cl = start() + carve_l:get() - offset:get()
		local cr = if hidden or room <= 0 then cl else start() + carve_r:get() - offset:get()
		local want: { [string]: number } = { layerParam0 = x0, layerParam1 = x1, layerParam2 = frame.band_top(), layerParam3 = BAR, layerParam4 = cl, layerParam5 = cr, layerParam6 = DROP, layerParam7 = 10 }
		local seen: { [any]: { [string]: number } } = {}
		for _, ref in { art.strip, art.tab, art.edge } do
			local el = ref:get()
			if el then
				local had = cut_params[el] or {}
				seen[el] = had
				local changed: { [string]: number } = {}
				for _, key in SHADE_PARAMS do
					local v = want[key]
					if had[key] ~= v then
						had[key] = v
						changed[key] = v
					end
				end
				if next(changed) then
					anime.utils.set(el, changed)
				end
			end
		end
		cut_params = seen
	end)
	local tab_motion = TabStrip.motion(function(): { string }
		return shown:get()
	end, function(id: string): string
		return id
	end, function(id: string): boolean
		return id == route.from_new
	end)
	local function over_band(_: any, e: any)
		local d = drag:peek()
		if not d then
			return
		end
		local list = shown:peek()
		local k = pins:peek()
		local pinned = table.find(splits.pinned:peek(), d.key) ~= nil
		local first = if pinned then 1 else k + 1
		local count = if pinned then k else #list - k
		local origin = if pinned then 0 else zone_start(k)
		local x = e.x - start() + offset:peek() - PAD - origin
		local slot = d.w + TAB_GAP
		if d.grab == nil or d.out then
			local at = (table.find(list, d.key) or first) - first + 1
			local again = table.clone(d)
			again.grab = if d.grab == nil then x - (at - 1) * slot else d.grab
			again.out = false
			drag:set(again)
			d = again
			anime.animate(slot_w, { value = d.w, duration = motion.DRAG_SLIDE.duration, ease = motion.DRAG_SLIDE.ease })
		end
		rider_x:set(PAD + origin + math.clamp(x - (d.grab :: number), -6, (count - 1) * slot + 6))
		local to = first + math.clamp(math.floor(x / slot) + 1, 1, count) - 1
		if list[to] ~= d.key then
			splits.move_tab(d.key, list[to])
		end
	end
	local function ruled(): boolean
		local k = pins:get()
		return k > 0 and slots() > k
	end
	-- From the window's left whatever the sidebar is doing, so they stay put as
	-- it moves. (Only the tabs' own part takes the pointer.)
	return ui.row {
		absolute = true,
		left = function(): number
			return -props.window_x()
		end,
		right = 0,
		top = function(): number
			return ui.raw(ds.TitleBar.middle() - BAR / 2, "on the top bar's line")
		end,
		h = BAR + 1,
		pointer_events = "none",
		hidden = props.hidden,
		theme = function(): any
			return require("@app/screens/shell/frame").bar_theme()
		end,
		ui.row {
			id = "session-tabs",
			role = "tablist",
			label = "Open chats",
			relative = true,
			grow = 1,
			min_w = 0,
			h_full = true,
			ml = function(): any
				return ui.raw(start(), "past what stands before the tabs")
			end,
			window_drag = true,
			pointer_events = "auto",
			pr = function(): any
				return ui.raw(props.right(), "clear of the title bar's controls at the right")
			end,
			ui.scroll_x {
				id = "session-tabs-scroll",
				ref = view,
				offset = offset,
				grow = 1,
				min_w = 0,
				h_full = true,
				fade_x = bands,
				ui.row {
					ref = content,
					relative = true,
					h_full = true,
					px = ui.raw(PAD, "the notch's room round its tabs"),
					pt = ui.raw(DROP, "a tab stands on the bar's line"),
					ui.div {
						id = "session-tabs-rule",
						absolute = true,
						left = function(): number
							return ui.raw(PAD + pins:get() * (PIN_W + TAB_GAP) + (PIN_RULE - TAB_GAP) / 2 - 0.5, "between the pinned and the rest")
						end,
						top = ui.raw(DROP + (TAB_H - 18) / 2, "the rule after the pinned"),
						w = 1,
						h = 18,
						bg = function(): string
							return frame.row_token("rule")
						end,
						opacity = ui.tween(ui.computed(function(): number
							return if ruled() and drag:get() == nil then 1 else 0
						end), { duration = math.max(motion.ms(120), 1), ease = "out(3)" }),
					},
					ui.row {
						-- (An empty `ui.each` still takes 16 points.)
						w = function(): any
							return if #shown:get() == 0 then 0 else nil
						end,
						shrink = 0,
						ui.each(tab_motion.list, function(id: string): Node
							return Tab { chat = id, motion = tab_motion }
						end, {
							key = function(id: string): string
								return id
							end,
							direction = "row",
							gap = ui.raw(TAB_GAP, "Zeron's gap between tabs"),
							animate = motion.layout(motion.DRAG_SLIDE),
						}),
					},
					ui.row {
						w = function(): any
							return if #drafts:get() == 0 then 0 else nil
						end,
						ml = function(): any
							local both = #shown:get() > 0 and #drafts:get() > 0
							local after_pins = both and pins:get() == #shown:get()
							return ui.raw((if both then TAB_GAP else 0) + (if after_pins then PIN_RULE else 0), "Zeron's gap between tabs")
						end,
						shrink = 0,
						ui.each(drafts, function(key: string): Node
							return DraftTab { key = key }
						end, {
							key = function(key: string): string
								return key
							end,
							direction = "row",
							gap = ui.raw(TAB_GAP, "Zeron's gap between tabs"),
						}),
					},
					ui.show(function(): boolean
						local d = drag:get()
						return d ~= nil and not d.out
					end, ui.presence(function(): string
						local d = drag:get()
						return if d then d.key else ""
					end, function(id: string): Node
						if id == "" then
							return ui.div {}
						end
						local function always(): boolean
							return true
						end
						local pinned = table.find(splits.pinned:peek(), id) ~= nil
						local rider: { [any]: any } = tab_box({
							w = function(): number
								local d = drag:get()
								return if d then d.w else width_of(id)
							end,
							pl = if pinned then 0 else nil,
							pr = if pinned then 0 else nil,
							overflow = "visible",
						}, always, function(): boolean
							return false
						end, if pinned
							then function(): number
								return 1
							end
							else nil)
						if pinned then
							table.insert(rider, tile_face(id, rider_hover))
						else
							for _, part in face_row(id, rider_hover, always) do
								table.insert(rider, part)
							end
						end
						return ui.div {
							id = "session-tab-rider",
							absolute = true,
							left = function(): number
								return rider_x:get()
							end,
							top = DROP,
							pointer_events = "none",
							ui.row(rider),
						}
					end)),
				},
			},
		},
		ui.show(function(): boolean
			return drag:get() ~= nil
		end, ui.div {
			id = "session-tabs-band",
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			h = BAR + LIFT,
			pointer_events = "auto",
			drop_when = function(payload: any): boolean
				return type(payload) == "table" and payload.session_tab ~= nil
			end,
			on_drag_over = over_band,
			on_drag_leave = leave_strip,
			on_drop = function()
				landed = true
			end,
		}),
	}
end

M.chat_of = chat_of
M.close = close

return M
