-- The window's one composer, wired: it travels between a new chat (lifted
-- to a little over the page's middle, opened out, its menus below) and a
-- session (docked at the foot, its menus above), on one critically damped
-- spring, as the gallery's App shell has it (Zeron's composer_dock.rs). It
-- is made once and follows where the shell is: the draft and what it
-- carries are each chat's own (screens/chat/shared), its model, effort
-- and access the session's (changed there, they change the session) or,
-- on a new chat, the new chat's own: those last chosen there, never a
-- chat's.
--
-- In a session it sends to that session (queued while a turn runs, which
-- Stop ends), and shows what the agent asks (an approval, a question) in
-- the pill, the queue and the plan in the deck over it. On a
-- new chat it starts a chat in the route's project and opens it.
--
-- `Composer()` is placed once over the page pane (absolute, filling it):
-- it lays itself out.

local ds = require("@ds")
local Draft = require("@ds/draft_text")
local Dictation = require("@app/screens/chat/dictation")
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
local cloud = require("@account/cloud")
local LinkNotice = require("@app/screens/chat/link_notice")
local Compaction = require("@ds/compaction")
local CloudTray = require("@app/screens/chat/cloud_tray")
local Held = require("@app/screens/chat/held")
local CloudMachine = require("@app/screens/chat/cloud_machine")
local Where = require("@app/screens/chat/where")
local Warm = require("@app/screens/chat/warm")
local DropOverlay = require("@ds/drop_overlay")
local Asks = require("@app/screens/chat/asks")
local Invocations = require("@app/screens/chat/invocations")
local Completions = require("@app/screens/chat/completions")
local ReplyImages = require("@app/screens/chat/reply_images")
local ZeronCommands = require("@app/screens/chat/zeron_commands")
-- (Loaded with the chat page: captures land on its composer.)
local Lanes = require("@app/screens/chat/lanes")
local Pills = require("@app/screens/chat/pills")
local AppCommands = require("@app/screens/chat/app_commands")
local ConfigPanel = require("@app/screens/chat/config_panel")
local Outgoing = require("@app/screens/chat/outgoing")
local Pages = require("@app/screens/chat/pages")
local Branch = require("@app/screens/chat/branch")

-- gpui's hero headline: its line, and its gap over the 28 of chips above the pill.
local HEADLINE_LINE, HEADLINE_GAP = 38, 18
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

local draft_mover: ((send: boolean) -> boolean)? = nil
for _, c in { { id = "chat.move_draft", title = "ds.composer.move_draft", send = false }, { id = "chat.send_new", title = "ds.composer.send_new", send = true } } do
	ui.command {
		id = c.id,
		title = ui.t(c.title):get(),
		menu = "File",
		key = function(): string
			local ok, keys = pcall(require, "@app/screens/shell/keys")
			return if ok and type(keys) == "table" then (keys :: any).key(c.id)() else ""
		end :: any,
		run = function(): boolean
			return draft_mover ~= nil and draft_mover(c.send)
		end,
	}
end

