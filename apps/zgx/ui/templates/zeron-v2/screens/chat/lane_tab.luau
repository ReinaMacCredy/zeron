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
local model_switch = require("@app/screens/chat/model_switch")
local composer_profiles = require("@app/screens/chat/composer_profiles")
local prefs = require("@app/screens/settings/prefs")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local LinkNotice = require("@app/screens/chat/link_notice")
local route = require("@app/screens/shell/route")
local Where = require("@app/screens/chat/where")
local Dock = require("@app/screens/chat/dock")
local ReplyImages = require("@app/screens/chat/reply_images")
local FileLinks = require("@app/screens/chat/file_links")
local LinkMenu = require("@app/screens/chat/link_menu")
local Shared = require("@app/screens/chat/shared")
local Completions = require("@app/screens/chat/completions")
local AppCommands = require("@app/screens/chat/app_commands")
local Outgoing = require("@app/screens/chat/outgoing")
local PanelState = require("@app/screens/panel/state")
local ZeronCommands = require("@app/screens/chat/zeron_commands")
local boundary = require("@app/screens/shell/boundary")
local Quote = require("@ds/quote")

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
local function composer(chat: string, view: any, name: string, panel_key: string?, tab_id: string?): Node
	-- (Its draft kept by the chat, as its picks: a switch of tabs or a
	-- reload finds it as it was; a passage quoted from its transcript lands
	-- in it.)
	local draft = ui.state(`chat_lane_draft_{chat}`, "")
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
	local carried = ui.use_state({} :: { any })
	if draft:peek() ~= "" and Shared.draft_peek(chat).text == "" then
		Shared.set_draft_of(chat, "text", draft:peek())
	end
	ui.effect(function()
		local d = Shared.draft_of(chat)
		if draft:peek() ~= d.text then
			draft:set(d.text)
		end
		carried:set(d.cargo)
	end)
	ui.effect(function()
		local text = draft:get()
		if Shared.draft_of(chat).text ~= text then
			Shared.set_draft_of(chat, "text", text)
		end
	end)
	local session = view.session
	local notice = ui.use_state(nil :: any)
	local shown_notice = ui.computed(function(): any
		local refused = Shared.draft_of(chat).refused
		return notice:get() or (if refused then { tone = "danger", text = refused, dismissable = true } else nil)
	end)
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
		if now.model ~= was.model or now.effort ~= was.effort or now.access ~= was.access or now.fast ~= was.fast or not Shared.same_pick(now.options, was.options) then
			seeded:set(now)
		end
	end)
	-- A pick on a chat that runs lands on it at once, as the chat page's does
	-- (a side chat's first message still carries its own).
	local landing: model_switch.Ask? = nil
	ui.effect(function()
		local picked: { [string]: any } = { model = model:get(), effort = effort:get(), fast = fast:get(), options = options:get() }
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
	local open_picker = ui.state(`chat_lane_model_open_{chat}`, 0)
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
			open_picker:set(open_picker:peek() + 1)
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
		local out: { [string]: any } = { model = m, effort = effort:peek(), fast = fast:peek(), options = options:peek(), access = access:peek() }
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
	local completions = Completions.bind {
		key = `chat_lane_{chat}`,
		agent = function(): string?
			return (as_made())
		end,
		target = function(): any
			return { chat = chat }
		end,
		in_chat = function(): boolean
			return true
		end,
	}
	-- A side chat past its day: nothing more goes to it (Zeron's composer.rs).
	local function expired(): boolean
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
	return ui.column { w_full = true, switch_dialog, profile_dialog, ui.show(expired, expired_line, ds.Composer {
		profiles = profiles,
		open_profiles = open_profiles,
		field = field,
		cargo = carried,
		on_remove_cargo = function(cid: string)
			Shared.remove_cargo_of(chat, cid)
		end,
		on_paste_text = cargo_props.on_paste_text,
		on_open_pasted = cargo_props.on_open_pasted,
		on_attach = cargo_props.on_attach,
		on_files = cargo_props.on_files,
		on_paste_image = cargo_props.on_paste_image,
		mentions = completions.mentions,
		on_mention = completions.on_mention,
		mention_enabled = completions.mention_enabled,
		mention_empty = completions.mention_empty,
		mention_failed = completions.mention_failed,
		mention_token = completions.mention_token,
		value = draft,
		-- (ds.Composer's placeholder is a string: read as the tab is made.)
		placeholder = locale.s("chat.lane_tab.placeholder"),
		running = running,
		access = access,
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
		open_model = open_picker,
		context_tokens = function(): number?
			local s: any = session:get()
			return if s then s.context_tokens else nil
		end,
		notice = shown_notice,
		on_dismiss_notice = function()
			notice:set(nil)
			Shared.refuse_in(chat, nil)
		end,
		harnesses = function(): { any }
			local c: any = store.catalog:get()
			return c.tabs or c.harnesses
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
		footer_status = false,
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
				req.profile = composer_profiles.find(lane_profile:peek()) and lane_profile:peek()
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
	ui.effect(function()
		if kind:get() == "chat" then
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
	local offset = ui.state(`chat_lane_offset_{slot}`, T.PAST_END)
	local anchor = ui.state(`chat_lane_anchor_{slot}`, nil :: any)
	local dock_h = ui.state(`chat_lane_dock_h_{slot}`, Dock.REST)
	local room = Dock.room(function(): number
		return dock_h:get()
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
		if r and not r.outside and panel and panel.open_file and home then
			(panel.open_file :: any)(home, r.path, route.project:peek(), { line = r.line, column = r.column, end_line = r.end_line })
		end
	end
	-- A passage quoted (its menu's Mention, or Attach): into its own draft.
	local draft = ui.state(`chat_lane_draft_{chat}`, "")
	local function quote(q: any)
		draft:set(Quote.add(draft:peek(), Quote.line(q)))
	end
	local transcript = T.Transcript {
		-- (Its own glide, runway and place: never the chat page's.)
		name = `lane.{slot}`,
		measure = function(): number
			return prefs.conversation_width:get()
		end,
		messages = view.messages,
		draft = view.draft,
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
		working = {
			rate = function(): number?
				return view.token_rate:get()
			end,
			says = function(sending: boolean): string?
				return LinkNotice.working_says(sending, sessions.now:get(), view.session:get())
			end,
		},
	}
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
				Dock.Dock { height = dock_h, composer = composer(chat, view, `lane.{slot}`, props.panel_key, slot), id = `lane-composer-{slot}` },
			}
		end, { grow = 1, w_full = true, min_h = 0 }),
	}
end)

return M
