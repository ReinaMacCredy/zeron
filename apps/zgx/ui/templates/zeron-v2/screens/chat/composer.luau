-- The window's one composer, wired: it travels between a new chat (lifted
-- to a little over the page's middle, opened out, its menus below) and a
-- session (docked at the foot, its menus above), on one critically damped
-- spring, as the gallery's App shell has it (Zeron's composer_dock.rs). It
-- is made once and follows where the shell is: the draft and what it
-- carries are each chat's own (screens/chat/shared), its model, effort
-- and access the session's (changed there, they change the session) or,
-- on a new chat, those the last new chat used.
--
-- In a session it sends to that session (queued while a turn runs, which
-- Stop ends), and shows what the agent asks (an approval, a question) in
-- the pill, the queue, the plan and what runs behind it in its trays. On a
-- new chat it starts a chat in the route's project and opens it.
--
-- `Composer()` is placed once over the page pane (absolute, filling it):
-- it lays itself out.

local ds = require("@ds")
local motion = require("@ds/motion")
local T = require("@ds/transcript")
local prefs = require("@app/screens/settings/prefs")
local store = require("@app/screens/chat/store")
local model_switch = require("@app/screens/chat/model_switch")
local composer_profiles = require("@app/screens/chat/composer_profiles")
local route = require("@app/screens/shell/route")
local cloud = require("@account/cloud")
local shared = require("@app/screens/chat/shared")
local sessions = require("@account/sessions")
local LinkNotice = require("@app/screens/chat/link_notice")
local Compaction = require("@ds/compaction")
local Where = require("@app/screens/chat/where")
local DropOverlay = require("@ds/drop_overlay")
local Invocations = require("@app/screens/chat/invocations")
local Completions = require("@app/screens/chat/completions")
local ReplyImages = require("@app/screens/chat/reply_images")
local ZeronCommands = require("@app/screens/chat/zeron_commands")
-- (Loaded with the chat page: captures land on its composer.)

local SessionPlace = require("@app/screens/chat/session_place")
local Lanes = require("@app/screens/chat/lanes")
local Pills = require("@app/screens/chat/pills")
local Review = require("@app/screens/chat/review")
local AppCommands = require("@app/screens/chat/app_commands")
local Outgoing = require("@app/screens/chat/outgoing")
local Asks = require("@app/screens/chat/asks")

-- The title bar's height (Zeron's): a new chat centres under it.
local TITLE_H = 38
-- gpui's hero headline: its line, and its gap over the 28 of chips above the pill.
local HEADLINE_LINE, HEADLINE_GAP = 38, 18
-- What Resume says (crates/proto/src/agent.rs `RESUME_PROMPT`).
-- The side chat's chord: the open composer's, as the panel's commands are its tab's.
local side_chat_opener: (() -> ())? = nil
local function side_chat_key(): string
	local ok, keys = pcall(require, "@app/screens/shell/keys")
	return if ok and type(keys) == "table" then (keys :: any).key("chat.side_chat")() else "cmd+;"
end
ui.command {
	id = "chat.side_chat",
	title = ui.t("ds.composer.side_chat"):get(),
	menu = "View",
	key = side_chat_key :: any,
	run = function(): boolean
		if side_chat_opener then
			side_chat_opener()
			return true
		end
		return false
	end,
}

