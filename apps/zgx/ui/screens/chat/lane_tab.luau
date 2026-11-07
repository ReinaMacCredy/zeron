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
local T = require("@ds/transcript")
local lanes = require("@app/screens/chat/lanes")
local store = require("@app/screens/chat/store")
local prefs = require("@app/screens/settings/prefs")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local LinkNotice = require("@app/screens/chat/link_notice")
local route = require("@app/screens/shell/route")
local Where = require("@app/screens/chat/where")
local Dock = require("@app/screens/chat/dock")
local Pills = require("@app/screens/chat/pills")
local ReplyImages = require("@app/screens/chat/reply_images")
local FileLinks = require("@app/screens/chat/file_links")
local LinkMenu = require("@app/screens/chat/link_menu")
local boundary = require("@app/screens/shell/boundary")
local Quote = require("@ds/quote")
-- (The chat page's places in its chats, which a split's panes share.)
local Shared = require("@app/screens/chat/shared")
local Asks = require("@app/screens/chat/asks")
local Completions = require("@app/screens/chat/completions")
local ZeronCommands = require("@app/screens/chat/zeron_commands")
local AppCommands = require("@app/screens/chat/app_commands")
local Outgoing = require("@app/screens/chat/outgoing")
local PanelState = require("@app/screens/panel/state")

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

function M.title(tab: any): string
	if tab.title and tab.title ~= "" then
		return tab.title
	end
	return locale.s(if tab.kind == "subagent" then "chat.lane_tab.subagent" else "chat.lane_tab.side_chat")
end

