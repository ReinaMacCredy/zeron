--!strict
-- A lane in a chat's panel (screens/panel): a side chat, or a subagent, as
-- the shipping app's right-pane tabs show them (RightSurface::SideChat,
-- ::Subagent). The host watches its chat beside the open one, in a slot of
-- its own (the tab's id: `chat.open_lane`, let go as the tab goes). A side
-- chat is its transcript and a composer of its own (its draft its own, not
-- the chat page's); a subagent's is its transcript alone, to read.
--
--   LaneTab { tab = { id, kind = "side_chat" | "subagent", chat, title } }

local ds = require("@ds")
local Dictation = require("@app/screens/chat/dictation")
local T = require("@ds/transcript")
local lanes = require("@app/screens/chat/lanes")
local TurnGlue = require("@app/screens/chat/turn_glue")
local store = require("@app/screens/chat/store")
local model_switch = require("@app/screens/chat/model_switch")
local composer_profiles = require("@app/screens/chat/composer_profiles")
local prefs = require("@app/screens/settings/prefs")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local LinkNotice = require("@app/screens/chat/link_notice")
local route = require("@app/screens/shell/route")
local Where = require("@app/screens/chat/where")
local Dock = require("@app/screens/chat/dock")
local Pills = require("@app/screens/chat/pills")
local CloudTray = require("@app/screens/chat/cloud_tray")
local LinkNotice = require("@app/screens/chat/link_notice")
local ReplyImages = require("@app/screens/chat/reply_images")
local FileLinks = require("@app/screens/chat/file_links")
local RowProps = require("@app/screens/chat/row_props")
local LinkChips = require("@app/screens/chat/link_chips")
local LinkMenu = require("@app/screens/chat/link_menu")
local boundary = require("@app/screens/shell/boundary")
local Draft = require("@ds/draft_text")
-- (The chat page's places in its chats, which a split's panes share.)
local Shared = require("@app/screens/chat/shared")
local Asks = require("@app/screens/chat/asks")
local Completions = require("@app/screens/chat/completions")
local Find = require("@app/screens/chat/find")
local ZeronCommands = require("@app/screens/chat/zeron_commands")
local AppCommands = require("@app/screens/chat/app_commands")
local Outgoing = require("@app/screens/chat/outgoing")
local PanelState = require("@app/screens/panel/state")
local look = require("@app/screens/shell/project_look")
local DropOverlay = require("@ds/drop_overlay")
local MessageRail = require("@ds/message_rail")

-- Two places in a transcript (ds/transcript's `keep`), the same.
local function same(a: any, b: any): boolean
	if a == b then
		return true
	end
	if type(a) ~= "table" or type(b) ~= "table" then
		return false
	end
	for k, v in a do
		if not same(v, b[k]) then
			return false
		end
	end
	for k in b do
		if a[k] == nil then
			return false
		end
	end
	return true
end

-- What its tab says, and its icon, as the shipping app's.
local M = {}

-- A side chat's row in the session list, as it stands now.
local function row_of(chat: string?): any
	if not chat then
		return nil
	end
	for _, list in sessions.now:get().children or {} do
		for _, c in list do
			if c.id == chat then
				return c
			end
		end
	end
	return nil
end

function M.title(tab: any): string
	-- (Its chat's name as it's named or renamed; before that, as it was opened.)
	local row = if tab.kind == "side_chat" then row_of(tab.chat) else nil
	if row and row.title and row.title ~= "" then
		return row.title
	end
	if tab.title and tab.title ~= "" then
		return tab.title
	end
	return locale.s(if tab.kind == "subagent" then "chat.lane_tab.subagent" else "chat.lane_tab.side_chat")
end

function M.icon(tab: any): string
	if tab.kind == "subagent" then
		return "side-chat"
	end
	local c = if tab.chat then lanes.child_row(tab.chat) else nil
	local kind = if lanes.forks:peek()[tab.chat or ""] then "fork" else (if c then c.kind else nil)
	if kind == "fork" then
		return "git-branch"
	elseif kind == "agent" then
		return "side-chat"
	end
	return "side-chat"
end

-- At work: a side chat whose chat is working, a subagent still running (gpui's `subagent_running`).
function M.running(tab: any): boolean
	if tab.kind == "subagent" then
		for _, s in lanes.subagents() do
			if s.ref == tab.chat then
				return s.status == "running" or s.status == "working"
			end
		end
		return false
	end
	local c = if tab.chat then lanes.child_row(tab.chat) else nil
	return c ~= nil and c.status == "working"
end

-- The shipping app's `child_surface_mark`.
function M.TabIcon(tab: any): Node
	return ui.presence(function(): string
		local mark: string? = nil
		if tab.kind == "subagent" then
			mark = (tab.data or {}).agent
			if not mark then
				local parent = (tab.data or {}).parent
				local p = if parent then Shared.chat_of(parent) else nil
				mark = if p then p.mark else nil
			end
		else
			local c = if tab.chat then lanes.child_row(tab.chat) else nil
			if c and c.kind == "agent" and not lanes.forks:get()[tab.chat or ""] then
				mark = c.mark or c.agent
			end
		end
		return if mark then `mark:{mark}` else `icon:{M.icon(tab)}`
	end, function(key: string): Node
		local mark = string.match(key, "^mark:(.+)$")
		if mark then
			return ds.AgentMark.Mark { label = mark, mark = mark, seed = mark, size = 14, bare = true }
		end
		return ds.Icon { name = string.sub(key, 6) :: any, size = 14 }
	end, { shrink = 0 })
end

-- A side chat's project: where its composer has it before its first message,
-- then its chat's; its parent's while neither is known.
local function project_of(tab: any, picked: State<string?>): any
	local chat = tab.chat
	local parent = lanes.unsent:get()[chat]
	local id: string? = nil
	if parent then
		id = picked:get()
	else
		local s = lanes.view(tab.id).session:get()
		if s and s.id == chat and s.harness then
			id = s.project
			parent = nil
		else
			local row = row_of(chat)
			parent = if row then row.parent else nil
		end
	end
	local now = sessions.now:get()
	if not id and parent then
		for _, c in now.chats do
			if c.id == parent then
				id = c.project
			end
		end
	end
	for _, p in now.projects do
		if p.id == id then
			return p
		end
	end
	return nil
end

-- A side chat's tile in its tab: its project's, as a chat's on the top bar (one
-- with no project, the side chat's glyph, there too).
function M.tile(tab: any): Node
	local picked = ui.state(`chat_lane_project_{tab.chat}`, nil :: string?)
	return ui.presence(function(): string
		local p = project_of(tab, picked)
		return if p then p.id else ""
	end, function(which: string): Node
		if which == "" then
			return ds.Tile { size = 20, inside = ds.Icon { name = "side-chat", size = 14 } }
		end
		return ds.Tile {
			size = 20,
			label = function(): string?
				local p = project_of(tab, picked)
				return if p then p.name else nil
			end,
			hue = function(): string?
				local p = project_of(tab, picked)
				return if p then look.of(p).hue else nil
			end,
			src = function(): string?
				local p = project_of(tab, picked)
				return if p then look.of(p).src else nil
			end,
			emoji = function(): string?
				local p = project_of(tab, picked)
				return if p then look.of(p).emoji else nil
			end,
			project = function(): string?
				local p = project_of(tab, picked)
				return if p then p.id else nil
			end,
		}
	end, { w = 20, h = 20, shrink = 0 })
end

-- What a side chat's tab says at its end: whether it awaits a person or is at
-- work (what a chat's tab on the top bar says).
local TAB_STATES: { [string]: boolean } = { approval = true, input = true, working = true, connecting = true }
function M.state(tab: any): string?
	local row = row_of(tab.chat)
	local kind = if row then row.status else nil
	return if kind and TAB_STATES[kind] then kind else nil
end

-- Each tab's find bar and model picker, by its slot, while it's made.
local finders: { [string]: any } = {}
local pickers: { [string]: State<number> } = {}
local steppers: { [string]: State<{ n: number, by: number }> } = {}
-- Each chat's composer field here, by its chat: where a passage mentioned from its transcript goes.
local fields: { [string]: any } = {}

function M.open_find(tab: any): boolean
	local find = finders[tab.id]
	return find ~= nil and find.open()
end

function M.open_model(tab: any): boolean
	local picker = pickers[tab.id]
	if not picker then
		return false
	end
	picker:add(1)
	return true
end

function M.step_effort(tab: any, by: number): boolean
	local stepper = steppers[tab.id]
	if not stepper then
		return false
	end
	stepper:set({ n = stepper:peek().n + 1, by = by })
	return true
end

-- A side chat's composer: the chat page's (ds.Composer), its footer the
-- same row: the access mode, where it works, the model. Its draft is its
-- own. Its picks start as its chat has them (the engine's `harness/model`,
-- its access, its place), or, before its first message makes it, as its
-- parent's are (the shipping app's `create_child_chat` copies the parent's
-- config and place): the parent's agent and model (an agent with no model
-- named starts on that agent's first), its access, its project, device and
-- checkout. Until then where it works is a picker, as a new chat's (Cloud
-- left out: a side chat of a Cloud chat shares its machine); after, it's
-- said, fixed, as a session's is. A pick the person made stays theirs, kept
-- by the chat across tab switches and a reload, and every message goes with
-- what's picked (a changed agent, model or access becomes the chat's, as the
-- chat page's does). A message that didn't go says so on the composer's own
-- notice line, above it, never over it.
-- How far a lane's pill row's pills reach, by its slot.
local function pills_reach(slot: string): State<number>
	return ui.state(`chat_lane_pills_reach_{slot}`, 0)
end

local function composer(chat: string, view: any, name: string, focus: State<number>, open_model: State<number>, step_effort: State<{ n: number, by: number }>, split: boolean?, focused: (() -> boolean)?, panel_key: string?, tab_id: string?, placeholder: string?): Node
	local reach = pills_reach(tab_id or name)
	local lifted = Dock.lift(tab_id or name)
	-- (Its draft kept by the chat, as its picks: a switch of tabs or a
	-- reload finds it as it was; a passage quoted from its transcript lands
	-- in it.)
	-- (A pane of the page's split that isn't focused shows the chat
	-- page's own draft for its chat, as that keeps it while another chat
	-- is shown (screens/chat/shared's drafts), so focused, the chat page's
	-- composer goes on with the same words.)
	local draft = ui.state(if split then `chat_split_draft_{chat}` else `chat_lane_draft_{chat}`, "")
	-- (Asked for by the panel, or by keys typed with the panel holding them: the
	-- field takes them as soon as it's there, as gpui's `focus_pending`.)
	local field = ui.ref()
	-- (A split's pane types in the window's composer: its own, hidden while
	-- it's focused, is never where a passage goes.)
	if not split then
		fields[chat] = field
		ui.on_cleanup(function()
			if fields[chat] == field then
				fields[chat] = nil
			end
		end)
	end
	local take_on_mount = ui.use_state(false)
	local function take_keys()
		local el = field:peek()
		if el then
			el:focus()
			local n = #draft:peek()
			el:set_selection_bytes(n, n)
		else
			take_on_mount:set(true)
		end
	end
	ui.effect(function()
		if take_on_mount:get() and field:get() then
			take_on_mount:set(false)
			take_keys()
		end
	end)
	if panel_key and tab_id then
		ui.on_cleanup(PanelState.focuser(panel_key, tab_id, take_keys))
	end
	-- Its draft's chips: one draft's per chat (screens/chat/shared), wherever
	-- it's kept.
	local carried = ui.use_state({} :: { any })
	ui.effect(function()
		carried:set(Shared.cargo_of(chat))
	end)
	-- (A split's pane: as the focus comes or goes, its size there at once,
	-- never growing into it (ds.Composer's `snap`).)
	local snap = ui.use_state(0)
	if split and focused then
		local was = focused()
		ui.effect(function()
			local now = focused()
			if now ~= was then
				was = now
				snap:set(snap:peek() + 1)
			end
		end)
	end
	-- A chip taken off its draft, wherever the draft is now.
	local function remove_carried(cid: string)
		Shared.remove_cargo_of(chat, cid)
	end
	-- The options picked in a question its chat asks.
	local chosen = ui.use_state({} :: { [string]: boolean })
	-- (Words kept from before a side chat's draft lived with the chat's.)
	if not split and draft:peek() ~= "" and Shared.draft_peek(chat).text == "" then
		Shared.set_draft_of(chat, "text", draft:peek())
	end
	-- (Its chat's draft, by its id: its words, chips and a question's
	-- picks, one draft per chat (screens/chat/shared), shown here and
	-- written back as they change, wherever it's kept. Never the focused
	-- chat's: the focus moving changes nothing this shows.)
	--
	-- Each side's write is told to the other by its length (`pulled`,
	-- `pushed`), so its echo isn't read back: both read the whole draft
	-- and compared it at every keystroke, both ways (3 ms a key at
	-- 30,000 lines). By length, so a write that changed nothing, and so
	-- came back as no run, never hides the next change. `known`: this
	-- pane's draft's length as last seen, so the mirror from the chat's
	-- draft reads this one's only where the lengths agree.
	local pulled, pushed, known = nil :: number?, nil :: number?, -1
	ui.effect(function()
		Shared.track_draft_of(chat)
		local echo = pushed
		pushed = nil
		local size = Shared.draft_size_of(chat)
		if echo ~= nil and size == echo then
			return
		end
		local d = Shared.draft_of(chat)
		if size ~= known or draft:peek() ~= d.text then
			pulled, known = size, size
			draft:set(d.text)
		end
		chosen:set(d.chosen)
	end)
	local first = true
	ui.effect(function()
		local size = Draft.size(draft)
		known = size
		local echo = pulled
		pulled = nil
		if echo ~= nil and size == echo then
			return
		end
		-- (Typed: it differs. Made: it may not.)
		local text = draft:peek()
		if first then
			first = false
			if Shared.draft_of(chat).text == text then
				return
			end
		end
		pushed = size
		Shared.set_draft_of(chat, "text", text)
	end)
	ui.effect(function()
		local picked = chosen:get()
		if Shared.chosen_of(chat) ~= picked then
			Shared.set_draft_of(chat, "chosen", picked)
		end
	end)
	local notice = ui.use_state(nil :: any)
	local shown_notice = ui.computed(function(): any
		local refused = Shared.draft_of(chat).refused
		return notice:get() or (if refused then { tone = "danger", text = refused, dismissable = true } else nil)
	end)
	-- What its chat asks, and its queue, todos and tasks, from its own
	-- session, as the window's composer shows them.
	local asks = Asks.bind {
		id = function(): string?
			return chat
		end,
		session = function(): any
			return view.session:get()
		end,
		chosen = chosen,
		asked = ui.state(`chat_split_asked_{chat}`, nil :: any),
		notice = function(n: any)
			notice:set(n)
		end,
		-- (A steer glides in its own transcript, as its sends do.)
		name = name,
		say = function(tone: string, key: string)
			notice:set({ tone = tone, text = locale.s(key), dismissable = true })
		end,
	}
	local tray = ui.state(`chat_split_tray_{chat}`, nil :: any)
	local session = view.session
	-- (Its words written through first, so the draft's read-back above keeps them.)
	local function set_cargo(list: { any })
		Shared.set_draft_of(chat, "text", draft:peek())
		Shared.set_cargo_of(chat, list)
	end
	-- Its Cloud machine, on a chat that has one (a side chat shares its
	-- parent's).
	local machine = CloudTray.bind {
		chat = function(): string?
			return chat
		end,
		device = function(): string?
			local s = session:get()
			return s and s.device
		end,
		running = function(): boolean
			local s = session:get()
			return s ~= nil and s.running == true
		end,
	}
	-- A queued message edited in its composer. (A split's pane: a press
	-- focuses it, the window's composer doing this.)
	local editing = ui.state(`chat_lane_editing_{chat}`, nil :: { item: string, aside: string, aside_cargo: { any }? }?)
	local edits = Asks.queue_edits {
		id = function(): string?
			return chat
		end,
		text = draft,
		editing = editing,
		focus = take_keys,
		say = function(tone: string, key: string)
			notice:set({ tone = tone, text = locale.s(key), dismissable = true })
		end,
		cargo = function(): { any }
			return Shared.draft_peek(chat).cargo
		end,
		set_cargo = set_cargo,
		refs = function(): string
			return Shared.refs_key(chat)
		end,
	}
	-- A tab closed mid-edit lets go of the edit, and made again finds its row
	-- no longer held: the draft it set aside back. (A row gone is another
	-- matter: it keeps the words, and saving says so.)
	if not split then
		ui.effect(function()
			local e = editing:get()
			local s = session:get()
			if not e or not s or s.state == "loading" then
				return
			end
			for _, row in s.queue or {} do
				if row.id == e.item and not row.editing then
					ui.batch(function()
						editing:set(nil)
						draft:set(e.aside)
						if e.aside_cargo then
							set_cargo(e.aside_cargo)
						end
					end)
				end
			end
		end)
	end
	local running = ui.computed(function(): boolean
		local s = session:get()
		return s ~= nil and s.running == true
	end)
	-- (Kept by the chat, outside the view: a pick stays across a switch of
	-- tabs and a reload.)
	local model = ui.state(`chat_lane_model_{chat}`, nil :: string?)
	local effort = ui.state(`chat_lane_effort_{chat}`, nil :: string?)
	local fast = ui.state(`chat_lane_fast_{chat}`, false)
	local options = ui.state(`chat_lane_options_{chat}`, {} :: { [string]: string })
	local access = ui.state(`chat_lane_access_{chat}`, nil :: string?)
	local project = ui.state(`chat_lane_project_{chat}`, nil :: string?)
	local device = ui.state(`chat_lane_device_{chat}`, nil :: string?)
	local checkout = ui.state(`chat_lane_checkout_{chat}`, nil :: string?)
	local plan = ui.state(`chat_lane_plan_{chat}`, false)
	local branch = ui.state(`chat_lane_branch_{chat}`, nil :: string?)
	-- Where it was seeded from (its parent's place), so a send can tell
	-- whether the person moved it.
	local seed = ui.state(`chat_lane_place_{chat}`, nil :: { [string]: any }?)
	local function unsent(): boolean
		return lanes.unsent:get()[chat] ~= nil
	end
	-- Its parent: the open chat's record when that's it (its access is
	-- there), else its row in the sidebar's list.
	local function parent_of(): (any?, any?)
		local parent = lanes.unsent:get()[chat]
		if not parent then
			return nil, nil
		end
		local row: any = nil
		for _, c in sessions.now:get().chats do
			if c.id == parent then
				row = c
			end
		end
		local open = store.session:get()
		return row, if open and open.id == parent then open else nil
	end
	-- What it runs as its chat (or its parent) has it: the agent, the
	-- model's picker value, the effort, the access.
	local function as_made(): (string?, string?, string?, string?)
		local s = session:get()
		if s and s.harness then
			return s.harness, s.model, s.effort, s.access
		end
		local row, open = parent_of()
		if open and open.harness then
			return open.harness, open.model, open.effort, open.access
		end
		if row then
			local m = row.model
			if m and row.harness and not string.find(m, "/", 1, true) then
				m = `{row.harness}/{m}`
			end
			return row.harness, m, nil, nil
		end
		return nil, nil, nil, nil
	end
	-- (What was put in the pickers last from the chat: a pick that differs
	-- is the person's, and stays. Kept by the chat too, so a tab made again,
	-- or a reload, tells the person's pick from the chat's.)
	local seeded = ui.state(`chat_lane_seeded_{chat}`, {} :: { [string]: any })
	ui.effect(function()
		local harness, want, eff, acc = as_made()
		if not want and harness then
			for _, m in store.catalog:get().models do
				if m.harness == harness then
					want = m.value
					break
				end
			end
		end
		acc = acc or "full"
		local was = seeded:peek()
		local now = table.clone(was)
		-- (Never seeded: whatever stands is the chat's to replace.)
		local first = next(was) == nil
		if want then
			if (first or model:peek() == was.model) and model:peek() ~= want then
				model:set(want)
			end
			now.model = want
		end
		if (first or effort:peek() == was.effort) and effort:peek() ~= eff then
			effort:set(eff)
		end
		now.effort = eff
		if (first or access:peek() == was.access) and access:peek() ~= acc then
			access:set(acc)
		end
		now.access = acc
		-- What's kept of a chat that runs: its level, Fast and options, as they
		-- stand there.
		local s: any = session:get()
		if s then
			local resolved = Shared.resolve_model(s, store.catalog:get())
			if resolved.effort and (first or effort:peek() == was.effort or effort:peek() == nil) and effort:peek() ~= resolved.effort then
				effort:set(resolved.effort)
			end
			now.effort = resolved.effort or now.effort
			if (first or fast:peek() == was.fast) and fast:peek() ~= (s.fast == true) then
				fast:set(s.fast == true)
			end
			now.fast = s.fast == true
			local given = s.options or {}
			if (first or Shared.same_pick(options:peek(), was.options)) and not Shared.same_pick(options:peek(), given) then
				options:set(given)
			end
			now.options = given
		end
		-- (Plan mode the same; before it runs, its parent's Fast and options too.)
		local made = session:get()
		if not made or not made.harness then
			local _, open = parent_of()
			made = open
		end
		for key, st in (if s then { plan = plan } else { fast = fast, options = options, plan = plan }) :: { [string]: any } do
			local v = if made then made[key] else nil
			-- (Off is what one with none set runs with.)
			local before = if was[key] ~= nil then was[key] elseif key == "options" then nil else false
			if v ~= nil and (first or Shared.same_pick(st:peek(), before)) and not Shared.same_pick(st:peek(), v) then
				st:set(v)
			end
			now[key] = v
		end
		if not same(now, was) then
			seeded:set(now)
		end
	end)
	-- A pick on a chat that runs lands on it at once, as the chat page's does
	-- (a side chat's first message still carries its own).
	local landing: model_switch.Ask? = nil
	ui.effect(function()
		local picked: { [string]: any } = { model = model:get(), effort = effort:get(), fast = fast:get(), options = options:get(), plan = plan:get() }
		local s: any = session:peek()
		if unsent() or not s or s.state ~= "ready" then
			return
		end
		local was = seeded:peek()
		local patch: { [string]: any } = {}
		local any_change = false
		for k, v in picked do
			if v ~= nil and not Shared.same_pick(v, was[k]) and not Shared.same_pick(v, s[k]) then
				patch[k] = v
				any_change = true
			end
		end
		if not any_change then
			return
		end
		if patch.model then
			for _, m in store.catalog:peek().models do
				if m.value == patch.model then
					patch.harness = m.harness
				end
			end
		end
		if landing then
			patch.note = landing.note
			patch.note_hint = landing.note_hint
			landing = nil
		end
		store.configure(chat, patch)
		local now = table.clone(was)
		for k, v in picked do
			now[k] = v
		end
		seeded:set(now)
	end)
	local close_picker = ui.state(`chat_lane_model_close_{chat}`, 0)
	local take_model, switch_dialog = model_switch.use(chat, {
		current = function(): string?
			return model:peek()
		end,
		history = function(): boolean?
			if unsent() or session:peek() == nil then
				return nil
			end
			return model_switch.has_history(view.messages:peek(), running:peek(), (session:peek() :: any).state == "ready")
		end,
		land = function(value: string, ask: model_switch.Ask?)
			landing = ask
			model:set(value)
			close_picker:set(close_picker:peek() + 1)
			-- (The keys back to the draft, once the dialog and its modal have gone.)
			ui.after(350, function()
				local el = field:peek()
				if el then
					el:focus()
				end
			end)
		end,
		back = function()
			open_model:add(1)
		end,
	})
	-- Its agent profiles, on this chat (before its first message, its own
	-- pick, sent with it).
	local lane_profile = ui.state(`chat_lane_profile_{chat}`, nil :: string?)
	local open_profiles = ui.state(`chat_lane_profiles_open_{chat}`, 0)
	local close_profiles = ui.state(`chat_lane_profiles_close_{chat}`, 0)
	local profiles, profile_dialog = composer_profiles.bind {
		key = chat,
		chat = function(): string?
			return if unsent() or session:get() == nil then nil else chat
		end,
		session = function(): any?
			local s: any = session:get()
			return if s and s.state == "ready" then s else nil
		end,
		committed = function(): boolean
			return not unsent() and session:get() ~= nil
		end,
		picks = function(): composer_profiles.Picks
			return { model = model:get(), effort = effort:get(), access = access:get(), options = options:get(), fast = fast:get() }
		end,
		set_picks = function(p: composer_profiles.Picks)
			model:set(p.model)
			effort:set(p.effort)
			access:set(p.access)
			options:set(p.options or {})
			fast:set(p.fast == true)
		end,
		draft_profile = lane_profile,
		history = function(): boolean?
			local s: any = session:peek()
			return model_switch.has_history(view.messages:peek(), running:peek(), s ~= nil and s.state == "ready")
		end,
		reopen = function()
			open_profiles:set(open_profiles:peek() + 1)
		end,
		refocus = function()
			local el = field:peek()
			if el then
				el:focus()
			end
		end,
		close = close_profiles,
	}
	-- Before its first message: where it works starts where its parent
	-- does (once, as the parent is known).
	ui.effect(function()
		if not unsent() or seed:peek() ~= nil then
			return
		end
		local row = parent_of()
		if not row then
			return
		end
		local place = {
			project = row.project or Where.NO_PROJECT,
			device = row.device,
			checkout = "local",
			branch = row.branch,
		}
		-- (Started elsewhere, the panel home's group "+": that project or
		-- device instead, its branch the project's own. The seed stays the
		-- parent's, so the send carries the place.)
		local at = lanes.starts:peek()[chat]
		local start_project = if at and at.project then at.project elseif at and at.device and at.device ~= place.device then nil else place.project
		local start_device = if at and at.device then at.device else place.device
		local moved = start_project ~= place.project or start_device ~= place.device
		project:set(start_project)
		device:set(start_device)
		checkout:set(place.checkout)
		branch:set(if moved then nil else place.branch)
		seed:set(place)
	end)
	local where = Where.bind {
		device = device,
		project = project,
		checkout = checkout,
		branch = branch,
		active = unsent,
		slot = `lane.{chat}`,
		remember = false,
	}
	-- (No Cloud machine of its own for a side chat.)
	local devices = ui.computed(function(): { any }
		local out = {}
		for _, d in where.devices:get() do
			if not d.cloud then
				table.insert(out, d)
			end
		end
		return out
	end)
	-- Once made: where it works, said.
	local function branch_fixed(): any
		if unsent() then
			return nil
		end
		local s = session:get()
		local function project_row(id: string?): any?
			for _, p in sessions.projects:get() do
				if p.id == id then
					return p
				end
			end
			return nil
		end
		if not s or s.harness == nil then
			-- (Just made, its chat not heard yet: where it was made.)
			local p = project_row(project:get())
			return {
				name = branch:get(),
				worktree = checkout:get() == "worktree",
				project = if p then p.name else nil,
				hue = if p then (route.look(p.id) or { hue = route.hue(p.id) }).hue else nil,
				src = if p then (route.look(p.id) or {}).src else nil,
				emoji = if p then (route.look(p.id) or {}).emoji else nil,
			}
		end
		local p = project_row(s.project)
		local on: string? = nil
		for _, d in sessions.devices:get() do
			if d.id == s.device then
				on = d.name
			end
		end
		return {
			device = on,
			name = s.branch,
			worktree = p ~= nil and s.cwd ~= nil and s.cwd ~= p.path,
			project = if p then p.name else nil,
			hue = if p then (route.look(p.id) or { hue = route.hue(p.id) }).hue else nil,
			src = if p then (route.look(p.id) or {}).src else nil,
			emoji = if p then (route.look(p.id) or {}).emoji else nil,
		}
	end
	-- What a message goes with: the pick, its agent, the effort, the access.
	local function picks(): { [string]: any }
		local m = model:peek()
		local out: { [string]: any } = { model = m, effort = effort:peek(), access = access:peek(), fast = fast:peek(), options = options:peek(), plan = plan:peek() }
		for _, c in store.catalog:peek().models do
			if c.value == m then
				out.harness = c.harness
			end
		end
		if not out.harness and type(m) == "string" then
			out.harness = string.match(m, "^([^/]+)/")
		end
		return out
	end
	-- Where its first message makes it: nil, its parent's place as it is;
	-- else where the person moved it (`where.plan()`).
	local function branch_kept(was: string?): boolean
		local now = branch:peek()
		if now == was then
			return true
		end
		-- (A parent's branch git doesn't list is shown as git's current, not picked.)
		local current: string? = nil
		for _, ref in where.refs:peek().list do
			if ref.name == was then
				return false
			end
			if ref.current then
				current = ref.name
			end
		end
		return now == current
	end
	local function place(): { [string]: any }?
		local was = seed:peek()
		if
			was
			and project:peek() == was.project
			and device:peek() == was.device
			and checkout:peek() == was.checkout
			and branch_kept(was.branch)
		then
			return nil
		end
		return where.plan() :: any
	end
	-- A new side chat's composer takes the keys as it's made.
	if lanes.take_keys:peek()[chat] then
		ui.on_mount(function()
			local left = table.clone(lanes.take_keys:peek())
			left[chat] = nil
			lanes.take_keys:set(left)
			focus:add(1)
		end)
	end
	-- (A chat sent already has its host in the session: `device` is only
	-- seeded while it's unsent, so reading it alone asked this computer's
	-- engine for a chat running on another's.)
	local accounts = Shared.model_accounts(function(): string?
		if unsent() then
			return device:get()
		end
		local s = session:get()
		return if s then s.device else nil
	end)
	-- What `@`, `/` and `$` offer, as the chat page's. (A split's pane: the
	-- window's composer types, a press focusing it.)
	local function run_command(command: string)
		if command == "model" then
			open_model:add(1)
		else
			ZeronCommands.run(command, if unsent() then nil else chat)
		end
	end
	local completions = if split
		then nil
		else Completions.bind {
			key = `chat_lane_{chat}`,
			text = draft,
			agent = function(): string?
				return Completions.agent_of(model:get(), if unsent() then nil else session:get())
			end,
			target = function(): any
				if unsent() then
					local p = project:get()
					return if p and p ~= Where.NO_PROJECT then { space = p, device = device:get() } else nil
				end
				local s = session:get()
				return { chat = chat, device = if s then s.device else nil }
			end,
			files = function(): any
				if unsent() then
					local p = project:peek()
					return if p and p ~= Where.NO_PROJECT then { space = p, device = device:peek() } else nil
				end
				local s = session:peek()
				return { chat = chat, device = if s then s.device else nil }
			end,
			in_chat = function(): boolean
				return not unsent()
			end,
			run = run_command,
			-- (`@` lists what the draft carries, and the other chats.)
			cargo = function(): { any }
				return carried:get()
			end,
			chat = function(): string?
				return chat
			end,
		}
	local dictation = Dictation.bind(draft, function(): string
		local edit = editing:get()
		return `{chat}|{name}|{if edit then edit.item else ""}`
	end, function(): boolean
		local current = session:get()
		return asks.question:get() ~= nil or asks.approval:get() ~= nil
			or (current ~= nil and (current.state == "missing" or current.state == "offline"))
			or (focused ~= nil and not focused())
	end, name)
	local queue_pictures = ReplyImages.for_attachments(function(): any
		local s = session:get()
		if not s or s.state == "missing" then
			return nil
		end
		return { id = s.id, device = s.device, cwd = s.cwd, here = sessions.now:get().here }
	end)
	-- A side chat past its day: nothing more goes to it (Zeron's composer.rs).
	local function expired(): boolean
		if split then
			return false
		end
		for _, list in sessions.now:get().children or {} do
			for _, c in list do
				if c.id == chat then
					return c.expires ~= nil and c.expires <= os.time()
				end
			end
		end
		return false
	end
	local expired_line = ui.row {
		id = "expired-sidechat",
		w_full = true,
		p = "$space.3",
		ui.text { ui.t("chat.composer.expired"), grow = 1, wrap = true, text_size = "$font.size.sm", color = "$text.tertiary" },
	}
	local cargo_props = Shared.cargo_props(chat)
	local function send_text(text: string, opts: { resume: boolean? }?)
		local say = function(n: any)
			notice:set(n)
		end
		-- Editing a queued message: Send saves it.
		if editing:peek() then
			edits.finish("commit")
			return
		end
		local side = AppCommands.side(text)
		if side then
			draft:set("")
			lanes.open_side(chat, side, route.current:peek().session)
			return
		end
		local compact_focus = AppCommands.compact(text)
		if compact_focus then
			draft:set("")
			AppCommands.run_compact(chat, session:peek(), running:peek(), compact_focus, function(key: string)
				say({ tone = "danger", text = locale.s(key), dismissable = true })
			end, function(why: string)
				say({ tone = "danger", text = why, dismissable = true })
			end)
			return
		end
		local trimmed = string.match(text, "^%s*(.-)%s*$") or ""
		-- A draft that is only one of Zeron's own commands runs it.
		local command = if completions then completions.command_of(text) else nil
		if command then
			draft:set("")
			run_command(command)
			return
		end
		-- (Nowhere it can start, a project gone: the line says why.)
		local is_unsent = lanes.unsent:peek()[chat] ~= nil
		if is_unsent and where.blocked:peek() then
			say({ tone = "danger", text = where.blocked:peek(), dismissable = true })
			return
		end
		local s = session:peek()
		local goes_to = if s and s.device then s.device else device:peek()
		if goes_to ~= nil and goes_to ~= sessions.now:peek().here then
			local too_large = Shared.linked_elsewhere(carried:peek())
			if too_large then
				say({ tone = "danger", text = too_large, dismissable = true })
				return
			end
		end
		-- (What's typed reaches the chat's draft by an effect: this moment's.)
		Shared.set_draft_of(chat, "text", draft:peek())
		local typed, cargo = Shared.take(chat)
		local parts = Outgoing.parts(string.match(typed, "^%s*(.-)%s*$") or "", trimmed, cargo)
		draft:set("")
		notice:set(nil)
		-- As the chat page sends: the prompt joins its transcript as the
		-- transcript's send steps clear of the end (it glides to the top,
		-- its reply's room opening under it), then goes to the host with
		-- that id, so the host's copy is the same row.
		-- (While a turn runs it's queued instead: it shows in the queue, and
		-- what's picked waits for the turn it will run in.)
		local queued = not is_unsent and running:peek()
		local req: { [string]: any } = if queued then {} else picks()
		req.attachments = parts.attachments
		req.appshots = parts.appshots
		req.restores = true
		local function failed(err: any)
			Shared.restore(parts.text, cargo)
			local why = if type(err) == "table" and err.message then tostring(err.message) else nil
			say({
				tone = "danger",
				text = if is_unsent and why then why else locale.s("chat.lane_tab.didnt_send"),
				dismissable = true,
			})
		end
		if queued then
			req.session, req.text = chat, parts.text
			store.send(req :: any):after(function() end, failed)
			return
		end
		local mid = store.new_id()
		req.message_id = mid
		local catalog = store.catalog:peek()
		local from_h = T.session_harness(catalog, session:peek())
		local to_h = req.harness or (if type(req.model) == "string" then string.match(req.model, "^([^/]+)/") else nil)
		local sw = if not is_unsent then T.switch_row(catalog, from_h, to_h, mid) else nil
		local sent: any = nil
		T.send(function()
			local here = view.messages:index_of("id", mid) ~= nil
			if not here then
				if sw and view.messages:len() > 0 then
					local tail = view.messages:at(view.messages:len())
					if (not tail or tail.role ~= "switch") and view.messages:index_of("id", sw.id) == nil then
						view.messages:push(sw)
					end
				end
				-- (On its way from this moment: the line under it says so.)
				view.messages:push(if opts and opts.resume then Outgoing.resumed(mid) else { id = mid, role = "user", text = parts.words, attachments = parts.shown, waiting = "sending" })
			end
		end, mid, name)
		-- Its first message makes it (New side chat), then sends.
		if is_unsent then
			req.profile = composer_profiles.find(lane_profile:peek()) and lane_profile:peek()
			sent = lanes.send_first(chat, parts.text, req, place())
		else
			req.session, req.text = chat, parts.text
			sent = store.send(req :: any)
		end
		sent:after(function() end, failed)
	end
	return ui.column { w_full = true, switch_dialog, profile_dialog, ui.show(expired, expired_line, ds.Composer {
		field = field,
		dictation = dictation,
		value = draft,
		snap = if split then snap else nil,
		-- (Its transcript's "Jump to latest", while it shows: its trays'
		-- pill moves aside, the two a centred pair. By the lane's name, its
		-- transcript's: screens/chat/shared's `pill_width_of`.)
		aside = function(): number?
			return Shared.pill_width_of("jump", name):get()
		end,
		on_pill_width = Shared.pill_teller("waiting", name),
		cargo = carried,
		-- (Its chips' × under the pointer, as the chat page's.)
		on_remove_cargo = remove_carried,
		on_paste_text = cargo_props.on_paste_text,
		on_open_pasted = cargo_props.on_open_pasted,
		-- (What's dropped on it is its tab's to take, or a split pane's card,
		-- under their own overlays.)
		drop_overlay = false,
		refs = function(): string
			return Shared.refs_key(chat)
		end,
		-- Its + and a picture pasted: chips on its own draft, as the chat page's.
		-- (A split's pane: a press focuses it, the window's composer taking
		-- over.)
		on_attach = if split then nil else cargo_props.on_attach,
		on_paste_image = if split then nil else cargo_props.on_paste_image,
		focus = if split then nil else focus,
		-- (ds.Composer's placeholder is a string: read as the tab is made.)
		-- (A split's pane is a chat of its own: the chat page's words.)
		placeholder = locale.s(placeholder or if split then "chat.composer.placeholder" else "chat.lane_tab.placeholder"),
		running = running,
		access = access,
		access_auto = function(): boolean
			return store.offers_auto(model:get())
		end,
		plan = plan,
		model = model,
		effort = effort,
		fast = fast,
		options = options,
		model_take = take_model,
		model_committed = function(): boolean
			return not unsent() and session:get() ~= nil
		end,
		model_looking = function(): boolean
			return store.catalog:get().state ~= "ready"
		end,
		model_retry = store.load_catalog,
		model_close = close_picker,
		open_model = open_model,
		step_effort = step_effort,
		context_tokens = function(): number?
			local s: any = session:get()
			return if s then s.context_tokens else nil
		end,
		-- (Its own line first, else its Cloud machine's, on a chat with one.)
		notice = ui.computed(function(): any
			local mine = shown_notice:get()
			if mine then
				return mine
			end
			local link = machine.cloud_link(Draft.has_text(draft))
			return if link then LinkNotice.of { cloud = link } else nil
		end),
		-- The app's settings: what sends, and whether Escape stops the agent.
		send_with = function(): string
			return prefs.send_with:get()
		end,
		on_escape = function(): boolean
			return store.escape_chat(chat, running:peek())
		end,
		disabled = function(): boolean
			local s = session:get()
			return s ~= nil and (s.state == "missing" or s.state == "offline")
		end,
		mentions = if completions then completions.lists else nil,
		on_mention = if completions then completions.on_mention else nil,
		mention_empty = if completions then completions.empty else nil,
		mention_sections = if completions then completions.sections else nil,
		on_dismiss_notice = function()
			notice:set(nil)
			Shared.refuse_in(chat, nil)
		end,
		on_notice_action = machine.wake,
		alerts = machine.alerts,
		on_alert_action = machine.on_alert,
		on_dismiss_alert = machine.dismiss,
		harnesses = function(): { any }
			local c: any = store.catalog:get()
			return c.tabs or c.harnesses
		end,
		models_loading = function(): boolean
			return store.catalog:get().complete ~= true
		end,
		models = function(): { any }
			return store.catalog:get().models
		end,
		accounts = accounts.accounts,
		account_group = accounts.account_group,
		on_account = accounts.on_account,
		on_accounts_open = accounts.on_accounts_open,
		-- What its context holds: the chat page's ring and card, from the
		-- side chat's own numbers (once it has run and they're reported).
		usage = function(): any
			local s = session:get()
			if not s or (s.context_tokens == nil and s.context_window == nil) then
				return nil
			end
			return { tokens = s.context_tokens, window = s.context_window }
		end,
		stats = function(): any
			return view.stats:get()
		end,
		footer_git = function(): boolean
			local s = session:get()
			for _, p in sessions.now:get().projects do
				if p.id == (if s then s.project else nil) then
					return p.git == true
				end
			end
			return false
		end,
		footer_status = split == true,
		project = project,
		projects = where.projects,
		device = device,
		devices = devices,
		checkout = checkout,
		branch = branch,
		branches = where.branches,
		branches_empty = where.branches_empty,
		branch_fixed = branch_fixed,
		-- (Work in's "New project": the shell's New project palette.)
		on_new_project = function()
			local add: any = boundary.load("@app/screens/shell/add_project")
			if add then
				add.open()
			end
		end,
		-- As the window's composer sends (screens/chat/composer): it deals with
		-- the draft itself.
		on_send = send_text,
		resume = function(): boolean
			local s: any = session:get()
			return s ~= nil and s.stopped == true and not editing:get()
		end,
		on_resume = function()
			send_text(Outgoing.RESUME_PROMPT, { resume = true })
		end,
		on_stop = function()
			store.interrupt(chat)
		end,
		-- (Hidden while focused, its draft shown by the window's composer: only that
		-- one's menus open, or a trigger typed there opens a second here.)
		mention_enabled = if split and focused
			then function(_trigger: string): boolean
				return not focused()
			end
			elseif completions then completions.enabled
			else nil,
		-- (Its chat's own.)
		question = asks.question,
		chosen = asks.chosen,
		on_pick = asks.on_pick,
		on_answer = asks.on_answer,
		on_close_question = asks.on_close_question,
		on_back = asks.on_back,
		approval = asks.approval,
		can_always = asks.can_always,
		on_approve = asks.on_approve,
		on_deny = asks.on_deny,
		queue = asks.queue,
		can_steer = asks.can_steer,
		on_queue_send = asks.on_queue_send,
		on_queue_steer_all = asks.on_queue_steer_all,
		queue_held = function(): boolean
			return #asks.engine_queue:get() > 0
		end,
		queue_pictures = queue_pictures,
		on_dismiss_todos = asks.on_dismiss_todos,
		on_queue_edit = if split then nil else edits.on_queue_edit,
		on_queue_save = if split then nil else edits.on_queue_save,
		on_queue_cancel = if split then nil else edits.on_queue_cancel,
		on_queue_remove = asks.on_queue_remove,
		on_queue_reorder = asks.on_queue_reorder,
		todos = asks.todos,
		pills = Pills.Row {
			id = `composer-pills-{chat}`,
			keys = take_keys,
			bind = {
				id = function(): string?
					return chat
				end,
				session = function(): any
					return view.session:get()
				end,
				subs = function(): { any }
					return view.subagents:get()
				end,
			},
			tasks = asks.tasks,
			on_extent = function(w: number)
				if math.abs(reach:peek() - w) >= 0.5 then
					reach:set(w)
				end
			end,
		},
		pills_reach = function(): number
			return reach:get()
		end,
		on_lift = function(n: number)
			if lifted:peek() ~= n then
				lifted:set(n)
			end
		end,
		tray = tray,
	}) }
end

-- A fork's tab while the engine copies it, or once it couldn't: the
-- shipping app's words, and Retry (the same fork again).
local function forking(chat: string): Node
	return ui.presence(function(): string?
		local f = lanes.forks:get()[chat]
		return if f then f.state else nil
	end, function(state: string): Node
		if state == "failed" then
			return ds.EmptyState {
				icon = "git-branch",
				size = "sm",
				fill = true,
				title = ui.t("chat.lane_tab.couldnt_fork") :: any,
				body = function(): string
					local f = lanes.forks:get()[chat]
					return if f and f.error then f.error else ""
				end,
				action = {
					label = ui.t("chat.lane_tab.retry") :: any,
					on_click = function()
						lanes.retry_fork(chat)
					end,
				},
			}
		end
		return ds.EmptyState { icon = "git-branch", size = "sm", fill = true, title = ui.t("chat.lane_tab.forking") :: any }
	end, { grow = 1, w_full = true })
end

-- The shipping app's `delegate_owner_line`.
local function owner_line(chat: string, owner: string): Node
	local function title(): string
		local p = Shared.chat_of(owner)
		return if p then p.title else locale.s("chat.lane_tab.this_chat")
	end
	local c0 = lanes.child_row(chat)
	local mark = if c0 then c0.mark or c0.agent else nil
	return ui.row {
		id = `delegate-owner-{chat}`,
		w_full = true,
		justify = "center",
		items = "center",
		gap = ui.raw(6, "side_chats.rs delegate_owner_line"),
		py = ui.raw(4, "side_chats.rs delegate_owner_line"),
		px = "$space.3",
		text_size = ui.raw(11, "side_chats.rs delegate_owner_line"),
		color = "$text.secondary",
		selectable = false,
		ui.div {
			shrink = 0,
			color = "$text.tertiary",
			aria_hidden = true,
			if mark then ds.AgentMark.Mark { label = mark, mark = mark, seed = mark, size = 12, bare = true } else ds.Icon { name = "bot", size = 12 },
		},
		ds.FadeText {
			function(): string
				return locale.s("chat.lane_tab.subagent_of", { title = title() })
			end,
			literal = true,
			shrink = 1,
			min_w = 0,
		},
		ui.button {
			id = `delegate-owner-open-{chat}`,
			label = locale.t("chat.lane_tab.open_parent"),
			h = "auto" :: any,
			p = 0,
			bg = "#00000000",
			hover = { bg = "#00000000", color = "$text.primary" },
			active = { bg = "#00000000" },
			color = "$text.primary",
			cursor = "pointer",
			shrink = 0,
			on_click = function()
				route.open_session(owner)
			end,
			ui.text { locale.t("chat.lane_tab.open_parent"), wrap = false, weight = "$font.weight.medium" },
		},
	}
end

-- (Above the composer, in the dock: its measured height keeps the transcript clear.)
local function owners(chat: string, split: boolean?): Node
	return ui.presence(function(): string?
		if split then
			return nil
		end
		local c = lanes.child_row(chat)
		return if c and c.delegated_by then c.delegated_by else nil
	end, function(owner: string): Node
		return owner_line(chat, owner)
	end, { w_full = true, shrink = 0 })
end

-- A lane's composer's height as measured, by its slot.
function M.dock_height(slot: string): State<number>
	return ui.state(`chat_lane_dock_h_{slot}`, Dock.REST)
end

-- gpui's `sidechat_context_line`: a side chat (not a fork, not an agent's thread) names the chat it reads from.
local function context_of(chat: string): string?
	if lanes.forks:get()[chat] then
		return nil
	end
	local c = lanes.child_row(chat)
	if c and c.kind ~= nil and c.kind ~= "side_chat" then
		return nil
	end
	local parent = if c then c.parent else lanes.unsent:get()[chat]
	local p = if parent then Shared.chat_of(parent) else nil
	if not p or not p.title or p.title == "" then
		return nil
	end
	return locale.s("chat.lane_tab.sidechat_of", { title = p.title })
end

M.Tab = ui.component(function(props: { tab: any, panel_key: string? }): Node
	local tab = props.tab
	local slot, chat = tab.id, tab.chat
	local data = tab.data or {}
	local view = lanes.view(slot)
	-- What the tab is now: a fork under way or failed, or its chat (a new
	-- side chat's not yet made, until its first message).
	local kind = ui.computed(function(): string
		local f = lanes.forks:get()[chat]
		if f then
			return "fork"
		end
		return if lanes.unsent:get()[chat] then "unsent" else "chat"
	end)
	-- Files, and the files pane's, carried over it, dropped anywhere on it, as
	-- on the chat page: chips on its own draft (a sidebar chat, mentioned), its
	-- composer taking the keys. (A split's pane is the page's to take: its card.)
	local focus = ui.use_state(0)
	-- Bumped, its model picker opens (⌘/, /model), as the chat page's.
	local open_model = ui.use_state(0)
	local step_effort = ui.use_state({ n = 0, by = 0 })
	-- ⌘F's own, over its own transcript.
	local find = Find.bind(`chat.lane.{slot}.find`, view.messages, function(): boolean
		return true
	end, `lane-find-{slot}`, function(): string?
		local s = view.session:get()
		return if s and s.state ~= "missing" then s.id else nil
	end)
	finders[slot], pickers[slot], steppers[slot] = find, open_model, step_effort
	ui.on_cleanup(function()
		if finders[slot] == find then
			finders[slot], pickers[slot], steppers[slot] = nil, nil, nil
		end
	end)
	local zone, carrying, carrying_session = nil :: any, nil :: any, nil :: any
	if tab.kind == "side_chat" and data.split ~= true then
		zone, carrying, carrying_session = Shared.own_drop_zone(`lane.{slot}`, chat, function()
			focus:add(1)
		end)
	end
	-- Watched once there's a chat to watch, while its tab is open: the
	-- panel lets it go as the tab closes. (A reload asks for the same chat
	-- again, which the host already has: nothing more happens.)
	-- (A split's pane is watched by the page, for as long as it's in the
	-- split, focused or not.)
	local split = data.split == true
	ui.effect(function()
		if kind:get() == "chat" and not split then
			lanes.watch(slot, chat, data.frozen == true, data.parent)
		end
	end)
	ui.on_cleanup(function()
		lanes.on_screen(slot, false)
	end)
	-- The transcript as the chat page has it (screens/chat/session), its
	-- composer docked over its foot the same way (screens/chat/dock): the
	-- send's runway and glide, the working row and the jump-to-latest pill
	-- held clear of the composer, its place kept across a switch of tabs
	-- and a reload (its anchor and offset kept by the tab), pictures read
	-- from its own folder, links, quotes into its own draft.
	local offset = ui.state(if split then `chat_split_offset_{chat}` else `chat_lane_offset_{slot}`, T.PAST_END)
	local anchor = ui.state(if split then `chat_split_anchor_{chat}` else `chat_lane_anchor_{slot}`, nil :: any)
	-- (A split's pane. Its transcript is its own chat's, from its own
	-- lane, focused or not, and made once for as long as it's in the
	-- split: the focus moving changes nothing it shows. Only what stands
	-- at its foot follows the focus: the window's composer over the
	-- focused one, its own composer over another. It's made once there's
	-- something to show, opening where the chat was last read
	-- (`shared.places`).)
	local function focused_now(): boolean
		local f = data.focused
		return f ~= nil and f() == true
	end
	local loaded = ui.computed(function(): boolean
		local s = view.session:get()
		return s ~= nil and s.id == chat and s.state ~= "loading"
	end)
	local messages: any = view.messages
	local reply: any = view.draft
	-- Where the chat was left: as the chat page last showed it (its place
	-- now, if that's this chat), else its place kept for it.
	local function place_of(): any
		if Shared.shown:peek() == chat then
			return Shared.anchor:peek()
		end
		return Shared.places:peek()[chat]
	end
	-- Where it's read here, kept the same way, once it has settled: the
	-- chat page, a pane made again (the split shown again, the chat opened
	-- alone) open there.
	local settled = false
	if split then
		ui.effect(function()
			local a = anchor:get()
			if not settled then
				return
			end
			local now = Shared.places:peek()
			if not same(now[chat], a) then
				local c = table.clone(now)
				c[chat] = a
				Shared.places:set(c)
			end
			if Shared.shown:peek() == chat and not same(Shared.anchor:peek(), a) then
				Shared.anchor:set(a)
			end
		end)
	end
	local ever = false
	local showable = ui.computed(function(): boolean
		if ever then
			return true
		end
		ever = messages:len() > 0 or loaded:get()
		return ever
	end)
	local dock_h = M.dock_height(slot)
	-- (A split's pane, focused: the window's composer stands at its foot.)
	local own_lift, page_lift = Dock.lift(slot), Dock.lift("page")
	local room = Dock.room(function(): number
		return if split and focused_now() then Shared.dock_h:get() else dock_h:get()
	end, function(): number
		return if split and focused_now() then page_lift:get() else own_lift:get()
	end)
	local function lane_session(): any
		local s = view.session:get()
		return if s and s.state ~= "missing" then s else nil
	end
	local pictures = ReplyImages.for_session(lane_session)
	local sent_pictures = ReplyImages.for_attachments(function(): any
		local s = lane_session()
		if s then
			return { id = s.id, device = s.device, cwd = s.cwd, here = sessions.here:get() }
		end
		-- A subagent's lane has no chat of its own (its doc is its parent's
		-- child): the pictures its tools kept are files on its parent's
		-- host, read as the parent's are.
		local parent = data.parent
		local open = store.session:get()
		if not parent or not open or open.id ~= parent or open.state == "missing" then
			return nil
		end
		return { id = open.id, device = open.device, cwd = open.cwd, here = sessions.here:get() }
	end)
	-- The session whose panel it's in (the page's): where a file it links
	-- opens, and a subagent it spawned.
	local function home_session(): string?
		-- (A split's pane: its own chat's panel, which shows as it's
		-- focused.)
		if split then
			return chat
		end
		local s = route.session:peek()
		return if s then s.id else nil
	end
	local function open_link(target: string)
		local other = string.match(target, "^zeron%-chat:([%w%-_]+)$")
		if other then
			route.open_session(other)
			return
		end
		local s = view.session:peek()
		local r = FileLinks.resolve(target, if s then s.cwd else nil)
		local panel: any = boundary.load("@app/screens/panel")
		local home = home_session()
		-- (A split's pane: focused, so its panel, where the file opens, shows.)
		if split and r then
			route.open_session(chat)
		end
		if r and panel and panel.open_file and home then
			(panel.open_file :: any)(home, r.path, route.project:peek(), { line = r.line, column = r.column, end_line = r.end_line })
		end
	end
	-- Text selected in it, annotated on its own chat's draft (a split's
	-- pane: the window's composer takes the keys after; a side chat's, its own;
	-- a subagent, with no composer, on its parent's).
	local function keys_here()
		if split or not props.panel_key or not PanelState.take_keys_now(props.panel_key, slot) then
			Shared.focus:add(1)
		end
	end
	local subagent = tab.kind == "subagent"
	local annotate = Shared.notes(function(): string?
		return if subagent then data.parent or home_session() else chat
	end, nil, {
		side = function(text: string)
			-- (A side chat already sent is no longer unsent: its parent is its row's.)
			local row = if split then nil else lanes.child_row(chat)
			lanes.ask_side(data.parent or lanes.unsent:peek()[chat] or (if row then row.parent else nil) or chat, text)
		end,
		focus = keys_here,
		-- (Into its own draft, at its own composer's caret; a split's pane:
		-- the window composer's, as the chat page's.)
		mention = if split or subagent
			then nil
			else function(q: any)
				Shared.mention_in(chat, { text = q.text, who = q.who or "" }, fields[chat], keys_here)
			end,
	})
	local function make_transcript(): Node
		return T.Transcript(RowProps.add({
		-- (Its own glide, runway and place: never the chat page's.)
		name = `lane.{slot}`,
		-- (A split's pane reads as the chat page: a little wider than its
		-- composer, running under its head and past its foot, faded.)
		measure = function(): number
			return prefs.conversation_width:get() + (if split then 24 else 0)
		end,
		side = if split then 20 else nil,
		fade_lead = if split then 12 else nil,
		over = if tab.kind == "subagent"
			then nil
			else function(t: any): Node
				return MessageRail.MessageRail { transcript = t, gate = split, name = `lane.{slot}` }
			end,
		messages = messages,
		draft = reply,
		offset = offset,
		keep = anchor,
		foot = if tab.kind == "subagent" then nil else room.foot,
		-- Its "Jump to latest" and the pill of the composer at its foot, a
		-- centred pair: its own composer's, or, a split's focused pane, the
		-- window's (screens/chat/shared's `pill_width_of`).
		beside = if tab.kind == "subagent"
			then nil
			else function(): number?
				return Shared.pill_width_of("waiting", if split and focused_now() then "window" else `lane.{slot}`):get()
			end,
		on_jump_width = Shared.pill_teller("jump", `lane.{slot}`),
		compact = function(): boolean
			return prefs.compact_transcript:get()
		end,
		annotate = annotate,
		compaction_wait = function(): any
			local s = view.session:get()
			return if s and s.id == chat then s.compaction_wait else nil
		end,
		find = if split then nil else find.marks(),
		-- (A subagent's liveness is its own doc's: no session of its own.)
		turn_state = if tab.kind == "subagent" then nil else TurnGlue.of(function(): any
			return view.session:get()
		end),
		on_retry = function(message: string)
			store.retry(chat, message)
		end,
		-- (A split's pane: the keys stay with this chat, in the window's composer.)
		on_jump = if split
			then function()
				ui.after(0, function()
					Shared.focus:add(1)
				end)
			end
			else nil,
		pictures = pictures,
		sent_pictures = sent_pictures,
		sent_from = function(): string?
			local s = lane_session()
			return if s then s.id else nil
		end,
		on_open_subagent = function(sub: any)
			local home = home_session()
			if home then
				lanes.open_subagent(home, sub)
			end
		end,
		open_link = open_link,
		chat_of = Shared.chat_of,
		threads = Shared.threads_for(function(): string?
			local s = lane_session()
			return if s then s.id else nil
		end, home_session, function(): any
			return view.session:get()
		end),
		-- (The chat this lane is beside is already on show: to it, as ever.)
		open_sender = function(chat: string, title: string)
			local home = home_session()
			if chat == home then
				open_link(`zeron-chat:{chat}`)
			elseif home then
				lanes.open(home, { kind = "side_chat", id = chat, title = title })
			end
		end,
		link_menu = LinkMenu(open_link, function(): any
			return view.session:peek()
		end),
		-- (A reply's link to one of its files or its agent's skills: a chip.)
		link_chip = LinkChips(function(): any
			return view.session:peek()
		end),
		-- (A split's pane, made again at the place taken up: whether
		-- that was its end.)
		was_at_end = if split then anchor:peek() == nil else nil,
		working = {
			rate = function(): number?
				return view.token_rate:get()
			end,
			tokens = function(): string?
				return view.turn_tokens:peek()
			end,
			says = function(sending: boolean): string?
				return LinkNotice.working_says(sending, sessions.now:get(), view.session:get())
			end,
		},
	}, function(): string?
		return chat
	end, lane_session, function(): any
		return view.session:peek()
	end))
	end
	local transcript = if split
		then ui.presence(function(): string
			return if showable:get() then "shown" else "waiting"
		end, function(k: string): Node
			if k == "waiting" then
				return ui.div {}
			end
			ui.batch(function()
				offset:set(T.PAST_END)
				anchor:set(place_of())
			end)
			settled = false
			-- (Its place put back over its first frames: not said again
			-- until it has settled there.)
			ui.after(400, function()
				settled = true
			end)
			return make_transcript()
		end, { w_full = true, grow = 1, min_h = 0 })
		else make_transcript()
	if tab.kind == "subagent" then
		return ui.column { id = `lane-{slot}`, class = "lane-tab subagent-tab", provide = { session = chat }, grow = 1, w_full = true, min_h = 0, relative = true, transcript, find.Bar() }
	end
	return ui.column {
		id = `lane-{slot}`,
		class = "lane-tab side-chat-tab",
		provide = { session = chat },
		on_visible = function(visible: boolean)
			lanes.on_screen(slot, visible)
		end,
		grow = 1,
		w_full = true,
		min_h = 0,
		-- (A real swap: the fork's word, then its chat.)
		ui.presence(function(): string
			return if kind:get() == "fork" then "fork" else "chat"
		end, function(k: string): Node
			if k == "fork" then
				return forking(chat)
			end
			-- As the chat page: the transcript to the composer's footer line,
			-- running on behind the composer, which floats at the foot.
			local body: { [any]: any } = {
				grow = 1,
				w_full = true,
				min_h = 0,
				relative = true,
				ui.column {
					w_full = true,
					h_full = true,
					min_h = 0,
					pb = Dock.page_pad(),
					ui.show(function(): boolean
						return not split and context_of(chat) ~= nil
					end, ui.text {
						function(): string
							return context_of(chat) or ""
						end,
						class = "sidechat-context",
						shrink = 0,
						px = ui.raw(12, "gpui's sidechat context inset"),
						py = ui.raw(8, "gpui's sidechat context inset"),
						text_size = "$font.size.xs",
						color = "$text.secondary",
						truncate = true,
					}),
					ui.column { w_full = true, grow = 1, min_h = 0, relative = true, transcript, if split then nil else find.Bar() },
				},
				-- (A split's pane, focused: the window's composer stands over
				-- this one, which is kept, unseen and out of Tab's way, rather
				-- than made again as the focus leaves, which would grow it
				-- from nothing each time.)
				ui.div {
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					bottom = 0,
					pointer_events = "none",
					opacity = function(): number
						return if split and focused_now() then 0 else 1
					end,
					hidden = function(): boolean
						return split and focused_now()
					end,
					Dock.Dock { height = dock_h, composer = ui.column { w_full = true, owners(chat, split), composer(chat, view, `lane.{slot}`, focus, open_model, step_effort, data.split == true, if split then focused_now else nil, props.panel_key, slot, tab.placeholder) }, id = `lane-composer-{slot}` },
				},
			}
			if zone then
				for k, v in zone do
					body[k] = v
				end
				table.insert(body, DropOverlay { shown = carrying, id = `lane-drop-{slot}` })
				table.insert(body, DropOverlay { shown = carrying_session, id = `lane-drop-mention-{slot}`, label = ui.t("chat.composer.drop_to_mention") :: any })
				-- A bare Escape that reached it (nothing inside took it): the
				-- setting's "Escape stops the agent", as the chat page's.
				body.on_key = function(e: any): boolean
					if e.key == "Escape" and not e.shift and not e.cmd and not e.alt then
						local s = view.session:peek()
						return store.escape_chat(chat, s ~= nil and s.running == true)
					end
					return false
				end
			end
			return ui.column(body :: any)
		end, { grow = 1, w_full = true, min_h = 0 }),
	}
end)

return M