ds.toast.action("chat.add_credits", function()
	route.settings("billing")
end)
-- Zeron's dock spring (ω ≈ 27).
local DOCK: SpringOptions = { stiffness = 730, damping = 54 }
-- Its gap to the window's foot, docked. Its width, rising and docked: the
-- transcript's column (Settings' "Conversation width"), as Zeron's
-- composer lines up with the conversation over it (its edges on the
-- column's).
local Dock = require("@app/screens/chat/dock")
local FOOT = Dock.GAP
local composer_surface = require("@ds/composer_surface")
store.dismiss_cards = Pills.dismiss

-- What a new chat starts with: what was last chosen on the new-chat page
-- (its settings, its own: a chat shown or made never changes them), kept
-- across launches. Each is put back as it was, none included, as the page
-- is come back to.
local defaults = ui.persist("chat_new_defaults", {
	model = nil :: string?,
	effort = nil :: string?,
	fast = false,
	options = nil :: { [string]: string }?,
	access = "full" :: string?,
	plan = false,
	mission = false :: any,
	checkout = "worktree" :: string?,
	device = nil :: string?,
	-- Where (nil: none picked yet; screens/chat/where picks one).
	project = nil :: string?,
	branch = nil :: string?,
})
store.new_defaults = defaults
-- A new chat's own value of each choice where its default is none: what
-- the composer shows then.
local NONE: { [string]: any } = { fast = false, options = {}, plan = false, mission = false }

-- The last default a plugin chose that was taken (its time, ms): one is
-- taken once, across restarts, never again over a pick made since.
local plugin_default_seen = ui.persist("chat_new_defaults_plugin", 0)

local FIELDS = { "model", "effort", "fast", "options", "access", "plan", "mission" }
-- Past ds/dialog's leaving and its modal's going (which gives the keys back
-- to what had them), the draft takes them.
local DIALOG_OUT = 350
local STARTING_AFTER = 400
-- Where a new chat is made, kept with them.
local WHERE = { "checkout", "device", "project", "branch" }
-- Whether two choices are the same: the options' picks (tables) by their
-- entries, none and an empty table alike; the rest by value.
-- An agent's default model: the row its catalog marks, else its first
-- (harness nil: the first agent listed's).
local function default_of(models: { any }, harness: string?): any
	local first = nil
	for _, m in models do
		local h = harness or m.harness
		harness = h
		if m.harness == h then
			if m.default then
				return m
			end
			first = first or m
		end
	end
	return first
end
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
-- The deck's tab open (its queue or plan), kept across a reload.
local TRAY = ui.state("chat_tray", nil :: ("todos" | "queue")?)

local function Composer(): Node
	-- The session shown, or nil on a new chat.
	local session_id = ui.computed(function(): string?
		local now = route.current:get()
		return if now.name == "session" then now.session else nil
	end)
	local function new_chat(): boolean
		return session_id:get() == nil
	end
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
	local mission = ui.state("chat_mission", false :: any)
	local device = ui.state("chat_device", nil :: string?)
	local checkout = ui.state("chat_checkout", "worktree" :: string?)
	local branch = ui.state("chat_branch", nil :: string?)
	local project = ui.state("chat_project", nil :: string?)
	-- A new chat's agent profile (its id, sent with its first message).
	local draft_profile = ui.state("chat_profile", nil :: string?)
	-- On Cloud, what a new chat's machine reaches (nil: the account's
	-- default, Settings → Cloud). Each new chat starts from the default.
	local reach = ui.state("chat_reach", nil :: string?)
	local locals: { [string]: State<any> } =
		{ model = model, effort = effort, fast = fast, options = options, access = access, plan = plan, mission = mission }

	-- The default plan size changed in Settings: a mission switched on here takes it.
	local default_size = prefs.mission_defaults:peek().size
	ui.effect(function()
		local size = prefs.mission_defaults:get().size
		if size == default_size then
			return
		end
		default_size = size
		local m = mission:peek()
		if type(m) == "table" and m.size ~= size then
			local next = table.clone(m)
			next.size = size
			mission:set(next)
		end
	end)

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
	local carry_next: { [string]: any }? = nil
	local undoable: { [string]: any }? = nil
	local moved_note = ui.state("chat_moved_note", nil :: { key: string, undo: boolean, n: number }?)
	moved_note:set(nil)
	ui.effect(function()
		local id = session_id:get()
		local s = open()
		ui.batch(function()
			if id == nil then
				local c = carry_next
				carry_next = nil
				if c then
					checkout:set(c.checkout)
					device:set(c.device)
					project:set(c.project)
					draft_profile:set(c.profile)
				elseif was:peek() ~= nil then
					-- Its own settings, each as it was left (none too: never
					-- what the chat just shown had).
					local d = defaults:peek()
					for _, k in FIELDS do
						local v = (d :: any)[k]
						if v == nil then
							v = NONE[k]
						end
						if not equal(locals[k]:peek(), v) then
							locals[k]:set(v)
						end
					end
					checkout:set(d.checkout)
					branch:set(d.branch)
					-- (A device asked of this new chat, a device's "+" in the
					-- sidebar, over the one last used; a project asked of it,
					-- a project's "+", over the one last picked here.)
					device:set(route.take_device() or d.device)
					local asked = route.take_project()
					project:set(if asked then asked.project elseif d.project ~= nil then d.project else route.current:peek().project)
				end
			elseif s then
				local resolved = shared.resolve_model(s, store.catalog:get())
				for _, k in FIELDS do
					local v = (s :: any)[k]
					if k == "mission" and v == nil then
						v = false
					end
					-- (None found: none shown, never the chat before's.)
					local own = (k == "model" or k == "effort") and resolved.model == nil
					if k == "model" or k == "effort" then
						v = resolved[k]
					end
					if (v ~= nil or own) and not equal(locals[k]:peek(), v) then
						locals[k]:set(v)
					elseif v == nil and (k == "model" or k == "effort") and locals[k]:peek() ~= nil then
						-- A session that names none runs its agent's own: never
						-- the last chat's model (another agent's, maybe one
						-- this device doesn't have), sent along as a pick.
						locals[k]:set(nil)
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
	-- What a new chat starts with, as a plugin allowed "models.set" chose it
	-- (screens/plugins publishes it once the engine allowed it): kept as a
	-- pick made here would be, and shown at once on a new chat.
	local plugin_default = ui.shared("plugins.new_chat_model", nil :: any)
	ui.effect(function()
		local asked = plugin_default:get()
		if type(asked) ~= "table" or type(asked.model) ~= "string" then
			return
		end
		local seq = tonumber(asked.seq) or 0
		if seq <= plugin_default_seen:peek() then
			return
		end
		plugin_default_seen:set(seq)
		local d = table.clone(defaults:peek()) :: any
		d.model = asked.model
		d.effort = asked.effort
		defaults:set(d)
		if session_id:peek() == nil then
			ui.batch(function()
				locals.model:set(asked.model)
				if asked.effort ~= nil then
					locals.effort:set(asked.effort)
				end
			end)
		end
	end)
	-- A project asked of a new chat already showing (a project's "+", the
	-- palette's New chat in…): picked. (One asked on the way here is the
	-- arrival's, above; a project picked here is otherwise the page's own,
	-- whatever the route last asked for.)
	ui.effect(function()
		local pick = route.project_pick:get()
		if pick == nil or session_id:peek() ~= nil or was:peek() ~= nil then
			return
		end
		local asked = route.take_project()
		if asked and project:peek() ~= asked.project then
			project:set(asked.project)
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
		local where: { [string]: any } = {
			checkout = checkout:get(),
			device = device:get(),
			project = project:get(),
			branch = branch:get(),
		}
		local id = session_id:peek()
		if id == nil then
			-- (Only once the page's own are out: never the chat left's, on
			-- their way out.)
			if was:peek() ~= nil then
				return
			end
			local d = table.clone(defaults:peek()) :: any
			local changed = false
			for _, k in FIELDS do
				local v = picked[k]
				if k ~= "mission" and not equal(d[k], v) then
					d[k] = v
					changed = true
				end
			end
			-- (By name: a choice of none is a nil a table's loop skips.)
			for _, k in WHERE do
				if d[k] ~= where[k] then
					d[k] = where[k]
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
	local function run_device(): string?
		local d = device:get()
		if d == nil and session_id:get() == nil then
			local p = route.project:get()
			d = if p then p.device else nil
		end
		return d
	end
	-- Whether the catalog shown is `d`'s (one of this device's says none).
	local function shows_catalog_of(d: string?): boolean
		local shown = (store.catalog:get() :: any).device
		return shown == d or (shown == nil and (d == nil or d == sessions.now:get().here))
	end
	-- (Asked once per device: the host keeps a look current itself, a late
	-- answer to an older one changing nothing, src/chat.rs `reload_remote`.)
	ui.effect(function()
		local d = run_device()
		if d ~= catalog_device:peek() then
			catalog_device:set(d)
			store.catalog_on(d)
		end
	end)
	-- A new chat moved onto Cloud runs only the agents Cloud has accounts
	-- for: a model it doesn't run gives way to Cloud's first, as the
	-- shipping app drops one on the switch. So does one the last new chat
	-- used that its device doesn't offer (once all its agents have said: a
	-- model still on its way isn't dropped), so a chat isn't made on a
	-- device for an agent it doesn't have.
	ui.effect(function()
		local cat = store.catalog:get()
		if session_id:get() ~= nil then
			return
		end
		local d = run_device()
		if d == Where.CLOUD then
			if (cat :: any).device ~= Where.CLOUD then
				return
			end
		elseif (cat :: any).complete ~= true or not shows_catalog_of(d) then
			return
		end
		local picked = model:peek()
		for _, m in cat.models do
			if m.value == picked then
				return
			end
		end
		local first = default_of(cat.models)
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
	-- A new chat's agent, started as the person opens it or types (see
	-- screens/chat/warm), with the send's own choices.
	local warm = Warm.bind {
		session = function(): string?
			return session_id:get()
		end,
		text = shared.text,
		owner = shared.owner,
		request = function(): any?
			-- (Only a new chat's.)
			if not new_chat() then
				return nil
			end
			local req = {
				model = model:get(),
				effort = effort:get(),
				fast = fast:get(),
				options = options:get(),
				access = access:get(),
				plan = plan:get(),
			}
			-- (Heard when the place changes too.)
			device:get()
			project:get()
			checkout:get()
			branch:get()
			if where.blocked:peek() then
				return nil
			end
			local p = where.plan()
			if p.cloud or p.worktree then
				return nil
			end
			-- (Only for a chat that will run here: one on another device
			-- would outlive this app's quit.)
			local here = (sessions.now:peek() :: any).here
			if p.device ~= nil and p.device ~= here then
				return nil
			end
			for _, m in store.catalog:get().models do
				if m.value == req.model then
					(req :: any).harness = m.harness
				end
			end
			if (req :: any).harness == nil then
				return nil
			end
			(req :: any).cwd = p.cwd;
			(req :: any).device = p.device
			return req
		end,
	}
	-- (A new chat's project, for what follows the route: its panel.)
	ui.effect(function()
		if new_chat() then
			local id = project:get()
			if route.new_chat_project:peek() ~= id then
				route.new_chat_project:set(id)
			end
		end
	end)

	local running = ui.computed(function(): boolean
		local s = open()
		return s ~= nil and s.running == true
	end)
	-- What the agent asks, and which of a question's options are chosen
	-- (fresh for each question); its queue and todos. (The same as
	-- a split pane's composer shows them: screens/chat/asks.)
	local own_notice = ui.state("chat_own_notice", nil :: any)
	local saving_row = ui.state("chat_queue_saving", nil :: string?)
	-- The lane of the transcript at its foot (screens/chat/shared's
	-- `at_foot`), while that's the chat `id`'s: in a split, the focused
	-- pane's (`split-<chat>`). Nil: the chat page's.
	local function foot_lane(id: string?): string?
		local lane = string.match(shared.at_foot:peek(), "^lane%.(.+)$")
		return if lane and id and lane == `split-{id}` then lane else nil
	end
	local asks = Asks.bind {
		id = function(): string?
			return session_id:peek()
		end,
		-- (A queued message steered in glides in the transcript at its foot.)
		name = function(): string?
			local lane = foot_lane(session_id:peek())
			return if lane then `lane.{lane}` else nil
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
		say = function(tone: string, key: string)
			own_notice:set { tone = tone, key = key, dismissable = true }
		end,
	}
	local ask, queue = asks.ask, asks.queue
	-- Its own goes as the chat changes (kept: a reload isn't a change).
	local notice_for = ui.state("chat_notice_for", "?")
	ui.effect(function()
		local id = session_id:get() or route.draft:get()
		if notice_for:peek() ~= id then
			notice_for:set(id)
			own_notice:set(nil)
		end
	end)
	-- Its Cloud machine, on a chat that has one: its state and its alert.
	local machine = CloudTray.bind {
		chat = function(): string?
			return session_id:get()
		end,
		device = function(): string?
			local s = open()
			return s and s.device
		end,
		running = function(): boolean
			return running:get()
		end,
		queue = function(): { any }
			return queue:get()
		end,
		waiting = function(): { any }?
			local s = open()
			return s and s.waiting
		end,
		retry = function(message: string?)
			local id = session_id:peek()
			if id and message then
				store.retry(id, message)
			end
		end,
		-- A send on its way: its machine coming up is said in the
		-- transcript, under it.
		sending = function(): boolean
			return CloudMachine.sending_last(store.messages)
		end,
	}
	-- This device's outbox for the chat (#1256): sends uploading or stopped,
	-- each removable where the engine can take one back. Whether it can is
	-- asked once a chat has one, by taking out a send that isn't there (an
	-- engine from before it refuses "unsupported"; then no Remove).
	local outbox = ui.computed(function(): { any }?
		local s = open()
		return s and s.waiting
	end)
	local removes_held = ui.state("chat_removes_held", nil :: { chat: string, can: boolean }?)
	ui.effect(function()
		local chat = session_id:get()
		if not chat or not CloudMachine.has_outbox(outbox:get()) then
			return
		end
		local known = removes_held:peek()
		if known and known.chat == chat then
			return
		end
		removes_held:set({ chat = chat, can = false })
		store.remove_held(chat, "zgx-asks-whether-it-can"):after(function()
			if session_id:peek() == chat then
				removes_held:set({ chat = chat, can = true })
			end
		end, function(err: any)
			local m = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
			-- (Unsure, not refused: asked again next time.)
			if m ~= "unsupported" and session_id:peek() == chat then
				removes_held:set(nil)
			end
		end)
	end)
	local function can_remove_held(): boolean
		local known = removes_held:get()
		return known ~= nil and known.chat == session_id:get() and known.can
	end
	-- The queue it shows: its own (one that waited long says how long), then
	-- what's held back for its machine (screens/chat/held), each removable,
	-- then this device's outbox.
	local shown_queue = ui.computed(function(): { any }
		local out: { any } = {}
		local now = machine.now()
		for _, q in queue:get() do
			if q.at and now - q.at > CloudMachine.LONG_WAIT then
				local row = table.clone(q)
				row.age = CloudMachine.age_words(now - q.at)
				table.insert(out, row)
			else
				table.insert(out, q)
			end
		end
		local reason = machine.hold_reason()
		for _, h in Held.of(session_id:get()) do
			table.insert(out, {
				id = Held.PREFIX .. h.id,
				text = h.text,
				-- (Refused, and its machine can take it: the person's to send.)
				waiting = if h.refused and not reason then nil else CloudMachine.held_label(reason),
			})
		end
		for _, row in CloudMachine.outbox_rows(outbox:get(), can_remove_held()) do
			table.insert(out, row)
		end
		return out
	end)
	-- A held message sent: out of the hold, to the chat. Refused, it's back in
	-- its place, marked so it doesn't only go again at once (and again): its
	-- row's Send sends it, Edit takes its words back, and the composer says
	-- it didn't send. Never lost: only Remove lets it go.
	local function release(chat: string, id: string)
		local e = Held.remove(chat, id)
		if not e then
			return
		end
		store.send(e.req :: any):after(function() end, function()
			local back = table.clone(e)
			back.refused = true
			Held.add(chat, back)
			own_notice:set { tone = "danger", key = "chat.composer.didnt_send", dismissable = true }
		end)
	end
	-- The machine can take them now (a size picked, a fresh machine): what's
	-- held goes, in order, however long it waited.
	ui.effect(function()
		local chat = session_id:get()
		if not chat or machine.hold_reason() ~= nil or machine.session() == nil then
			return
		end
		for _, e in Held.of(chat) do
			if not e.refused then
				release(chat, e.id)
			end
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
			connection = sessions.connection:get(),
			cloud = machine.cloud_link(Draft.has_text(shared.text) or #shared.cargo:get() > 0 or #queue:get() > 0),
			host_unreachable = host_unreachable,
			host = host,
			intent = Draft.has_text(shared.text) or #shared.cargo:get() > 0 or #queue:get() > 0,
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
	-- A new chat with no model yet: the first agent's default. (Never a session's:
	-- its model is its own.) Whenever that's so: the catalog come, or the
	-- new page come back to with none chosen there, so what shows is what a
	-- reload shows too.
	ui.effect(function()
		local c = store.catalog:get()
		if session_id:get() == nil and c.state == "ready" and model:get() == nil and #c.models > 0 then
			local first = default_of(c.models)
			model:set(first.value)
			effort:set(first.effort)
		end
	end)

	-- The agent the chat runs (its model's): whose completion settings the
	-- composer follows (the chat's own when the catalog doesn't name it).
	local function agent(): string?
		return Completions.agent_of(model:get(), if session_id:get() then open() else nil)
	end
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
	local function mission_offered(): boolean
		if new_chat() then
			local row = harness_row()
			local profile = composer_profiles.find(draft_profile:get())
			local off: { string } = if profile then (profile :: any).tools_off or {} else {}
			return row ~= nil and row.offers_mission == true and not table.find(off, "mcp__zeron__plan") and not table.find(off, "mcp__zeron__delegate_task")
		end
		local s = open()
		return s ~= nil and (s :: any).mission_offered == true
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
		text = shared.text,
		agent = agent,
		target = invocation_target,
		-- (A made chat's files are on its host: the device goes too, or this
		-- Mac's engine is asked about a chat it doesn't hold.)
		files = function(): any
			local id = session_id:peek()
			if id then
				local s = store.session:peek()
				return { chat = id, device = if s and s.id == id then s.device else nil }
			end
			local p = route.project:peek()
			return if p then { space = p.id, device = p.device } else nil
		end,
		in_chat = function(): boolean
			return session_id:get() ~= nil
		end,
		run = function(name: string)
			ZeronCommands.run(name, session_id:peek())
		end,
		-- (`@` lists what the draft carries, and the other chats.)
		cargo = function(): { any }
			return shared.cargo:get()
		end,
		chat = function(): string?
			return session_id:get()
		end,
	}

	-- A queued message being edited in the composer: which, and the draft
	-- it set aside (put back when the edit ends).
	local editing = ui.state("chat_editing", nil :: { item: string, chat: string?, aside: string, aside_cargo: { any }? }?)
	local parked = ui.shared("chat.composer.edits", {} :: { [string]: any })
	local edits = Asks.queue_edits {
		id = function(): string?
			return session_id:peek()
		end,
		text = shared.text,
		editing = editing,
		focus = function()
			shared.focus:add(1)
		end,
		say = function(tone: string, key: string)
			own_notice:set { tone = tone, key = key, dismissable = true }
		end,
		cargo = function(): { any }
			return shared.cargo:peek()
		end,
		set_cargo = function(list: { any })
			shared.cargo:set(list)
		end,
		refs = function(): string
			return shared.refs_key()
		end,
	}
	local end_edit = edits.finish
	-- Moving to another chat: the draft shown is put away under the chat
	-- left and the next chat's own comes out (shared.show), as Zeron's
	-- composer swaps drafts on navigation. An edit under way goes with its
	-- chat (its words in that draft, the host holding the message), and is
	-- under way again as that chat shows.
	-- (Only a real move: a reload, the same chat shown, keeps the edit.)
	ui.effect(function()
		local key = session_id:get() or route.draft:get()
		local was = shared.showing()
		if was == key then
			return
		end
		ui.batch(function()
			local all = table.clone(parked:peek())
			all[was] = editing:peek()
			local back = all[key]
			all[key] = nil
			parked:set(all)
			shared.show(key)
			editing:set(back)
			if back then
				TRAY:set("queue")
			end
		end)
	end)

	-- Arriving on a chat (another session or a new chat shown: a sidebar or
	-- Activity row, Back or Forward, ⌘N, the palette; a split's pane taking
	-- the focus; back from Settings): the composer takes the keys. Once
	-- what brought it here is done (a palette closing
	-- puts focus back where it was first), so this comes last; a field
	-- that takes the keys after (a rename) has them. The window opening on
	-- a chat is an arrival too: what's typed from the launch on goes into
	-- the composer, with no click first (Zeron's composer has the keys as it
	-- opens; ademark's A1 types from the launch and timed out on every run
	-- while they were nowhere). (Kept: a reload isn't one.)
	local arrived = ui.state("chat_composer_arrived", "?")
	ui.effect(function()
		local now = route.current:get()
		local pane = route.chat:get().session
		local chat = now.name == "session" or now.name == "home"
		-- (A session's project is the chat's own, known once the engine
		-- lists it: its coming isn't a move, and it took the keys from
		-- whatever had them by then, a dialog opened at launch.)
		local project = if now.name == "home" then now.project or "" else ""
		local at = `{now.name}|{now.session or ""}|{project}|{pane or ""}`
		if arrived:peek() == at then
			return
		end
		-- (A tab strip's arrows keep the keys on the tab they chose.)
		if route.take_keys_kept() or not chat then
			arrived:set(at)
			return
		end
		-- Marked arrived as the keys are given, not before: the window's
		-- scripts loaded again meanwhile (the engine's plugins coming in as
		-- the window opens) drop this turn's callback, and the scripts made
		-- again give them instead.
		ui.after(0, function()
			if arrived:peek() == at then
				return
			end
			arrived:set(at)
			shared.focus:add(1)
		end)
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
	local function config_target(): any
		local claude = agent() == "claude-code"
		local id = session_id:peek()
		if id then
			local s = open()
			return { chat = id, device = if s then s.device else nil, claude = claude }
		end
		local plan = where.plan()
		return { space = if plan.project ~= Where.NO_PROJECT then plan.project else nil, device = plan.device, claude = claude }
	end
	local function open_config()
		local link = sessions.connection:peek()
		if link == "offline" or link == "reconnecting" then
			own_notice:set { tone = "warning", key = "chat.composer.engine_not_connected", dismissable = true }
			return
		end
		ConfigPanel.open(config_target())
	end
	ui.effect(function()
		local asked = ConfigPanel.reopen_ask:get()
		if asked then
			ConfigPanel.reopen_ask:set(nil)
			if asked.chat == session_id:peek() then
				open_config()
			end
		end
	end)
	ui.effect(function()
		local shown = ConfigPanel.shown:get()
		local id = session_id:get()
		local asking = asks.question:get() ~= nil or asks.approval:get() ~= nil
		if shown and (id ~= ConfigPanel.chat or asking) then
			ConfigPanel.close()
		end
	end)
	ui.effect(function()
		local claude = agent() == "claude-code"
		if ConfigPanel.shown:get() and ConfigPanel.claude:peek() ~= claude then
			ConfigPanel.retarget(config_target())
		end
	end)
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
		if AppCommands.argument(draft, "config") == "" and not resume then
			shared.text:set("")
			open_config()
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
		local parts = Outgoing.parts(if resume then "" else typed, text, carried, Pages.open(session_id:peek(), store.messages:peek(), carried))
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
			-- its own agent (another's goes through the switch dialog); its
			-- agent's own default is never sent as a pick.
			local opened = open()
			if opened and (opened :: any).model == nil and not running:peek() and model:peek() ~= shared.resolve_model(opened, store.catalog:peek()).model then
				for _, m in store.catalog:peek().models do
					if m.value == model:peek() and m.harness == (opened :: any).harness then
						req.model = m.value
						req.effort = effort:peek()
						req.harness = m.harness
					end
				end
			end
			local queued_row: string? = nil
			local steered: string? = nil
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
					Pages.unsteer(id, steered)
					-- (The session's line says why, when the engine did.)
					local s = store.session:peek()
					if not (s and s.notice) then
						failure_notice(e)
					end
					shared.restore(text_out, carried)
				end)
			end
			-- Its machine can't take it yet: kept, in the queue, till it can.
			if machine.hold_reason() then
				Held.add(id, { id = store.new_id(), at = os.time(), text = words, req = req })
				return
			end
			if running:peek() then
				local s = open()
				-- Steered as it goes into a turn that takes it: no row to show.
				if not (req.steer_now and s and s.steers == true) then
					queued_row = asks.queue_add(text_out, if #attachments > 0 then attachments else nil)
				else
					steered = Pages.steer(id, text_out)
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
			-- (To the transcript at its foot: in a split, the focused pane's,
			-- which shows its lane's list, the prompt joining it there too.)
			local lane = foot_lane(id)
			local lists = { store.messages }
			if lane then
				table.insert(lists, Lanes.view(lane).messages)
			end
			T.send(function()
				for _, list in lists do
					if list:index_of("id", mid) == nil then
						list:push(if resume
							then Outgoing.resumed(mid)
							else {
								id = mid,
								role = "user",
								text = words,
								attachments = parts.shown,
								-- On its way from this moment: the line under it
								-- says so at once.
								waiting = "sending",
							})
					end
				end
				deliver(mid)
			end, mid, if lane then `lane.{lane}` else nil)
			return false
		end
		if undoable then
			undoable = nil
			moved_note:set(nil)
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
			mission = if mission_offered() then mission:peek() else nil,
			profile = composer_profiles.find(draft_profile:peek()) and draft_profile:peek(),
		}
		for _, m in store.catalog:peek().models do
			if m.value == req.model then
				req.harness = m.harness
			end
		end
		local plan = where.plan()
		-- (`why`: what to say instead, a Cloud chat's reason in words.)
		local failed = function(why: string?)
			own_notice:set { tone = "danger", text = why, key = if why then nil else "chat.composer.didnt_start", dismissable = true }
			shared.restore(text_out, carried)
		end
		-- On Cloud, as the shipping app starts one: the chat opens at once
		-- under the id it's made with, your message in it, sending, and its
		-- machine starting under that (a tool row, as the machine's own setup
		-- steps are); Cloud makes it meanwhile, and the message goes once it
		-- has. Refused, back to the new chat with the words and why.
		if plan.cloud then
			warm.stop()
			local chat_id = store.new_id()
			req.project = plan.project
			req.device = plan.device
			req.branch = plan.branch
			req.session = chat_id
			req.fresh = true
			req.message_id = store.new_id()
			req.worktree = plan.worktree
			T.sent_here(req.message_id :: string)
			own_notice:set(nil)
			store.creating(req)
			store.open(chat_id)
			route.open_session(chat_id)
			sessions
				.create({ id = chat_id, project = plan.project, branch = plan.branch, cloud = true, reach = reach:peek() })
				:after(function(made: any)
					local cwd = if type(made) == "table" then made.cwd else nil
					req.cwd = if type(cwd) == "string" and cwd ~= "" then cwd else nil
					-- (The next new chat starts from the default again.)
					reach:set(nil)
					store.created(chat_id)
					store.send(req):after(function() end, function(_e: any) end)
				end, function(e: any)
					local why = if type(e) == "table" and type(e.message) == "string" and string.find(e.message, " ", 1, true)
						then e.message
						else nil
					store.create_failed(chat_id)
					-- Still on it: back to the new chat, where the words and
					-- why are said. (Gone to another chat meanwhile, that
					-- chat's composer is left alone.)
					if route.current:peek().name == "session" and route.current:peek().session == chat_id then
						route.new_chat(plan.project)
						-- (Said once the new chat is back: a chat moving
						-- clears the composer's own line.)
						ui.after(0, function()
							failed(why)
						end)
					end
				end)
			return
		end
		-- (Only once making it takes a while: said at once, its line pushed a new chat's composer down as it sent.)
		local making = true
		ui.after(STARTING_AFTER, function()
			if making then
				own_notice:set { key = "chat.composer.starting", spinner = true }
			end
		end)
		-- Made where the project is (or on the device chosen), then its first
		-- message run there, as the shipping app starts one (the send sets
		-- its config: the model, effort, access and plan chosen here): in
		-- the project's folder, the worktree its ref is checked out in, or a
		-- new worktree off that ref (screens/chat/where's plan). On Cloud, made
		-- there: the run goes in its machine's checkout, named as it's made.
		req.project = plan.project
		req.device = plan.device
		req.branch = plan.branch
		-- (The id its agent was started early for, if it was: the send goes
		-- into that agent.)
		local warmed = warm.take()
		sessions
			.create({
				id = warmed,
				project = plan.project,
				device = if plan.project then nil else plan.device,
				branch = plan.branch,
				cwd = if plan.worktree then nil else plan.cwd,
				cloud = plan.cloud,
			})
			:after(function(made: any)
				making = false
				local id = if plan.cloud and type(made) == "table" then made.id else made
				if type(id) ~= "string" then
					if warmed then
						store.unwarm(warmed)
					end
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
				route.open_session(id)
				store.send(req):after(function() end, function(_e: any) end)
			end, function(e: any)
				making = false
				-- (Never made: the agent started for it goes.)
				if warmed then
					store.unwarm(warmed)
				end
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
	-- (The catalog's device, a new chat's project's host included.)
	local accounts = shared.model_accounts(run_device)
	local function branch_fixed(): ds.ComposerFixed?
		local s = open()
		if not s then
			return nil
		end
		local path: string?, project: string? = nil, nil
		for _, p in sessions.projects:get() do
			if p.id == s.project then
				path, project = p.path, p.name
			end
		end
		local lb = live_branch:get()
		local name = if lb.key:sub(1, #s.id + 1) == `{s.id}|` then lb.name else (s :: any).branch
		local on: string? = nil
		for _, d in sessions.devices:get() do
			if d.id == (s :: any).device then
				on = d.name
			end
		end
		-- On Cloud: its machine's session (a side chat shares its parent's).
		local machine = if cloud.on_device((s :: any).device) then cloud.session_for(s.id, (s :: any).device) else nil
		return {
			device = on,
			name = name,
			cloud = cloud.on_device((s :: any).device),
			reach = if machine then machine.reach else nil,
			worktree = path ~= nil and s.cwd ~= nil and s.cwd ~= path,
			project = project,
			hue = if project and s.project then (route.look(s.project) or { hue = route.hue(s.project) }).hue else nil,
			src = if project and s.project then (route.look(s.project) or {}).src else nil,
			emoji = if project and s.project then (route.look(s.project) or {}).emoji else nil,
		}
	end
	local fx_o = ui.state("chat_composer_fx_o", 1)
	local fx_y = ui.state("chat_composer_fx_y", 0)
	fx_o:set(1)
	fx_y:set(0)
	local moving = false
	local note_n = 0
	local function say_moved(key: string, undo: boolean)
		note_n += 1
		local n = note_n
		moved_note:set { key = key, undo = undo, n = n }
		ui.after(5000, function()
			local now = moved_note:peek()
			if now and now.n == n then
				moved_note:set(nil)
				undoable = nil
			end
		end)
	end
	local function fx_to(o: number, y: number, ms: number)
		anime.animate(fx_o, { value = o, duration = ms, ease = "out(3)" })
		anime.animate(fx_y, { value = y, duration = ms, ease = "out(3)" })
	end
	local function move_draft(send_now: boolean): boolean
		local id = session_id:peek()
		local s: any = open()
		local d = if id then shared.draft_peek(id) else nil
		if moving or not id or not s or not d or editing:peek() or (string.find(d.text, "%S") == nil and #d.cargo == 0) then
			return false
		end
		moving = true
		local fixed = branch_fixed()
		local carry = {
			checkout = if fixed and fixed.worktree then "worktree" else "local",
			device = s.device,
			project = s.project or Where.NO_PROJECT,
			profile = if type(s.profile) == "table" then s.profile.id else nil,
			branch = if fixed then fixed.name else nil,
		}
		local before = { defaults = defaults:peek(), profile = draft_profile:peek() }
		local reduced = motion.reduced_now()
		local ms = if reduced then 120 else 320
		fx_to(0, if reduced then 0 else -12, ms)
		ui.after(ms, function()
			carry_next = carry
			-- (A draft written in on the new-chat page stays: `new_chat` opens a fresh one for this.)
			route.new_chat(s.project)
			local to = route.draft:peek()
			local wholes = shared.move_draft(id, to)
			-- (The route's project lands on arrival and a project change clears the branch: each is set a tick later.)
			ui.after(0, function()
				if new_chat() and project:peek() ~= carry.project then
					project:set(carry.project)
				end
				ui.after(0, function()
					if new_chat() and carry.branch and branch:peek() ~= carry.branch then
						branch:set(carry.branch)
					end
					fx_y:set(if reduced then 0 else 16)
					fx_to(1, 0, ms)
					moving = false
					shared.focus:add(1)
					if send_now then
						send(shared.text:peek())
						local left = shared.draft_peek(to)
						if left.text == "" and #left.cargo == 0 then
							say_moved("chat.composer.sent_new", false)
						end
						return
					end
					undoable = { from = id, to = to, wholes = wholes, before = before }
					say_moved("chat.composer.draft_moved", true)
				end)
			end)
		end)
		return true
	end
	draft_mover = move_draft
	local function undo_move()
		local u = undoable
		undoable = nil
		moved_note:set(nil)
		if not u then
			return
		end
		ui.batch(function()
			shared.put_whole(u.to, u.wholes.to)
			shared.put_whole(u.from, u.wholes.from)
			defaults:set(u.before.defaults)
			draft_profile:set(u.before.profile)
			route.open_session(u.from)
		end)
		shared.focus:add(1)
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
	local WINDOW: { [string]: string } = { Session = "5h", ["5h"] = "5h", Week = "weekly", Weekly = "weekly", Day = "daily", Daily = "daily" }
	local function exhausted_window(s: any): string?
		for _, a in accounts.accounts(s.harness or "", s.model or "") or {} do
			if a.active then
				for _, w in a.windows or {} do
					if w.used >= 1 then
						return WINDOW[w.label]
					end
				end
				return nil
			end
		end
		return nil
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
			local rest = if l.compacts
				then ui.t("chat.composer.limit.waiting_compacts", { name = name, at = at })
				else ui.t("chat.composer.limit.waiting", { name = name, at = at })
			return { waiting = true, rest = rest:get(), actions = actions }
		end
		if at then
			table.insert(actions, { label = ui.t("chat.composer.limit.wait"):get(), on_click = if l.to then act("wait", "arm") else act("arm") })
		end
		local window = exhausted_window(s)
		return {
			waiting = false,
			lead = if window
				then ui.t("chat.composer.limit.reached_window", { name = name, window = ui.t(`chat.composer.limit.window.{window}`):get() }):get()
				else ui.t("chat.composer.limit.reached", { name = name }):get(),
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
	-- How far the pill row's pills reach, and how far it stands raised over the deck's pill (the transcript keeps the room).
	local pills_reach = ui.state("chat_pills_reach", 0)
	local lifted = require("@app/screens/chat/dock").lift("page")

	local dictation = Dictation.bind(shared.text, function(): string
		return `{session_id:get() or route.draft:get()}|{if editing:get() then editing:get().item else ""}|{route.current:get().name}`
	end, function(): boolean
		return asks.question:get() ~= nil or asks.approval:get() ~= nil or route.current:get().name == "settings" or route.current:get().name == "plugin"
	end, "main")
	local dock = ui.ref()
	local function config_height(): number
		local _ = dock.height
		local ok, size = pcall(function(): any
			return ui.window.size:get()
		end)
		local tall = if ok and type(size) == "table" and type(size.height) == "number" then size.height else 800
		local top = tall
		local el = dock:peek()
		local placed, r = pcall(ui.window_rect, el)
		if el and placed and r then
			top = r.y
		end
		local hero = new_chat()
		local room = if hero then top - 44 - 16 else top - 52 - 44
		return math.max(if hero then 140 else 180, math.min(room, math.clamp(tall * 0.62, 300, 620)))
	end
	local function config_chips(): any
		local models, efforts = {}, {}
		for _, m in store.catalog:get().models do
			if m.harness == "claude-code" then
				table.insert(models, { value = m.value, label = m.label })
				if m.value == model:get() then
					for _, l in m.efforts or {} do
						table.insert(efforts, l.value)
					end
				end
			end
		end
		return { models = models, model = model:get(), efforts = efforts, effort = effort:get(), access = access:get() }
	end
	local function config_chip(chip: string, value: string)
		if chip == "model" then
			if not take_model(value) then
				model:set(value)
			end
		elseif chip == "effort" then
			effort:set(value)
		elseif chip == "access" then
			access:set(value)
		end
	end
	local function composer(): Node
		return ds.Composer {
			dictation = dictation,
			-- Its measures, fold and open tray kept across a reload, so it
			-- is laid out as it was from the first frame.
			keep = "chat",
			toolbar_id = "composer-toolbar",
			drop_ring = function(): boolean
				return shared.mentioning_over("composer")
			end,
			ring = function(): number
				return Branch.ring:get()
			end,
			open_model = shared.open_model,
			step_effort = shared.step_effort,
			model_open = model_open,
			model_committed = function(): boolean
				return session_id:get() ~= nil
			end,
			model_take = take_model,
			refs = function(): string
				return shared.refs_key()
			end,
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
			-- ("Jump to latest" of the transcript at its foot, while it
			-- shows: the trays' pill moves aside. In a split, the focused
			-- pane's: shared's `at_foot`.)
			aside = function(): number?
				return shared.pill_width_of("jump", shared.at_foot:get()):get()
			end,
			on_pill_width = shared.pill_teller("waiting", "window"),
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
			access_modes = function(): { string }?
				local row = harness_row()
				local modes = if row then row.access_modes else nil
				if modes == nil then
					return nil
				end
				return if #modes > 0 then modes else { "full" }
			end,
			access_auto = function(): boolean
				return store.offers_auto(model:get())
			end,
			plan = plan,
			mission = {
				value = mission,
				defaults = function(): any
					return prefs.mission_defaults:peek()
				end,
				shown = mission_offered,
				planned = function(): boolean
					local s = open()
					return s ~= nil and (s :: any).planned == true
				end,
			},
			prediction = function(): string?
				local s = open()
				if not s or editing:get() then
					return nil
				end
				return (s :: any).prediction
			end,
			resume = function(): boolean
				local s = open()
				return s ~= nil and (s :: any).stopped == true and not editing:get()
			end,
			on_resume = function()
				send(Outgoing.RESUME_PROMPT, { resume = true })
			end,
			limit = limit,
			pills = Pills.Row {
				bind = Pills.window_bind(),
				tasks = asks.tasks,
				waiting = true,
				picks = function(): string?
					return session_id:get()
				end,
				on_extent = function(w: number)
					if math.abs(pills_reach:peek() - w) >= 0.5 then
						pills_reach:set(w)
					end
				end,
			},
			pills_reach = function(): number
				return pills_reach:get()
			end,
			panel = ConfigPanel.Over {
				chips = config_chips,
				on_chip = config_chip,
				height = function(): number
					return config_height()
				end,
				on_closed = function()
					shared.focus:add(1)
				end,
			},
			on_lift = function(n: number)
				if lifted:peek() ~= n then
					lifted:set(n)
				end
			end,
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
			models_loading = function(): boolean
				return store.catalog:get().complete ~= true
			end,
			-- The session's model named by the host (its catalog's name, else
			-- its id), for the chip while the catalog shown doesn't list it.
			-- (Not a pick of another: that one's in the catalog. A session
			-- naming none: its agent's own default, by name.)
			model_label = function(): string?
				local s = open()
				if s and s.model == model:get() then
					return s.model_label
				end
				return nil
			end,
			-- A session naming no model runs its agent's own: the catalog's
			-- first of that agent (the host's `model_label` names it too),
			-- shown with its mark and reasoning, never sent as a pick.
			default_model = function(): string?
				local s = open()
				if not s or s.model ~= nil or s.harness == nil then
					return nil
				end
				local m = default_of(store.catalog:get().models, s.harness)
				return m and m.value
			end,
			models = function(): { ds.Model }
				return store.catalog:get().models
			end,
			accounts = accounts.accounts,
			account_group = accounts.account_group,
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
				return if open() then store.stats:get() else nil
			end,
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
				next = function(): any
					local s: any = open()
					return if s then s.next_compaction or nil else nil
				end,
				marks = function(): any
					local s: any = open()
					return if s then s.context_marks or nil else nil
				end,
				on_compact = function()
					compact("")
				end,
				busy = function(): boolean
					local s: any = open()
					return s ~= nil and (s.compacting == true or (s.compaction_wait or nil) ~= nil)
				end,
			},
			footer_git = function(): boolean
				local s = open()
				for _, p in sessions.now:get().projects do
					if p.id == (if s then s.project else nil) then
						return p.git == true
					end
				end
				return false
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
			branch_fixed = branch_fixed,
			-- (Only ever the person's: the engine takes it from this app alone.)
			on_reach = function(r: string)
				local s = open()
				local machine = if s then cloud.session_for(s.id, (s :: any).device) else nil
				if machine then
					cloud.set_session_reach(machine.chat, r)
				end
			end,
			reach = reach,
			reach_default = cloud.reach,
			cargo = shared.cargo,
			on_remove_cargo = shared.remove_cargo,
			on_paste_text = cargo_props.on_paste_text,
			on_open_pasted = cargo_props.on_open_pasted,
			on_attach = cargo_props.on_attach,
			attach_items = function(): { any }
				local chat = session_id:peek()
				if not chat or not Pills.plan_hidden(chat) then
					return {}
				end
				return { { label = ui.t("ds.pills.mission.show"):get(), icon = "checklist", on_click = function()
					Pills.show_plan(chat)
				end } }
			end,
			-- (A drop on the pill: the chat's overlay goes with it.)
			on_files = cargo_props.on_files,
			on_paste_image = cargo_props.on_paste_image,
			drop_overlay = false,
			on_send = send,
			draft_moves = {
				shown = function(): boolean
					return session_id:get() ~= nil and editing:get() == nil
				end,
				move = function()
					move_draft(false)
				end,
				send_new = function()
					move_draft(true)
				end,
				keys = function(): { move: string, send_new: string }
					local ok, keys = pcall(require, "@app/screens/shell/keys")
					if not ok or type(keys) ~= "table" then
						return { move = "", send_new = "" }
					end
					return { move = (keys :: any).combo("chat.move_draft"), send_new = (keys :: any).combo("chat.send_new") }
				end,
			},
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
			queue = shown_queue,
			can_steer = asks.can_steer,
			-- (A held row is the hold's: it goes on its own, or is let go.)
			-- (A held row goes on its own; one refused, by its Send.)
			on_queue_send = function(row: string)
				local held, chat = Held.id_of(row), session_id:peek()
				if CloudMachine.outbox_id(row) then
					return
				elseif not held then
					asks.on_queue_send(row)
				elseif chat and not machine.hold_reason() then
					release(chat, held)
				end
			end,
			on_queue_steer_all = asks.on_queue_steer_all,
			queue_held = function(): boolean
				return #asks.engine_queue:get() > 0
			end,
			queue_pictures = queue_pictures,
			on_dismiss_todos = asks.on_dismiss_todos,
			-- (A refused held row's Edit: its words back in the composer,
			-- after what's there.)
			on_queue_edit = function(row: string)
				local held, chat = Held.id_of(row), session_id:peek()
				if CloudMachine.outbox_id(row) then
					return
				elseif not held then
					edits.on_queue_edit(row)
					return
				end
				local e = if chat then Held.remove(chat, held) else nil
				if e then
					local words, draft = e.req.text or e.text, shared.text:peek()
					shared.text:set(if draft == "" then words else `{draft}\n\n{words}`)
				end
			end,
			on_queue_save = edits.on_queue_save,
			on_queue_cancel = edits.on_queue_cancel,
			on_queue_remove = function(row: string)
				local held, chat = Held.id_of(row), session_id:peek()
				if held then
					if chat then
						Held.remove(chat, held)
					end
					return
				end
				-- An outbox send: out for good, and its prompt with it. (Gone
				-- already, it went out or was taken out elsewhere; the list
				-- says so as it changes.)
				local sent = CloudMachine.outbox_id(row)
				if sent then
					if chat and can_remove_held() then
						local message = nil
						for _, w in outbox:peek() or {} do
							if w.id == sent then
								message = w.message
							end
						end
						store.remove_held(chat, sent, message):after(function() end, function()
							own_notice:set { tone = "danger", key = "chat.composer.didnt_remove", dismissable = true }
						end)
					end
					return
				end
				asks.on_queue_remove(row)
			end,
			-- (Held rows stay after the queue's own: only those move.)
			on_queue_reorder = function(from: number, to: number)
				local own = #queue:peek()
				if from <= own and to <= own then
					asks.on_queue_reorder(from, to)
				end
			end,
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
			-- (The only notice with an action: a sleeping machine's Wake.)
			on_notice_action = machine.wake,
			alerts = machine.alerts,
			on_alert_action = machine.on_alert,
			on_dismiss_alert = machine.dismiss,
			disabled = function(): boolean
				local s = store.session:get()
				local id = session_id:get()
				return id ~= nil and s ~= nil and s.id == id and (s.state == "missing" or s.state == "offline")
			end,
			focus = shared.focus,
			mentions = completions.lists,
			on_mention = function(item: any, trigger: string, token: string)
				completions.on_mention(item, trigger, token)
				local draft = string.match(shared.text:peek(), "^%s*(.-)%s*$") or ""
				if trigger == "/" and type(item) == "table" and item.value == "config" and AppCommands.argument(draft, "config") == "" then
					shared.text:set("")
					open_config()
				end
			end,
			mention_enabled = completions.enabled,
			mention_empty = completions.empty,
			mention_sections = completions.sections,
			draft_key = function(): any
				return session_id:get() or route.draft:get()
			end,
		}
	end

	local page = ui.ref()
	-- The page's height as last laid out, kept (a reload keeps state): a
	-- composer made again reads it until its own page is measured, so its
	-- spring starts where it rests instead of rising into place again.
	local page_h = ui.state("chat_page_h", 0)
	local composer_zone = shared.drop_zone("composer")
	-- There to open the model picker in (the shell's ⌘/), while it is:
	-- counted, not a flag. A page that swaps its composer mounts the new one
	-- before the old one's cleanup runs, and a flag the old one cleared was
	-- left false under the new one: ⌘/ did nothing (seen on Windows). Each
	-- composer takes back only the count it added.
	local counted = false
	ui.on_mount(function()
		if not counted then
			counted = true
			shared.composer_mounted()
		end
	end)
	-- (Counted: a composer made again mounts before the old one goes.)
	ui.on_cleanup(function()
		if counted then
			counted = false
			shared.composer_gone()
		end
	end)
	-- How tall a new chat's composer is with nothing in it (what it's placed
	-- by, so what's written in it doesn't move its top).
	local rest_tall = ui.state("chat_composer_rest_tall", nil :: number?)
	-- (A draft just deleted, it's empty while still easing back, and that
	-- height would move its top: taken once that's over.)
	local REST_SETTLE_MS = 500
	local had_draft = false
	local rest_settles_at = 0
	local rest_new = false
	ui.effect(function()
		local tall = dock.height or 0
		-- (Just come from a session, it's in that form yet, easing out of it: its height kept from before stands meanwhile.)
		local nc = new_chat()
		if nc and not rest_new and rest_tall:peek() ~= nil then
			rest_settles_at = ui.now() + REST_SETTLE_MS
		end
		rest_new = nc
		if shared.text:get() ~= "" or #shared.cargo:get() > 0 or next(shared.quotes:get()) ~= nil then
			had_draft = true
			return
		end
		if had_draft then
			had_draft = false
			rest_settles_at = ui.now() + REST_SETTLE_MS
		end
		if tall <= 0 or not nc then
			return
		end
		local function take()
			local d = dock:peek()
			local now = if d and (d.height or 0) > 0 then d.height else tall
			if new_chat() and not had_draft and math.abs((rest_tall:peek() or 0) - now) >= 0.5 then
				rest_tall:set(now)
			end
		end
		local left = rest_settles_at - ui.now()
		if left > 0 and rest_tall:peek() ~= nil then
			ui.after(left, take)
		else
			take()
		end
	end)
	-- (gpui's hero anchor: 0.5 of the room the pill leaves, 0.3 while the Home widgets are open.)
	local function raised_by(h: number, rest: number): number
		return composer_surface.lift:get() * 0.2 * math.max(0, h - rest)
	end
	-- Moving between a new chat and a session, either way and whatever moved
	-- it (a send, a new chat, a chat opened): it travels there from where it
	-- shows, its form easing with it, as gpui's dock does (composer_dock.rs:
	-- critically damped, ω = 12 / T, 0.42 s to a session and 0.47 s back).
	-- Another session puts it there at once, in its new form and size
	-- (`snap`). Motion reduced, it's there.
	local function travel_ease(docking: boolean): any
		local w = 12 / (if docking then 0.42 else 0.47)
		return anime.createSpring { stiffness = w * w, damping = 2 * w }
	end
	local GLIDE = { duration = 260, ease = "out(3)" }
	local TAKEOFF = composer_surface.TAKEOFF
	-- How far it shows from where it's laid out, on its way there.
	-- (Kept state, as this file's others: the composer is made in a plain
	-- function, not a component.)
	local place = ui.state("chat_composer_place", 0)
	local place_x = ui.state("chat_composer_place_x", 0)
	place:set(0)
	place_x:set(0)
	-- (Held where it is while its model picker is open: `model_open`.)
	local was_new: boolean? = nil
	-- (Its model picker open on a new chat: how far its top gives way so its
	-- chip stays, the picker hanging from it, as what's written grows.)
	local hold = ui.state("chat_composer_hold", 0)
	local function new_y(): number
		local tall = dock.height or 0
		if tall <= 0 then
			tall = shared.dock_h:get()
		end
		local rest = rest_tall:get() or tall
		local h = page.height or 0
		local at = Dock.hero_offset() - rest / 2 - raised_by(if h > 0 then h else page_h:get(), rest)
		-- (Only a draft that would run off the page's foot moves it.)
		if h > 0 then
			at = math.min(at, h / 2 - FOOT - tall)
		end
		return at + hold:get()
	end
	ui.effect(function()
		if not new_chat() then
			return
		end
		local _ = composer_surface.moved:get()
		local h = page.height or page_h:get()
		local pg, d, pill = page:peek(), dock:peek(), composer_surface.element
		local inner = dock.height or 0
		if d and pill and d.y and pill.y and pill.height then
			inner = pill.y + pill.height - d.y
		end
		local foot = (if pg and pg.y then pg.y else 0) + h / 2 + new_y() + inner
		if math.abs(composer_surface.foot:peek() - foot) >= 0.5 then
			composer_surface.foot:set(foot)
		end
	end)
	local held_tall: number? = nil
	local moving: { [any]: any } = {}
	local function travel(offset: State<number>, from: number, docking: boolean)
		if moving[offset] then
			moving[offset]:cancel()
			moving[offset] = nil
		end
		if motion.reduced_now() or math.abs(from) < 0.5 then
			offset:set(0)
			return
		end
		offset:set(from)
		moving[offset] = anime.animate(offset, { value = 0, ease = travel_ease(docking), delay = if docking then TAKEOFF else nil })
	end
	ui.on_cleanup(function()
		for _, run in moving do
			run:cancel()
		end
		composer_surface.hero:set(1)
		composer_surface.thread:set(1)
	end)
	-- What the page it arrives on shows of its own, staged on the same clock
	-- (gpui's `Visuals::advance`): a session's transcript over 0.20 to 0.65
	-- of it, a new chat's words and widgets over 0.50 to 0.95.
	local function arrive(docking: boolean)
		local shows = if docking then composer_surface.thread else composer_surface.hero
		if moving[shows] then
			moving[shows]:cancel()
			moving[shows] = nil
		end
		if motion.reduced_now() then
			shows:set(1)
			return
		end
		local clock = if docking then 420 else 470
		local from, to = if docking then 0.20 else 0.50, if docking then 0.65 else 0.95
		shows:set(0)
		moving[shows] = anime.animate(shows, { value = 1, delay = from * clock + (if docking then TAKEOFF else 0), duration = (to - from) * clock, ease = "inOut(2)" })
	end
	-- (Its page's middle as it moved, and when: the panel shutting with a session moves it, a layout later.)
	local middle_was: { at: number, x: number, docking: boolean }? = nil
	local function middle(): number?
		local pg = page:peek()
		return if pg and pg.x and pg.width then pg.x + pg.width / 2 else nil
	end
	-- (Settings and a plugin's page hide it over the chat they were opened from: under them it's there at once.)
	local was_on: string? = nil
	ui.effect(function()
		local nc = new_chat()
		local on = route.current:get().name
		local flipped = was_new ~= nil and nc ~= was_new
		local chats = (on == "home" or on == "session") and (was_on == "home" or was_on == "session")
		was_new, was_on = nc, on
		form_new:set(nc)
		if flipped and not chats then
			snap:set(snap:peek() + 1)
			travel(place, 0, false)
			travel(place_x, 0, false)
		elseif flipped then
			local d, pg = dock:peek(), page:peek()
			local h = if pg and (pg.height or 0) > 0 then pg.height else page_h:peek()
			local tall = if d and (d.height or 0) > 0 then d.height else shared.dock_h:peek()
			local hero, docked = h / 2 + new_y(), h - FOOT - tall
			local shown = (if nc then docked else hero) + place:peek()
			travel(place, shown - (if nc then hero else docked), not nc)
			local x = middle()
			middle_was = if x then { at = ui.now(), x = x + place_x:peek(), docking = not nc } else nil
			arrive(not nc)
		end
		if not nc then
			held_tall = nil
			hold:set(0)
			return
		end
		local tall = dock.height or 0
		if model_open:get() then
			held_tall = held_tall or tall
			hold:set(held_tall - tall)
		elseif held_tall then
			held_tall = nil
			anime.animate(hold, { value = 0, duration = GLIDE.duration, ease = GLIDE.ease })
		end
	end)
	ui.effect(function()
		local _ = page.width
		local was, x = middle_was, middle()
		middle_was = nil
		if was and x and ui.now() - was.at < 100 then
			travel(place_x, was.x - x, was.docking)
		end
	end)
	-- (Moving to another chat, as the focus moves between a split's
	-- panes, is shown for something else too: its draft, and its size for
	-- it, there at once rather than growing or shrinking into it.)
	local chat_was: string? = route.chat:peek().session
	ui.effect(function()
		local now = route.chat:get().session
		if now ~= chat_was then
			local between = now ~= nil and chat_was ~= nil
			chat_was = now
			if between then
				snap:set(snap:peek() + 1)
			end
		end
	end)
	-- Over the whole pane, the pointer passing through it to the page
	-- (its wheel too) but for the composer itself.
	return ui.overlay {
		ref = page,
		-- Not seen over a plugin's page (the chat's is kept unseen under it),
		-- nor anything placed round the composer: the page takes the pointer.
		opacity = function(): number
			return if route.current:get().name == "plugin" then 0 else 1
		end,
		on_resize = function(size: { w: number, h: number })
			-- (Hidden, it has no height: not somewhere to move to, nor to glide back from.)
			if size.h >= 1 and math.abs(page_h:peek() - size.h) >= 1 then
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
			-- Mid-page on a new chat, docked on a session, travelling between
			-- them (`place`).
			id = "chat-composer-dock",
			x = function(): number
				return place_x:get()
			end,
			y = function(): number
				return (if new_chat() then new_y() else 0) + place:get()
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
				opacity = function(): number
					return fx_o:get()
				end,
				y = function(): number
					return fx_y:get()
				end,
				max_w = ui.spring(
					ui.computed(function(): number
						return prefs.conversation_width:get()
					end),
					DOCK
				),
				on_resize = function(size: { w: number, h: number })
					if size.h >= 1 and math.abs(shared.dock_h:peek() - size.h) > 0.01 then
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
				require("@app/screens/shell/arrival").Pill(),
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
					opacity = function(): number
						return composer_surface.hero:get()
					end,
					exit = motion.transition({ opacity = 0, duration = 105, ease = "inOut(2)" }),
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
		ui.presence(function(): string?
			local n = moved_note:get()
			return if n then tostring(n.n) else nil
		end, function(_: string): Node
			local n = moved_note:peek()
			return ui.row {
				absolute = true,
				left = 0,
				right = 0,
				top = 48,
				justify = "center",
				pointer_events = "none",
				ui.row(require("@ds/glass").style({
					id = "chat-moved-toast",
					pointer_events = "auto",
					items = "center",
					gap = "$space.3",
					pl = "$space.3",
					pr = "$space.2",
					py = "$space.2",
					ui.text { ui.t(if n then n.key else "chat.composer.draft_moved"), text_size = "$font.size.sm", color = "$text.primary", wrap = false },
					if n and n.undo
						then ds.Button { ui.t("chat.composer.undo_move"), tone = "secondary", size = "xs", on_click = undo_move }
						else nil,
				}, "popup")),
			}
		end, {
			enter = { opacity = { 0, 1 }, y = { 8, 0 }, duration = 180, ease = "outCubic" },
			exit = { opacity = 0, duration = 180 },
		}),
		switch_dialog,
		profile_dialog,
		-- (A sidebar session carried: dropped, a mention; production's words.)
		DropOverlay { shown = shared.carrying_session, id = "chat-drop-mention", label = ui.t("chat.composer.drop_to_mention") :: any },
	}
end

return Composer