local RESUME_PROMPT = "Resume without asking to continue."
ds.toast.action("chat.add_credits", function()
	route.settings("billing")
end)
-- Zeron's dock spring (ω ≈ 27).
local DOCK: SpringOptions = { stiffness = 730, damping = 54 }
-- Its gap to the window's foot, docked. Its width, rising and docked: the
-- transcript's column (Settings' "Conversation width"), as Zeron's
-- composer lines up with the conversation over it (its edges on the
-- column's).
local FOOT = require("@app/screens/chat/dock").GAP
store.dismiss_cards = Pills.dismiss

-- What a new chat starts with: what the last one used.
local defaults = ui.persist("chat_new_defaults", {
	model = nil :: string?,
	effort = nil :: string?,
	fast = false,
	access = "full" :: string?,
	plan = false,
	checkout = "worktree" :: string?,
	device = nil :: string?,
})

local FIELDS = { "model", "effort", "fast", "options", "access", "plan" }
-- Past ds/dialog's leaving and its modal's going (which gives the keys back
-- to what had them), the draft takes them.
local DIALOG_OUT = 350
-- Whether two choices are the same: the options' picks (tables) by their
-- entries, none and an empty table alike; the rest by value.
local function equal(a: any, b: any): boolean
	if type(a) == "table" or type(b) == "table" then
		local x: { [string]: any } = if type(a) == "table" then a else {}
		local y: { [string]: any } = if type(b) == "table" then b else {}
		for k, v in x do
			if y[k] ~= v then
				return false
			end
		end
		for k, v in y do
			if x[k] ~= v then
				return false
			end
		end
		return true
	end
	return a == b
end
-- The tray open behind the composer (its todos, tasks or queue), kept
-- across a reload.
local TRAY = ui.state("chat_tray", nil :: ("todos" | "tasks" | "queue")?)

local function Composer(): Node
	-- The session shown, or nil on a new chat.
	local session_id = ui.computed(function(): string?
		local now = route.current:get()
		return if now.name == "session" then now.session else nil
	end)
	local function new_chat(): boolean
		return session_id:get() == nil
	end
	-- (Set as it sends from a new chat: the move to the session that
	-- follows slides, as the prompt glides; see `place`.)
	local slide_next = false
	-- Its form (a new chat's, opened out; a session's): the place it shows
	-- at's, changing with its place (see `place`).
	local form_new = ui.state("chat_composer_form_new", session_id:peek() == nil)
	form_new:set(session_id:peek() == nil)
	-- Moved as it switches between them (ds.Composer's `snap`): its form and
	-- size there at once, not eased.
	local snap = ui.state("chat_composer_snap", 0)
	-- Whether its model picker is open: the composer is held where it is
	-- while it is (the picker hangs from its chip; a new chat's composer,
	-- re-centring as its draft grows or shrinks, moved the chip and the
	-- picker with it on every frame of its glide), and goes to its place
	-- as the picker closes.
	local model_open = ui.state("chat_model_open", false)
	local function shown_new(): boolean
		return form_new:get()
	end
	-- The open session, once it is the one shown.
	local function open(): store.Session?
		local s = store.session:get()
		local id = session_id:get()
		if s and id and s.id == id and s.state == "ready" then
			return s
		end
		return nil
	end

	-- The choices, bound to the composer, following the session (or a new
	-- chat's defaults) and changing it.
	local model = ui.state("chat_model", nil :: string?)
	local effort = ui.state("chat_effort", nil :: string?)
	local fast = ui.state("chat_fast", false)
	-- The choices made of the model's options (Devin Fusion's lead and
	-- sidekick), by option id.
	local options = ui.state("chat_options", {} :: { [string]: string })
	local access = ui.state("chat_access", "full" :: string?)
	local plan = ui.state("chat_plan", false)
	local device = ui.state("chat_device", nil :: string?)
	local checkout = ui.state("chat_checkout", "worktree" :: string?)
	local branch = ui.state("chat_branch", nil :: string?)
	local project = ui.state("chat_project", nil :: string?)
	-- A new chat's agent profile (its id, sent with its first message).
	local draft_profile = ui.state("chat_profile", nil :: string?)
	local locals: { [string]: State<any> } =
		{ model = model, effort = effort, fast = fast, options = options, access = access, plan = plan }

	-- A pick answered yes in the switch dialog: its words for the divider.
	local landing: model_switch.Ask? = nil
	-- In: what the session says (or the defaults, arriving on a new chat).
	-- `applied`: what the choices were set to from the session, so only a
	-- choice made here goes out to it (a session whose config isn't known,
	-- or has no model, is never handed this composer's).
	local applied: { [string]: any } = {}
	-- The chat it last followed ("?" before any): kept, so a reload isn't
	-- an arrival (a new chat's choices put back to its defaults).
	local was = ui.state("chat_composer_was", "?" :: string?)
	ui.effect(function()
		local id = session_id:get()
		local s = open()
		ui.batch(function()
			if id == nil then
				if was:peek() ~= nil then
					local d = defaults:peek()
					for _, k in FIELDS do
						local v = (d :: any)[k]
						if v ~= nil and not equal(locals[k]:peek(), v) then
							locals[k]:set(v)
						end
					end
					checkout:set(d.checkout)
					-- (A device asked of this new chat, a device's "+" in the
					-- sidebar, over the one last used.)
					device:set(route.take_device() or d.device)
					project:set(route.current:peek().project)
				end
			elseif s then
				local resolved = shared.resolve_model(s, store.catalog:get())
				for _, k in FIELDS do
					local v = (s :: any)[k]
					-- (None found: none shown, never the chat before's.)
					local own = (k == "model" or k == "effort") and resolved.model == nil
					if k == "model" or k == "effort" then
						v = resolved[k]
					end
					if (v ~= nil or own) and not equal(locals[k]:peek(), v) then
						locals[k]:set(v)
					end
					applied[k] = if v ~= nil or own then v else locals[k]:peek()
				end
				if s.device and device:peek() ~= s.device then
					device:set(s.device)
				end
				if s.project and project:peek() ~= s.project then
					project:set(s.project)
				end
			end
		end)
		if was:peek() ~= id then
			was:set(id)
		end
	end)
	-- A new chat's project follows the route's: as the route comes home, or
	-- its project changes there. (Kept, so a reload, the route as it was,
	-- leaves a project picked in the composer alone.)
	local route_was = ui.state("chat_composer_route", "")
	ui.effect(function()
		local now = route.current:get()
		local key = `{now.name}:{now.project or ""}`
		if route_was:peek() == key then
			return
		end
		route_was:set(key)
		if now.name == "home" and project:peek() ~= now.project then
			project:set(now.project)
		end
	end)
	-- A device asked of a new chat already showing (a device's "+" in the
	-- sidebar, the composer here): picked, with the route's project. (One
	-- asked on the way here from a session is the arrival's, above.)
	ui.effect(function()
		local pick = route.device_pick:get()
		if pick == nil or session_id:peek() ~= nil or was:peek() ~= nil then
			return
		end
		local d = route.take_device()
		ui.batch(function()
			-- (None: the device's own pick, screens/chat/where.)
			local p = route.current:peek().project
			if project:peek() ~= p then
				project:set(p)
			end
			if d and device:peek() ~= d then
				device:set(d)
			end
		end)
	end)
	-- Out: a choice made here changes the session, or is what the next new
	-- chat starts with.
	ui.effect(function()
		local picked: { [string]: any } = {}
		for _, k in FIELDS do
			picked[k] = locals[k]:get()
		end
		local where: { [string]: any } = { checkout = checkout:get(), device = device:get() }
		local id = session_id:peek()
		if id == nil then
			local d = table.clone(defaults:peek()) :: any
			local changed = false
			for k, v in picked do
				if not equal(d[k], v) then
					d[k] = v
					changed = true
				end
			end
			for k, v in where do
				if d[k] ~= v then
					d[k] = v
					changed = true
				end
			end
			if changed then
				defaults:set(d)
			end
			return
		end
		local s = store.session:peek()
		if not s or s.id ~= id or s.state ~= "ready" then
			return
		end
		local patch: { [string]: any } = {}
		local any_change = false
		for k, v in picked do
			if v ~= nil and not equal(applied[k], v) and not equal((s :: any)[k], v) then
				patch[k] = v
				applied[k] = v
				any_change = true
			end
		end
		if any_change then
			-- A model of another agent takes its agent with it.
			if patch.model then
				for _, m in store.catalog:peek().models do
					if m.value == patch.model then
						patch.harness = m.harness
					end
				end
			end
			-- The switch's divider, when it was asked.
			if landing then
				patch.note = landing.note
				patch.note_hint = landing.note_hint
				landing = nil
			end
			store.configure(id, patch)
		end
	end)

	-- The device it runs on (the session's, or where a new chat is made):
	-- the model picker lists that device's agents and models, as the
	-- shipping app's does. (nil: this one, or not known yet.)
	local catalog_device = ui.state("chat_catalog_device", nil :: string?)
	ui.effect(function()
		local d = device:get()
		if d == nil and session_id:get() == nil then
			local p = route.project:get()
			d = if p then p.device else nil
		end
		if d ~= catalog_device:peek() then
			catalog_device:set(d)
			store.catalog_on(d)
		end
	end)
	-- A new chat moved onto Cloud runs only the agents Cloud has accounts
	-- for: a model it doesn't run gives way to Cloud's first, as the
	-- shipping app drops one on the switch.
	ui.effect(function()
		local cat = store.catalog:get()
		if session_id:get() ~= nil or device:get() ~= Where.CLOUD or (cat :: any).device ~= Where.CLOUD then
			return
		end
		local picked = model:peek()
		for _, m in cat.models do
			if m.value == picked then
				return
			end
		end
		local first = cat.models[1]
		if first then
			model:set(first.value)
			if first.effort then
				effort:set(first.effort)
			end
		end
	end)
	-- Where a new chat works (screens/chat/where): the devices, the chosen
	-- one's projects, its refs, kept in step and remembered; how it's sent.
	local where = Where.bind {
		device = device,
		project = project,
		checkout = checkout,
		branch = branch,
		active = new_chat,
	}

	local running = ui.computed(function(): boolean
		local s = open()
		return s ~= nil and s.running == true
	end)
	-- What the agent asks, and which of a question's options are chosen
	-- (fresh for each question); its queue, todos and tasks. (The same as
	-- a split pane's composer shows them: screens/chat/asks.)
	local own_notice = ui.state("chat_own_notice", nil :: any)
	local saving_row = ui.state("chat_queue_saving", nil :: string?)
	local asks = Asks.bind {
		id = function(): string?
			return session_id:peek()
		end,
		session = open,
		chosen = shared.chosen,
		-- (The request the options were picked for: kept, so a reload keeps
		-- them.)
		asked = ui.state("chat_asked_by", nil :: any),
		notice = function(n: any)
			own_notice:set(n)
		end,
		saving = function(): string?
			return saving_row:get()
		end,
	}
	local ask, queue = asks.ask, asks.queue
	-- A line above it: one of its own (a send that failed), else the
	-- session's.
	-- Its own goes as the chat changes (kept: a reload isn't a change).
	local notice_for = ui.state("chat_notice_for", "?")
	ui.effect(function()
		local id = session_id:get() or ""
		if notice_for:peek() ~= id then
			notice_for:set(id)
			own_notice:set(nil)
		end
	end)
	local notice = ui.computed(function(): any
		local mine = own_notice:get()
		local refused = shared.refused:get()
		if refused and not mine then
			return { tone = "danger", text = refused, dismissable = true }
		end
		if mine then
			-- (Its words kept as a catalogue key, said in the language shown.)
			if mine.key then
				local said = table.clone(mine)
				said.text = ui.t(mine.key):get()
				return said
			end
			return mine
		end
		-- A new chat that can't start where it's set to: why.
		local why = where.blocked:get()
		if why then
			return { tone = "warning", text = why }
		end
		local s = open()
		if s and s.notice then
			if s.notice.refused then
				local said = table.clone(s.notice)
				said.text = ui.t("chat.composer.didnt_send_why", { reason = s.notice.refused }):get()
				return said
			end
			return s.notice
		end
		-- The connection: this device's, then the chat's host.
		local now = sessions.now:get()
		local host_unreachable, host = LinkNotice.host_of(now, s)
		return LinkNotice.of {
			connection = now.connection,
			host_unreachable = host_unreachable,
			host = host,
			intent = string.find(shared.text:get(), "%S") ~= nil or #shared.cargo:get() > 0 or #queue:get() > 0,
		}
	end)

	-- The models looked up once: not again on each reload after a lookup
	-- that failed (the composer made again).
	local looked_up = ui.state("chat_catalog_asked", false)
	ui.on_mount(function()
		if store.catalog:peek().state ~= "ready" and not looked_up:peek() then
			looked_up:set(true)
			store.load_catalog()
		end
	end)
	ui.effect(function()
		local want = shared.want_agent:get()
		local c = store.catalog:get()
		if want == nil or session_id:get() ~= nil or c.state ~= "ready" then
			return
		end
		shared.want_agent:set(nil)
		local now = nil
		for _, m in c.models do
			if m.value == model:peek() then
				now = m.harness
			end
		end
		if now == want then
			return
		end
		for _, m in c.models do
			if m.harness == want then
				model:set(m.value)
				effort:set(m.effort)
				return
			end
		end
	end)
	-- A new chat with no model yet: the catalog's first. (Never a session's:
	-- its model is its own.)
	ui.effect(function()
		local c = store.catalog:get()
		if session_id:peek() == nil and c.state == "ready" and model:peek() == nil and #c.models > 0 then
			model:set(c.models[1].value)
			effort:set(c.models[1].effort)
		end
	end)

	-- What a sent message shows of what it carries.
	-- What `@` offers: the files of where it works (the session's folder,
	-- or a new chat's project), searched as the mention is typed. (`/` and
	-- `$` offer nothing yet.)
	-- The agent the chat runs (its model's): whose completion settings the
	-- composer follows.
	local function agent(): string?
		local chosen = model:get()
		for _, m in store.catalog:get().models do
			if m.value == chosen then
				return m.harness
			end
		end
		return nil
	end
	-- What each trigger lists, as the agent's settings have it: with
	-- "Separate / commands" off, `/` lists the skills after the commands
	-- (Zeron's composer.rs `completion_trigger`).
	-- Where the agent works, for its commands and skills: the session's
	-- folder (on its device), or a new chat's project.
	local function invocation_target(): Invocations.Target?
		local id = session_id:get()
		if id then
			local s = store.session:get()
			return { chat = id, device = if s and s.id == id then s.device else nil }
		end
		local p = route.project:get()
		if not p then
			return nil
		end
		local target: Invocations.Target = { space = p.id :: string, device = p.device :: string? }
		return target
	end
	local queue_pictures = ReplyImages.for_attachments(function(): any
		local s = open()
		if not s then
			return nil
		end
		return { id = s.id, device = s.device, cwd = s.cwd, here = sessions.now:get().here }
	end)
	local completions = Completions.bind {
		key = "chat",
		agent = agent,
		target = invocation_target,
		in_chat = function(): boolean
			return session_id:get() ~= nil
		end,
		attachments = function(): { any }
			local out = {}
			for _, a in shared.cargo:get() do
				if a.path then
					table.insert(out, { value = a.path, label = a.name, src = if a.kind == "image" then a.path else nil, file = a.kind ~= "image" })
				end
			end
			return out
		end,
	}

	-- A queued message being edited in the composer: which, and the draft
	-- it set aside (put back when the edit ends).
	local editing = ui.state("chat_editing", nil :: { item: string, aside: string }?)
	local function end_edit(action: string)
		local e = editing:peek()
		local id = session_id:peek()
		if not e or not id then
			return
		end
		local words = shared.text:peek()
		ui.batch(function()
			editing:set(nil)
			shared.text:set(e.aside)
		end)
		if action == "commit" then
			saving_row:set(e.item)
		end
		store.queue_finish(id, e.item, action, if action == "commit" then words else nil):after(function()
			saving_row:set(nil)
		end, function(err: any)
			saving_row:set(nil)
			local why = err and err.message
			own_notice:set {
				tone = "danger",
				key = if why == "conflict" then "chat.composer.edit_conflict" else "chat.composer.edit_not_saved",
				dismissable = true,
			}
		end)
	end
	-- Moving to another chat: an edit under way is left as it was (its
	-- displaced draft back in place), then the draft shown is put away
	-- under the chat left and the next chat's own comes out (shared.show),
	-- as Zeron's composer swaps drafts on navigation.
	-- (Only a real move: a reload, the same chat shown, keeps the edit.)
	ui.effect(function()
		local key = session_id:get() or ""
		if shared.showing() == key then
			return
		end
		if editing:peek() then
			end_edit("cancel")
		end
		shared.show(key)
	end)

	-- A side chat of this one: a bare ask opens one that's open or lives,
	-- else a new one; with words, always a new one, sent them.
	local function open_side_chat(prompt: string?)
		local id = session_id:peek()
		if not id then
			own_notice:set { tone = "danger", key = "chat.composer.sidechat_needs_chat", dismissable = true }
			return
		end
		Lanes.open_side(id, prompt)
	end
	side_chat_opener = function()
		open_side_chat(nil)
	end
	-- "Keep full history": the resume card stays away for this quiet
	-- spell of this chat (gpui's `keep_full_history`), back once it's used.
	local resume_kept = ui.state("chat_resume_kept", nil :: { id: string, at: string }?)
	local function resume_offer(): any
		local s: any = open()
		local o = if s then s.resume else nil
		local k = resume_kept:get()
		if not o or (k and k.id == s.id and k.at == o.at) then
			return nil
		end
		return o
	end
	local function compact(focus: string)
		AppCommands.run_compact(session_id:peek(), open(), running:peek(), focus, function(key: string)
			own_notice:set { tone = "danger", key = key, dismissable = true }
		end, function(why: string)
			own_notice:set { tone = "danger", text = why, dismissable = true }
		end)
	end
	-- Why a send failed, in words: the host's word for a file, else the
	-- generic line.
	local function failure_notice(e: any)
		local why = if type(e) == "table" then e.message else e
		local keys: { [string]: string } = {
			too_large = "chat.composer.too_large",
			too_large_remote = "chat.composer.too_large_remote",
			unreadable = "chat.composer.unreadable",
			not_file = "chat.composer.unreadable",
			offline = "chat.composer.engine_not_connected",
		}
		own_notice:set { tone = if why == "offline" then "warning" else "danger", key = keys[why] or "chat.composer.didnt_send", dismissable = true }
	end
	-- A chat on Cloud, or a new one going there, with no credits left.
	local function out_of_credits(): boolean
		local c = cloud.now:get().status
		if not (c and c.credits and c.credits <= 0 and cloud.on()) then
			return false
		end
		local s = open()
		if session_id:get() then
			return s ~= nil and s.device ~= nil and s.device == cloud.device()
		end
		return where.plan().cloud ~= nil
	end
	-- Whatever it does with the draft it does here: `false`, so ds.Composer
	-- leaves it as it stands. A send that doesn't go keeps its words and chips.
	local function send(text: string, opts: { inverted: boolean?, resume: boolean? }?): boolean
		-- Editing a queued message: Send saves it, and the draft set aside comes back.
		if editing:peek() then
			end_edit("commit")
			return false
		end
		if out_of_credits() then
			ds.toast.show {
				title = ui.t("chat.composer.credits_out"):get(),
				tone = "danger",
				action = { label = ui.t("chat.composer.credits_add"):get(), run = "chat.add_credits" },
			}
			return false
		end
		local resume = opts ~= nil and opts.resume == true
		-- Zeron's commands the send takes: nothing goes to this chat's turn.
		local draft = shared.text:peek()
		local side = AppCommands.side(draft)
		if side and not resume then
			shared.text:set("")
			open_side_chat(side)
			return false
		end
		local focus = AppCommands.compact(draft)
		if focus and not resume then
			shared.text:set("")
			compact(focus)
			return false
		end
		-- A draft that is only one of Zeron's own commands runs it (its
		-- token goes; anything carried stays).
		local zeron = completions.command_of(draft)
		if zeron and not resume then
			shared.text:set("")
			ZeronCommands.run(zeron, session_id:peek())
			return false
		end
		-- Nowhere it can start (a project no longer there): the line above
		-- says why, and the draft stays.
		if not session_id:peek() and where.blocked:peek() then
			return false
		end
		local s_now = open()
		local goes_to = if session_id:peek() then (if s_now then s_now.device else nil) else where.plan().device
		local elsewhere = (goes_to ~= nil and goes_to ~= sessions.now:peek().here) or (not session_id:peek() and where.plan().cloud ~= nil)
		local too_large = if elsewhere then shared.linked_elsewhere(shared.cargo:peek()) else nil
		if too_large then
			own_notice:set { tone = "danger", text = too_large, dismissable = true }
			return false
		end
		local typed, carried = shared.take()
		local parts = Outgoing.parts(if resume then "" else typed, text, carried)
		local words, text_out, attachments = parts.words, parts.text, parts.attachments
		local id = session_id:peek()
		own_notice:set(nil)
		if id then
			-- The prompt shows at once, rising to the view's top with its
			-- reply's room under it (ds/transcript's send); the engine's copy
			-- arrives as the same row.
			-- (While a turn runs it's queued instead: it shows in the queue.)
			local req: { [string]: any } = { session = id, text = text_out, attachments = attachments, appshots = parts.appshots, restores = true }
			pcall(ui.invoke, "chat.prediction_consume", { id = id })
			if running:peek() and (prefs.follow_up:peek() == "steer") ~= (opts ~= nil and opts.inverted == true) then
				req.steer_now = true
			end
			-- A chat with no model of its own runs the one its chip shows, of
			-- its own agent (another's goes through the switch dialog).
			local opened = open()
			if opened and (opened :: any).model == nil and not running:peek() then
				for _, m in store.catalog:peek().models do
					if m.value == model:peek() and m.harness == (opened :: any).harness then
						req.model = m.value
						req.effort = effort:peek()
						req.harness = m.harness
					end
				end
			end
			local queued_row: string? = nil
			local function deliver(message_id: string?)
				req.message_id = message_id
				store.send(req :: any):after(function(got: any)
					if queued_row then
						asks.queue_added(queued_row, got)
					end
				end, function(e: any)
					if queued_row then
						asks.queue_failed(queued_row)
					end
					-- (The session's line says why, when the engine did.)
					local s = store.session:peek()
					if not (s and s.notice) then
						failure_notice(e)
					end
					shared.restore(text_out, carried)
				end)
			end
			if running:peek() then
				local s = open()
				-- Steered as it goes into a turn that takes it: no row to show.
				if not (req.steer_now and s and s.steers == true) then
					queued_row = asks.queue_add(text_out, if #attachments > 0 then attachments else nil)
				end
				deliver(nil)
				return false
			end
			-- Handed to the host as the prompt joins the transcript (ds/
			-- transcript's send steps clear of the end first, a frame
			-- before): nothing of the host's (its copy of the prompt, the
			-- engine's) can reach the list before the glide is ready, and
			-- the prompt is there once, whoever adds it first.
			local mid = store.new_id()
			T.send(function()
				local here = store.messages:index_of("id", mid) ~= nil
				if not here then
					store.messages:push {
						id = mid,
						role = "user",
						text = words,
						attachments = parts.shown,
						-- On its way from this moment: the line under it
						-- says so at once.
						waiting = "sending",
					}
				end
				deliver(mid)
			end, mid)
			return false
		end
		local req: store.Request = {
			text = text_out,
			attachments = attachments,
			appshots = parts.appshots,
			project = project:peek(),
			device = device:peek(),
			checkout = checkout:peek(),
			branch = branch:peek(),
			model = model:peek(),
			effort = effort:peek(),
			fast = fast:peek(),
			options = options:peek(),
			access = access:peek(),
			plan = plan:peek(),
			profile = composer_profiles.find(draft_profile:peek()) and draft_profile:peek(),
		}
		for _, m in store.catalog:peek().models do
			if m.value == req.model then
				req.harness = m.harness
			end
		end
		local plan = where.plan()
		own_notice:set { key = if plan.cloud then "chat.composer.starting_cloud" else "chat.composer.starting", spinner = true }
		-- (`why`: what to say instead, a Cloud chat's reason in words.)
		local failed = function(why: string?)
			own_notice:set { tone = "danger", text = why, key = if why then nil else "chat.composer.didnt_start", dismissable = true }
			shared.restore(text_out, carried)
		end
		-- Made where the project is (or on the device chosen), then its first
		-- message run there, as the shipping app starts one (the send sets
		-- its config: the model, effort, access and plan chosen here): in
		-- the project's folder, the worktree its ref is checked out in, or a
		-- new worktree off that ref (screens/chat/where's plan). On Cloud, made
		-- there: the run goes in its machine's checkout, named as it's made.
		req.project = plan.project
		req.device = plan.device
		req.branch = plan.branch
		sessions
			.create({
				project = plan.project,
				device = if plan.project then nil else plan.device,
				branch = plan.branch,
				cwd = if plan.worktree then nil else plan.cwd,
				cloud = plan.cloud,
			})
			:after(function(made: any)
				local id = if plan.cloud and type(made) == "table" then made.id else made
				if type(id) ~= "string" then
					failed()
					return
				end
				if plan.cloud then
					plan.cwd = if type(made.cwd) == "string" and made.cwd ~= "" then made.cwd else nil
				end
				req.session = id
				req.fresh = true
				-- Sent from here: its turn holds the runway as it opens.
				req.message_id = store.new_id()
				T.sent_here(req.message_id :: string)
				req.cwd = plan.cwd
				req.worktree = plan.worktree
				-- Opened first, then sent: the host shows a prompt at once
				-- only for the chat it has open, so the new page has your
				-- message from its first frame, sending, rather than nothing
				-- until the engine's first word. (Sent after it, it waited
				-- for the transcript, an empty page meanwhile.) A send that
				-- fails then says so on that message, with Retry, as any
				-- chat's does.
				own_notice:set(nil)
				store.open(id)
				-- (Down with its prompt: the dock's one slide.)
				slide_next = true
				route.open_session(id)
				store.send(req):after(function() end, function(_e: any) end)
			end, function(e: any)
				-- (A Cloud chat's reason is said in words; an outcome word isn't.)
				local why = if plan.cloud and type(e) == "table" and type(e.message) == "string" and string.find(e.message, " ", 1, true)
					then e.message
					else nil
				failed(why)
			end)
		return false
	end

	-- Where the open chat works, live: git's current branch in its own
	-- folder (an agent may have made or switched one since it began; the
	-- branch named on the chat is only where it started), and whether that
	-- folder is a worktree of its project's. Asked again as each turn ends.
	local live_branch = ui.state("chat_live_branch", { key = "", name = nil :: string? })
	ui.effect(function()
		local s = open()
		if not s or not s.cwd then
			return
		end
		local key = `{s.id}|{s.cwd}|{if s.running then "run" else "idle"}`
		if live_branch:peek().key == key then
			return
		end
		local cwd, id = s.cwd, s.id
		local known = if live_branch:peek().key:sub(1, #id + 1) == `{id}|` then live_branch:peek().name else nil
		live_branch:set { key = key, name = known or (s :: any).branch }
		local here = sessions.now:peek().here
		local ok, answer = pcall(store.refs, cwd, if s.device ~= here then s.device else nil)
		if not ok or answer == nil then
			return
		end
		answer:after(function(list: any)
			if live_branch:peek().key ~= key then
				return
			end
			for _, ref in list or {} do
				if ref.current then
					live_branch:set { key = key, name = ref.name }
				end
			end
		end, function() end)
	end)
	local accounts = shared.model_accounts(function(): string?
		return device:get()
	end)
	local place = SessionPlace.bind {
		open = open,
		cloud = Where.CLOUD,
		live_branch = function(): string?
			local s = open()
			local lb = live_branch:get()
			return if s and lb.key:sub(1, #s.id + 1) == `{s.id}|` then lb.name else (s and (s :: any).branch)
		end,
		notice = function(n: any)
			own_notice:set(n)
		end,
	}
	-- The agent the composer runs, as its catalog lists it.
	local function harness_row(): any
		local id = agent()
		for _, h in store.catalog:get().harnesses do
			if (h :: any).id == id or (h :: any).value == id then
				return h
			end
		end
		return nil
	end
	-- A usage limit, in Zeron's words (composer/limit_notice.rs).
	local function reset_words(at: number?): string?
		if not at or at <= os.time() then
			return nil
		end
		local left = at - os.time()
		local stamp = if left < 22 * 3600
			then string.gsub(os.date("%I:%M %p", at) :: string, "^0", "")
			elseif left < 7 * 86400 then os.date("%a", at)
			else string.gsub(os.date("%b %d", at) :: string, " 0", " ")
		local said = tostring(stamp)
		return if string.find(said, ":", 1, true) then ui.t("chat.composer.limit.at", { time = said }):get() else said
	end
	local function limit(): any
		local s = open()
		local l = s and (s :: any).limit
		if not s or not l then
			return nil
		end
		local h = harness_row()
		local name = if h and (h :: any).label then (h :: any).label else ui.t("chat.composer.limit.agent"):get()
		local at = reset_words(if l.resets_at then l.resets_at / 1000 else nil)
		local waiting = l.auto == true and at ~= nil
		local function call(op: string, next_op: string?)
			local ok, answer = pcall(ui.call, "chat.limit", { id = s.id, op = op })
			if ok and answer then
				answer:after(function()
					if next_op then
						call(next_op)
					end
				end, function() end)
			end
		end
		local function act(op: string, next_op: string?)
			return function()
				call(op, next_op)
			end
		end
		local actions: { any } = {}
		if not waiting or l.to then
			table.insert(actions, {
				label = if l.to_label then ui.t("chat.composer.limit.switch_to", { model = l.to_label }):get() else ui.t("chat.composer.limit.switch"):get(),
				primary = not waiting,
				on_click = if l.to then act("switch") else function()
					shared.open_model_picker()
				end,
			})
		end
		if waiting then
			table.insert(actions, { label = ui.t("chat.composer.limit.cancel"):get(), on_click = act("cancel") })
			return { waiting = true, rest = ui.t("chat.composer.limit.waiting", { name = name, at = at }):get(), actions = actions }
		end
		if at then
			table.insert(actions, { label = ui.t("chat.composer.limit.wait"):get(), on_click = if l.to then act("wait", "arm") else act("arm") })
		end
		return {
			waiting = false,
			lead = ui.t("chat.composer.limit.reached", { name = name }):get(),
			rest = if at then ui.t("chat.composer.limit.resets", { at = at }):get() else ui.t("chat.composer.limit.unknown"):get(),
			actions = actions,
		}
	end

	local function has_words(): boolean
		return string.find(shared.text:get(), "%S") ~= nil
	end
	local cargo_props = shared.cargo_props(nil, function()
		shared.dropping:set({})
	end)
	-- A pick on a chat with history asks first; answered yes, it lands on the
	-- chat with its divider.
	local close_picker = ui.state("chat_model_close", 0)
	local take_model, switch_dialog = model_switch.use("page", {
		current = function(): string?
			return model:peek()
		end,
		history = function(): boolean?
			if session_id:peek() == nil then
				return nil
			end
			return model_switch.has_history(store.messages:peek(), running:peek(), open() ~= nil)
		end,
		land = function(value: string, ask: model_switch.Ask?)
			landing = ask
			model:set(value)
			close_picker:set(close_picker:peek() + 1)
			-- (The keys back to the draft, once the dialog has gone.)
			ui.after(DIALOG_OUT, function()
				shared.focus:add(1)
			end)
		end,
		back = function()
			shared.open_model:set(shared.open_model:peek() + 1)
		end,
	})
	-- Its agent profiles: the card's open and close.
	local open_profiles = ui.state("chat_profiles_open", 0)
	local close_profiles = ui.state("chat_profiles_close", 0)
	local profiles, profile_dialog = composer_profiles.bind {
		key = "page",
		chat = function(): string?
			return session_id:get()
		end,
		session = function(): any?
			return open()
		end,
		committed = function(): boolean
			return session_id:get() ~= nil
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
		draft_profile = draft_profile,
		history = function(): boolean?
			return model_switch.has_history(store.messages:peek(), running:peek(), open() ~= nil)
		end,
		reopen = function()
			open_profiles:set(open_profiles:peek() + 1)
		end,
		refocus = function()
			shared.focus:add(1)
		end,
		close = close_profiles,
	}
	local function composer(): Node
		return ds.Composer {
			profiles = profiles,
			open_profiles = open_profiles,
			-- Its measures, fold and open tray kept across a reload, so it
			-- is laid out as it was from the first frame.
			keep = "chat",
			toolbar_id = "composer-toolbar",
			open_model = shared.open_model,
			model_open = model_open,
			model_committed = function(): boolean
				return session_id:get() ~= nil
			end,
			model_take = take_model,
			model_looking = function(): boolean
				return store.catalog:get().state ~= "ready"
			end,
			model_retry = store.load_catalog,
			model_close = close_picker,
			context_tokens = function(): number?
				local s = open()
				return if s then (s :: any).context_tokens else nil
			end,
			tray = TRAY,
			value = shared.text,
			placeholder = ui.computed(function(): string
				if new_chat() and project:get() == Where.NO_PROJECT then
					return ui.t("chat.composer.no_project_placeholder"):get()
				end
				return ui.t("chat.composer.placeholder"):get()
			end) :: any,
			expanded = shown_new,
			snap = snap,
			menus_below = shown_new,
			running = running,
			access = access,
			plan = plan,
			-- (While the catalog comes, or from an older host: there.)
			plan_available = function(): boolean
				local h = harness_row()
				return h == nil or h.supports_plan ~= false
			end,
			plan_asks = function(): boolean
				local h = harness_row()
				if h and h.plan_asks ~= nil then
					return h.plan_asks == true
				end
				local a = agent()
				return a == "claude-code" or a == "mock"
			end,
			access_offered = function(): { string }?
				local h = harness_row()
				if not h or h.access_modes == nil then
					return nil
				end
				return if #h.access_modes > 0 then h.access_modes else { "full" }
			end,
			access_note = function(): string?
				local h = harness_row()
				if not h or h.access_modes == nil or #h.access_modes > 1 then
					return nil
				end
				local only = h.access_modes[1] or "full"
				return ui.t("chat.composer.access_only", {
					name = h.label or "",
					mode = string.lower(ui.t(`ds.composer.access.{only}`):get()),
				}):get()
			end,
			prediction = function(): string?
				local s = open()
				if not s or editing:get() then
					return nil
				end
				return (s :: any).prediction
			end,
			resume = function(): boolean
				local s = open()
				return s ~= nil and (s :: any).stopped == true
			end,
			on_resume = function()
				send(RESUME_PROMPT, { resume = true })
			end,
			limit = limit,
			pills = Pills.Row { bind = Pills.window_bind(), tasks = asks.tasks, waiting = true },
			review = Review.Chip { chat = function(): string?
				return session_id:get()
			end },
			review_findings = Review.Findings { chat = function(): string?
				return session_id:get()
			end },
			side_tab = {
				shown = function(): boolean
					return session_id:get() ~= nil
				end,
				open = function(): boolean
					local id = session_id:get()
					return id ~= nil and Lanes.side_tab_open ~= nil and Lanes.side_tab_open(id)
				end,
				status = function(): string?
					local now = os.time()
					local newest: any = nil
					for _, c in Lanes.side(session_id:get()) do
						if (c.kind == nil or c.kind == "side_chat") and (c.expires == nil or c.expires > now) and (not newest or (c.created or 0) > (newest.created or 0)) then
							newest = c
						end
					end
					local status = newest and newest.status
					if status == "working" then
						return "$accent"
					elseif status == "awaitingInput" then
						return "$warning"
					elseif status == "errored" then
						return "$danger"
					end
					return nil
				end,
				hint = function(): string?
					return ui.shortcut_text(side_chat_key())
				end,
				on_click = function()
					open_side_chat(nil)
				end,
			},
			send_block = function(): string?
				return if out_of_credits() then ui.t("chat.composer.credits_tip"):get() else nil
			end,
			caption = function(): string?
				local s = open()
				if not s or not running:get() or (s :: any).steers ~= false then
					return nil
				end
				if prefs.follow_up:get() == "steer" and has_words() then
					return ui.t("chat.composer.steer_queues"):get()
				end
				return nil
			end,
			model = model,
			effort = effort,
			fast = fast,
			options = options,
			-- The catalog, followed in place as it comes and changes.
			harnesses = function(): { ds.Harness }
				local c: any = store.catalog:get()
				return c.tabs or c.harnesses
			end,
			models = function(): { ds.Model }
				return store.catalog:get().models
			end,
			accounts = accounts.accounts,
			on_account = accounts.on_account,
			on_accounts_open = accounts.on_accounts_open,
			-- What its context holds (the ring shows once a window is
			-- reported, its card saying the numbers).
			usage = function(): ds.ComposerUsage?
				local s = open()
				if not s or (s.context_tokens == nil and s.context_window == nil) then
					return nil
				end
				return { tokens = s.context_tokens, window = s.context_window }
			end,
			stats = function(): any
				return store.stats:get()
			end,
			-- Compaction: the chip while it checks or compacts, the resume
			-- card on a large idle Claude chat, and the ring card's part.
			compaction = {
				label = function(): string?
					local s: any = open()
					return Compaction.chip_label(if s then s.compaction_wait else nil)
				end,
				resume = resume_offer,
				on_resume_compact = function()
					compact("")
				end,
				on_resume_keep = function()
					local s: any = open()
					if s and s.resume then
						resume_kept:set({ id = s.id, at = s.resume.at })
					end
				end,
				plan = function(): any
					local s: any = open()
					return if s then s.compact_plan else nil
				end,
				blocked = function(): string?
					local s: any = open()
					local b = if s then s.compact_blocked else nil
					if b == "compacting" then
						return ui.t("chat.composer.compact_in_progress"):get()
					elseif b == "running" then
						return ui.t("chat.composer.compact_wait_turn"):get()
					elseif type(b) == "table" then
						return b.text
					end
					return nil
				end,
				on_compact = function()
					compact("")
				end,
			},
			working = function(): boolean
				local s: any = open()
				if s and s.in_flight ~= nil then
					return s.in_flight
				end
				return s ~= nil and s.running == true
			end,
			footer_git = function(): boolean
				local s = open()
				for _, p in sessions.now:get().projects do
					if p.id == (if s then s.project else nil) then
						return p.git == true
					end
				end
				return false
			end,
			on_terminal = function()
				local panel: any = require("@app/screens/shell/boundary").load("@app/screens/panel")
				if panel and panel.toggle_terminal then
					panel.toggle_terminal()
				end
			end,
			terminal_open = function(): boolean
				local panel: any = require("@app/screens/shell/boundary").load("@app/screens/panel")
				return panel ~= nil and panel.terminal_shown ~= nil and panel.terminal_shown()
			end,
			terminal_keys = function(): string
				return require("@app/screens/shell/keys").combo("shell.toggle_terminal")
			end,
			project = project,
			projects = where.projects,
			-- (The shell's New project palette.)
			on_new_project = function()
				local add: any = require("@app/screens/shell/boundary").load("@app/screens/shell/add_project")
				if add then
					add.open()
				end
			end,
			device = device,
			devices = where.devices,
			checkout = checkout,
			branch = branch,
			branches = where.branches,
			branches_empty = where.branches_empty,
			branch_fixed = place.fixed,
			branch_menu = place.menu,
			cargo = shared.cargo,
			on_remove_cargo = shared.remove_cargo,
			on_paste_text = cargo_props.on_paste_text,
			on_open_pasted = cargo_props.on_open_pasted,
			on_attach = cargo_props.on_attach,
			-- (A drop on the pill: the chat's overlay goes with it.)
			on_files = cargo_props.on_files,
			on_paste_image = cargo_props.on_paste_image,
			drop_overlay = false,
			on_send = send,
			on_stop = function()
				local id = session_id:peek()
				if id then
					store.stop(id):after(function() end, function(e: any)
						own_notice:set { tone = "danger", text = ui.t("chat.composer.stop_failed", { why = tostring(if type(e) == "table" then e.message else e) }):get(), dismissable = true }
					end)
				end
			end,
			-- The shipping app's settings: what sends, and whether Escape
			-- stops the agent.
			send_with = function(): string
				return prefs.send_with:get()
			end,
			-- A queued message being edited: Escape leaves it as it was.
			on_escape = function(): boolean
				if editing:peek() then
					end_edit("cancel")
					return true
				end
				return store.escape()
			end,
			queue = queue,
			queue_pictures = queue_pictures,
			can_steer = asks.can_steer,
			on_queue_send = asks.on_queue_send,
			on_queue_steer_all = asks.on_queue_steer_all,
			on_dismiss_todos = asks.on_dismiss_todos,
			on_queue_edit = function(item: string)
				local id = session_id:peek()
				if not id or editing:peek() then
					return
				end
				store.queue_edit(id, item):after(function(got: any)
					editing:set { item = item, aside = shared.text:peek() }
					shared.text:set(if got then got.text else "")
					shared.focus:add(1)
				end, function(err: any)
					own_notice:set {
						tone = "warning",
						key = if err and err.message == "locked" then "chat.composer.edit_locked" else "chat.composer.edit_unavailable",
						dismissable = true,
					}
				end)
			end,
			on_queue_save = function()
				end_edit("commit")
			end,
			on_queue_cancel = function()
				end_edit("cancel")
			end,
			on_queue_remove = asks.on_queue_remove,
			on_queue_reorder = asks.on_queue_reorder,
			todos = asks.todos,
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
			notice = notice,
			on_dismiss_notice = function()
				own_notice:set(nil)
				shared.refused:set(nil)
			end,
			disabled = function(): boolean
				local s = store.session:get()
				local id = session_id:get()
				return id ~= nil and s ~= nil and s.id == id and (s.state == "missing" or s.state == "offline")
			end,
			focus = shared.focus,
			mentions = completions.mentions,
			on_mention = completions.on_mention,
			mention_enabled = completions.mention_enabled,
			mention_empty = completions.mention_empty,
			mention_failed = completions.mention_failed,
			mention_token = completions.mention_token,
		}
	end

	local page = ui.ref()
	-- The page's height as last laid out, kept (a reload keeps state): a
	-- composer made again reads it until its own page is measured, so its
	-- spring starts where it rests instead of rising into place again.
	local page_h = ui.state("chat_page_h", 0)
	local composer_zone = shared.drop_zone("composer")
	-- There to open the model picker in (the shell's ⌘/), while it is.
	ui.on_mount(function()
		shared.composer_shown:set(true)
	end)
	ui.on_cleanup(function()
		shared.composer_shown:set(false)
	end)
	local dock = ui.ref()
	-- How tall a new chat's composer is with nothing in it (what it's placed
	-- by, so what's written in it doesn't move its top).
	local rest_tall = ui.state("chat_composer_rest_tall", nil :: number?)
	-- (A draft just deleted, it's empty while still easing back, and that
	-- height would move its top: taken once that's over.)
	local REST_SETTLE_MS = 500
	local had_draft = false
	local rest_settles_at = 0
	ui.effect(function()
		local tall = dock.height or 0
		if shared.text:get() ~= "" or #shared.cargo:get() > 0 or next(shared.quotes:get()) ~= nil then
			had_draft = true
			return
		end
		if had_draft then
			had_draft = false
			rest_settles_at = ui.now() + REST_SETTLE_MS
		end
		if tall <= 0 or not new_chat() then
			return
		end
		local function take()
			if math.abs((rest_tall:peek() or 0) - tall) >= 0.5 then
				rest_tall:set(tall)
			end
		end
		local left = rest_settles_at - ui.now()
		if left > 0 and rest_tall:peek() ~= nil then
			ui.after(left, take)
		else
			take()
		end
	end)
	-- Where it stands: mid-page on a new chat (centred under the title bar),
	-- else docked at the foot (0), as laid out now.
	local function target_y(): number
		if not new_chat() then
			return 0
		end
		-- (Measured, else as last measured: a node made again reads 0 until
		-- laid out.)
		local h = page.height or 0
		if h <= 0 then
			h = page_h:get()
		end
		local tall = dock.height or 0
		if tall <= 0 then
			tall = shared.dock_h:get()
		end
		-- (Its top stays as a draft grows: it's placed by its empty height,
		-- and grows down from there. Only a draft that would run off the
		-- page's foot moves it, up by as much.)
		local rest = rest_tall:get() or tall
		local hero = TITLE_H + (h - TITLE_H - 96) / 2
		local docked = h - FOOT - rest / 2
		return math.min(hero - docked, rest - tall)
	end
	-- Moving between a new chat and a session: its own send from a new chat
	-- slides it down with the prompt (`slide_next`); any other move (another
	-- chat opened, a new chat) puts it there at once, in its new form and
	-- size, no fade, no slide, nothing growing (and as that size is
	-- measured, it follows at once too). In the page, as it grows or the window resizes, it
	-- moves there as the dock's spring did. Motion reduced, it's there.
	local GLIDE = { duration = 260, ease = "out(3)" }
	-- How long after a switch its place follows its size at once (the
	-- form's size, there at once too (`snap`), is measured a frame later).
	local SNAP_MS = 400
	-- (Kept state, as this file's others: the composer is made in a plain
	-- function, not a component.)
	local place = ui.state("chat_composer_place", 0)
	-- (Held where it is while its model picker is open: `model_open`.)
	local was_new: boolean? = nil
	-- (Its model picker open on a new chat: how far its top gives way so its
	-- chip stays, the picker hanging from it, as what's written grows.)
	local hold = ui.state("chat_composer_hold", 0)
	local held_tall: number? = nil
	local gliding: any = nil
	local snap_until = 0
	local function glide_to(y: number)
		if gliding then
			gliding:cancel()
			gliding = nil
		end
		if motion.reduced_now() or ui.now() < snap_until or math.abs(place:peek() - y) < 0.5 then
			place:set(y)
			return
		end
		gliding = anime.animate(place, { value = y, duration = GLIDE.duration, ease = GLIDE.ease })
	end
	-- Made again (as the window opens on its last session, the composer is
	-- built twice), a glide begun here must not land after: `place` is kept,
	-- and it would put a session's composer where a new chat's stood.
	ui.on_cleanup(function()
		if gliding then
			gliding:cancel()
			gliding = nil
		end
	end)
	ui.effect(function()
		local y = target_y()
		local nc = new_chat()
		local switched = was_new ~= nil and nc ~= was_new and not slide_next
		was_new = nc
		slide_next = false
		if switched then
			snap_until = ui.now() + SNAP_MS
			snap:set(snap:peek() + 1)
		end
		form_new:set(nc)
		-- On a new chat its place is the layout's (see the row below), so the
		-- page resizing moves it with the page; this is only where a send
		-- slides from.
		if nc and not switched then
			if gliding then
				gliding:cancel()
				gliding = nil
			end
			place:set(y)
			local tall = dock.height or 0
			if model_open:get() then
				held_tall = held_tall or tall
				hold:set(held_tall - tall)
			elseif held_tall then
				held_tall = nil
				anime.animate(hold, { value = 0, duration = GLIDE.duration, ease = GLIDE.ease })
			end
			return
		end
		held_tall = nil
		hold:set(0)
		glide_to(y)
	end)
	-- Over the whole pane, the pointer passing through it to the page
	-- (its wheel too) but for the composer itself. (Under settings the shell
	-- hides the chat frame, its state kept: nothing here is made again.)
	return ui.overlay {
		ref = page,
		-- Not seen over a plugin's page (the chat's is kept unseen under it),
		-- nor anything placed round the composer: the page takes the pointer.
		opacity = function(): number
			return if route.current:get().name == "plugin" then 0 else 1
		end,
		on_resize = function(size: { w: number, h: number })
			if math.abs(page_h:peek() - size.h) >= 1 then
				page_h:set(size.h)
			end
		end,
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		ui.row {
			absolute = true,
			pointer_events = "none",
			left = 0,
			right = 0,
			-- On a new chat by its top, from the page's middle: it moves with
			-- the page as the page lays out, and what's written grows it down.
			-- Docked, by its foot.
			top = function(): any
				return if new_chat() then "50%" else nil
			end,
			bottom = function(): any
				return if new_chat() then nil else FOOT
			end,
			px = require("@app/screens/chat/dock").SIDES,
			justify = "center",
			-- Mid-page on a new chat, docked on a session: sliding there as it
			-- sends from a new chat (with its prompt), there at once otherwise
			-- (another chat, a new chat: `place`).
			id = "chat-composer-dock",
			y = function(): number
				if not new_chat() then
					return place:get()
				end
				local tall = dock.height or 0
				if tall <= 0 then
					tall = shared.dock_h:get()
				end
				local rest = rest_tall:get() or tall
				local at = TITLE_H / 2 - 48 - rest / 2
				-- (Only a draft that would run off the page's foot moves it.)
				local h = page.height or 0
				if h > 0 then
					at = math.min(at, h / 2 - FOOT - tall)
				end
				return at + hold:get()
			end,
			ui.div {
				ref = dock,
				id = "chat-composer",
				hook = "chat.composer",
				-- (Over the page, not in it: the session it sends to, as the
				-- session page provides it; none on a new chat.)
				provide = { session = session_id },
				w_full = true,
				pointer_events = "auto",
				max_w = ui.spring(
					ui.computed(function(): number
						return prefs.conversation_width:get()
					end),
					DOCK
				),
				on_resize = function(size: { w: number, h: number })
					if math.abs(shared.dock_h:peek() - size.h) > 0.01 then
						shared.dock_h:set(size.h)
					end
				end,
				-- Files carried over the composer itself: the chat's drop,
				-- as over the page under it.
				on_file_hover = composer_zone.on_file_hover,
				on_file_drop = composer_zone.on_file_drop,
				drop_when = composer_zone.drop_when,
				on_drag_over = composer_zone.on_drag_over,
				on_drag_leave = composer_zone.on_drag_leave,
				on_hover = composer_zone.on_hover,
				on_drop = composer_zone.on_drop,
				-- (A new chat with nowhere to work: its page asks for a project in
				-- its place, as Zeron's first launch does. Hidden here, inside
				-- the pill's own box: a `hidden` on the row above, which spans
				-- the pane, left a box over the transcript taking its wheel.)
				-- (And not over a plugin's page: the chat's page is kept unseen
				-- under it, and so is this.)
				ui.show(function(): boolean
					return shown_new() and not route.needs_project:get() and route.current:get().name ~= "plugin"
				end, ui.row {
					id = "new-chat-headline",
					absolute = true,
					left = 0,
					right = 0,
					top = -(HEADLINE_LINE + HEADLINE_GAP + 28),
					h = HEADLINE_LINE,
					justify = "center",
					items = "center",
					pointer_events = "none",
					exit = motion.transition({ opacity = 0, duration = 160, ease = "out(3)" }),
					ui.text {
						ui.t("chat.home.headline"),
						text_size = ui.raw(30, "gpui's hero headline"),
						line_height = ui.raw(HEADLINE_LINE, "gpui's hero headline"),
						color = "$text.primary",
						wrap = false,
					},
				}),
				ui.show(function(): boolean
					return not route.needs_project:get() and route.current:get().name ~= "plugin"
				end, composer()),
			},
		},
		-- Files carried over the chat (the page or the composer): all of it
		-- dimmed under "Drop to attach", as Zeron's conversation shows it
		-- (shell.rs `attachment_drop_overlay`: the scrim, the words in the
		-- text's ink, at once and gone at once; no accent).
		DropOverlay { shown = shared.carrying, id = "chat-drop" },
		switch_dialog,
		profile_dialog,
		-- (A sidebar session carried: dropped, a mention; production's words.)
		DropOverlay { shown = shared.carrying_session, id = "chat-drop-mention", label = ui.t("chat.composer.drop_to_mention") :: any },
	}
end

return Composer