function M.icon(tab: any): string
	return if tab.kind == "subagent" then "bot" else "side-chat"
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
local function composer(chat: string, view: any, name: string, split: boolean?, focused: (() -> boolean)?, panel_key: string?, tab_id: string?): Node
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
	-- (A split's pane: its draft's chips, as the chat page keeps them.)
	local carried = ui.use_state({} :: { any })
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
	-- A chip taken off a split pane's draft, wherever the draft is now.
	local function remove_carried(cid: string)
		Shared.remove_cargo_of(chat, cid)
	end
	-- (A split's pane: the options picked in a question its chat asks.)
	local chosen = ui.use_state({} :: { [string]: boolean })
	-- (Words kept from before a side chat's draft lived with the chat's.)
	if not split and draft:peek() ~= "" and Shared.draft_peek(chat).text == "" then
		Shared.set_draft_of(chat, "text", draft:peek())
	end
	-- (Its chat's draft, by its id: its words, chips and a question's
	-- picks, one draft per chat (screens/chat/shared), shown here and
	-- written back as they change, wherever it's kept. Never the focused
	-- chat's: the focus moving changes nothing this shows.)
	ui.effect(function()
		local d = Shared.draft_of(chat)
		if draft:peek() ~= d.text then
			draft:set(d.text)
		end
		carried:set(d.cargo)
		chosen:set(d.chosen)
	end)
	ui.effect(function()
		local text = draft:get()
		if Shared.draft_of(chat).text ~= text then
			Shared.set_draft_of(chat, "text", text)
		end
	end)
	if split then
		ui.effect(function()
			local picked = chosen:get()
			if Shared.draft_of(chat).chosen ~= picked then
				Shared.set_draft_of(chat, "chosen", picked)
			end
		end)
	end
	-- (A split's pane: what its chat asks, and its queue, todos and tasks,
	-- from its own session, as the window's composer shows them.)
	local notice = ui.use_state(nil :: any)
	local shown_notice = ui.computed(function(): any
		local refused = Shared.draft_of(chat).refused
		return notice:get() or (if refused then { tone = "danger", text = refused, dismissable = true } else nil)
	end)
	local asks = if split
		then Asks.bind {
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
		}
		else nil
	local tray = ui.state(`chat_split_tray_{chat}`, nil :: any)
	local session = view.session
	local running = ui.computed(function(): boolean
		local s = session:get()
		return s ~= nil and s.running == true
	end)
	-- (Kept by the chat, outside the view: a pick stays across a switch of
	-- tabs and a reload.)
	local model = ui.state(`chat_lane_model_{chat}`, nil :: string?)
	local effort = ui.state(`chat_lane_effort_{chat}`, nil :: string?)
	local access = ui.state(`chat_lane_access_{chat}`, nil :: string?)
	local project = ui.state(`chat_lane_project_{chat}`, nil :: string?)
	local device = ui.state(`chat_lane_device_{chat}`, nil :: string?)
	local checkout = ui.state(`chat_lane_checkout_{chat}`, nil :: string?)
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
		if now.model ~= was.model or now.effort ~= was.effort or now.access ~= was.access then
			seeded:set(now)
		end
	end)
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
			for _, p in sessions.now:get().projects do
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
		return {
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
		local out: { [string]: any } = { model = m, effort = effort:peek(), access = access:peek() }
		for _, c in store.catalog:peek().models do
			if c.value == m then
				out.harness = c.harness
			end
		end
		return out
	end
	-- Where its first message makes it: nil, its parent's place as it is;
	-- else where the person moved it (`where.plan()`).
	local function place(): { [string]: any }?
		local was = seed:peek()
		if
			was
			and project:peek() == was.project
			and device:peek() == was.device
			and checkout:peek() == was.checkout
			and branch:peek() == was.branch
		then
			return nil
		end
		return where.plan() :: any
	end
	local accounts = if split
		then Shared.model_accounts(function(): string?
			return device:get()
		end)
		else nil
	local queue_pictures = ReplyImages.for_attachments(function(): any
		local s = session:get()
		if not s or s.state == "missing" then
			return nil
		end
		return { id = s.id, device = s.device, cwd = s.cwd, here = sessions.now:get().here }
	end)
	local completions = Completions.bind {
		key = if split then `chat_split_{chat}` else `chat_lane_{chat}`,
		agent = function(): string?
			return (as_made())
		end,
		target = function(): any
			return { chat = chat }
		end,
		in_chat = function(): boolean
			return true
		end,
		attachments = function(): { any }
			local out = {}
			for _, a in carried:get() do
				if a.path then
					table.insert(out, { value = a.path, label = a.name, src = if a.kind == "image" then a.path else nil, file = a.kind ~= "image" })
				end
			end
			return out
		end,
	}
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
	return ui.show(expired, expired_line, ds.Composer {
		field = field,
		value = draft,
		snap = if split then snap else nil,
		cargo = carried,
		-- (Its chips' × under the pointer, as the chat page's.)
		on_remove_cargo = remove_carried,
		on_paste_text = cargo_props.on_paste_text,
		on_open_pasted = cargo_props.on_open_pasted,
		on_attach = cargo_props.on_attach,
		on_files = cargo_props.on_files,
		on_paste_image = cargo_props.on_paste_image,
		-- (ds.Composer's placeholder is a string: read as the tab is made.)
		-- (A split's pane is a chat of its own: the chat page's words.)
		placeholder = locale.s(if split then "chat.composer.placeholder" else "chat.lane_tab.placeholder"),
		running = running,
		access = access,
		model = model,
		effort = effort,
		notice = shown_notice,
		on_dismiss_notice = function()
			notice:set(nil)
			Shared.refuse_in(chat, nil)
		end,
		harnesses = function(): { any }
			return store.catalog:get().harnesses
		end,
		models = function(): { any }
			return store.catalog:get().models
		end,
		-- (A split's pane is a chat of its own; a side chat has none, as Zeron's.)
		accounts = if accounts then accounts.accounts else nil,
		on_account = if accounts then accounts.on_account else nil,
		on_accounts_open = if accounts then accounts.on_accounts_open else nil,
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
		working = function(): boolean
			local s: any = session:get()
			if s and s.in_flight ~= nil then
				return s.in_flight
			end
			return running:get()
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
		-- As the window's composer sends (screens/chat/composer): `false`, as it
		-- has dealt with the draft itself.
		on_send = function(text: string): boolean
			local say = function(n: any)
				notice:set(n)
			end
			local side = AppCommands.side(text)
			if side then
				draft:set("")
				lanes.open_side(chat, side, route.current:peek().session)
				return false
			end
			local focus = AppCommands.compact(text)
			if focus then
				draft:set("")
				AppCommands.run_compact(chat, session:peek(), running:peek(), focus, function(key: string)
					say({ tone = "danger", text = locale.s(key), dismissable = true })
				end, function(why: string)
					say({ tone = "danger", text = why, dismissable = true })
				end)
				return false
			end
			local trimmed = string.match(text, "^%s*(.-)%s*$") or ""
			local zeron = completions.command_of(trimmed)
			if zeron then
				draft:set("")
				ZeronCommands.run(zeron, chat)
				return false
			end
			-- (Nowhere it can start, a project gone: the line says why.)
			local is_unsent = lanes.unsent:peek()[chat] ~= nil
			if is_unsent and where.blocked:peek() then
				say({ tone = "danger", text = where.blocked:peek(), dismissable = true })
				return false
			end
			local s = session:peek()
			local goes_to = if s and s.device then s.device else device:peek()
			if goes_to ~= nil and goes_to ~= sessions.now:peek().here then
				local too_large = Shared.linked_elsewhere(carried:peek())
				if too_large then
					say({ tone = "danger", text = too_large, dismissable = true })
					return false
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
			local mid = store.new_id()
			local req = picks()
			req.message_id = mid
			req.attachments = parts.attachments
			req.appshots = parts.appshots
			req.restores = true
			local sent: any = nil
			T.send(function()
				local here = view.messages:index_of("id", mid) ~= nil
				if not here then
					-- (On its way from this moment: the line under it says so.)
					view.messages:push { id = mid, role = "user", text = parts.words, attachments = parts.shown, waiting = "sending" }
				end
			end, mid, name)
			-- Its first message makes it (New side chat), then sends.
			if is_unsent then
				sent = lanes.send_first(chat, parts.text, req, place())
			else
				req.session, req.text = chat, parts.text
				sent = store.send(req :: any)
			end
			sent:after(function() end, function(err: any)
				Shared.restore(parts.text, cargo)
				local why = if type(err) == "table" and err.message then tostring(err.message) else nil
				say({
					tone = "danger",
					text = if is_unsent and why then why else locale.s("chat.lane_tab.didnt_send"),
					dismissable = true,
				})
			end)
			return false
		end,
		on_stop = function()
			store.interrupt(chat)
		end,
		-- (Hidden while focused, its draft shown by the window's composer: only that
		-- one's menus open, or a trigger typed there opens a second here.)
		mentions = completions.mentions,
		on_mention = completions.on_mention,
		mention_enabled = if split and focused
			then function(trigger: string): boolean
				return not focused() and completions.mention_enabled(trigger)
			end
			else completions.mention_enabled,
		mention_empty = completions.mention_empty,
		mention_failed = completions.mention_failed,
		mention_token = completions.mention_token,
		-- (A split's pane, its chat's own.)
		question = if asks then asks.question else nil,
		chosen = if asks then asks.chosen else nil,
		on_pick = if asks then asks.on_pick else nil,
		on_answer = if asks then asks.on_answer else nil,
		on_close_question = if asks then asks.on_close_question else nil,
		on_back = if asks then asks.on_back else nil,
		approval = if asks then asks.approval else nil,
		can_always = if asks then asks.can_always else nil,
		on_approve = if asks then asks.on_approve else nil,
		on_deny = if asks then asks.on_deny else nil,
		queue = if asks then asks.queue else nil,
		can_steer = if asks then asks.can_steer else nil,
		on_queue_send = if asks then asks.on_queue_send else nil,
		on_queue_steer_all = if asks then asks.on_queue_steer_all else nil,
		queue_pictures = if asks then queue_pictures else nil,
		on_dismiss_todos = if asks then asks.on_dismiss_todos else nil,
		on_queue_remove = if asks then asks.on_queue_remove else nil,
		on_queue_reorder = if asks then asks.on_queue_reorder else nil,
		todos = if asks then asks.todos else nil,
		pills = if asks
			then Pills.Row {
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
			}
			else nil,
		tray = if split then tray else nil,
	})
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

-- A lane's composer's height as measured, by its slot.
function M.dock_height(slot: string): State<number>
	return ui.state(`chat_lane_dock_h_{slot}`, Dock.REST)
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
	local room = Dock.room(function(): number
		return if split and focused_now() then Shared.dock_h:get() else dock_h:get()
	end)
	local function lane_session(): any
		local s = view.session:get()
		return if s and s.state ~= "missing" then s else nil
	end
	local pictures = ReplyImages.for_session(lane_session)
	local sent_pictures = ReplyImages.for_attachments(function(): any
		local s = lane_session()
		if not s then
			return nil
		end
		return { id = s.id, device = s.device, cwd = s.cwd, here = sessions.now:get().here }
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
		if split and r and not r.outside then
			route.open_session(chat)
		end
		if r and not r.outside and panel and panel.open_file and home then
			(panel.open_file :: any)(home, r.path, route.project:peek(), { line = r.line, column = r.column, end_line = r.end_line })
		end
	end
	-- A passage quoted (its menu's Mention, or Attach): into its own draft.
	local draft = ui.state(`chat_lane_draft_{chat}`, "")
	local function quote(q: any)
		-- (A split's pane: into its chat's own draft, which the window's
		-- composer types into once it's focused.)
		if split then
			route.open_session(chat)
			ui.after(0, function()
				Shared.mention(q)
			end)
			return
		end
		draft:set(Quote.add(draft:peek(), Quote.line(q)))
	end
	local function make_transcript(): Node
		return T.Transcript {
		-- (Its own glide, runway and place: never the chat page's.)
		name = `lane.{slot}`,
		-- (A split's pane reads as the chat page: a little wider than its
		-- composer, running under its head and past its foot, faded.)
		measure = function(): number
			return prefs.conversation_width:get() + (if split then 24 else 0)
		end,
		side = if split then 20 else nil,
		fade_lead = if split then 12 else nil,
		messages = messages,
		draft = reply,
		offset = offset,
		keep = anchor,
		foot = if tab.kind == "subagent" then nil else room.foot,
		compact = function(): boolean
			return prefs.compact_transcript:get()
		end,
		on_mention = quote,
		on_attach = quote,
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
		link_menu = LinkMenu(open_link, function(): any
			return view.session:peek()
		end),
		-- (A split's pane, made again at the place taken up: whether
		-- that was its end.)
		was_at_end = if split then anchor:peek() == nil else nil,
		working = {
			rate = function(): number?
				return view.token_rate:get()
			end,
			says = function(sending: boolean): string?
				return LinkNotice.working_says(sending, sessions.now:get(), view.session:get())
			end,
		},
	}
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
		return ui.column { id = `lane-{slot}`, class = "lane-tab subagent-tab", provide = { session = chat }, grow = 1, w_full = true, min_h = 0, transcript }
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
			return ui.column {
				grow = 1,
				w_full = true,
				min_h = 0,
				relative = true,
				ui.column {
					w_full = true,
					h_full = true,
					min_h = 0,
					pb = Dock.page_pad(),
					ui.column { w_full = true, grow = 1, min_h = 0, relative = true, transcript },
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
					Dock.Dock { height = dock_h, composer = composer(chat, view, `lane.{slot}`, data.split == true, if split then focused_now else nil, props.panel_key, slot), id = `lane-composer-{slot}` },
				},
			}
		end, { grow = 1, w_full = true, min_h = 0 }),
	}
end)

return M
