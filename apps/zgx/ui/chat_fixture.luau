-- The chat page on its own, with a made-up session and no engine: what
-- tests/chat_*.test.luau drive. The host's calls (screens/chat/store) are
-- stubbed to change this data as the engine would; `ui.command`s stand in
-- for what the engine does on its own (a reply streaming, a turn ending,
-- an approval asked). Here, beside app.luau, as a mounted script loads
-- modules from its own directory down.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
-- A picture whose read comes later (`kinds_image_lands`): stubbed before the page reads any.
local ReplyImages = require("@app/screens/chat/reply_images")
local image_waits: { (any) -> () } = {}
local host_read = ReplyImages.read
ReplyImages.read = function(target: any, path: string): any
	if path ~= "/work/atlas/late.png" then
		return host_read(target, path)
	end
	return {
		after = function(_self: any, ok: (any) -> (), _err: any)
			table.insert(image_waits, ok)
		end,
	}
end
local store = require("@app/screens/chat/store")
local route = require("@app/screens/shell/route")
local chat = require("@app/screens/chat")
local sessions = require("@account/sessions")
local files = require("@app/screens/files/api")
-- Cloud for the new chat's Work in (none until `cloud_stub.offer`).
require("./cloud_stub")

local EFFORT = {
	{ value = "low", label = "Low" },
	{ value = "medium", label = "Medium" },
	{ value = "high", label = "High" },
}
store.catalog:set {
	state = "ready",
	harnesses = { { id = "claude-code", label = "Claude", mark = "claude" } },
	models = {
		{ value = "opus", label = "Opus", harness = "claude-code", efforts = EFFORT, effort = "high" },
		{ value = "sonnet", label = "Sonnet", harness = "claude-code", efforts = EFFORT, effort = "medium" },
		-- A model configured in place (Devin Fusion's shape): its lead and
		-- sidekick are options.
		{
			value = "fusion",
			label = "Fusion",
			harness = "claude-code",
			efforts = EFFORT,
			effort = "high",
			fast = true,
			options = {
				{ id = "lead", label = "Lead", default = "fable", choices = { { id = "fable", label = "Fable 5.1" }, { id = "sol", label = "GPT-6 Sol" } } },
				{ id = "sidekick", label = "Sidekick", default = "swe-medium", choices = { { id = "swe-medium", label = "SWE-2 Medium" }, { id = "swe-high", label = "SWE-2 High" } } },
			},
		},
	},
}

-- Two sessions' transcripts: a long one and a short one.
local function long(n: number): { any }
	local out = {}
	for i = 1, n do
		table.insert(out, { id = `u{i}`, role = "user", text = `Question number {i}: how does part {i} work?` })
		table.insert(out, {
			id = `a{i}`,
			role = "assistant",
			text = `Part {i} works like this.\n\nIt reads its input, then it writes **its answer**, then it stops. That's the whole of part {i}.`,
			model = "Opus",
			-- (Each its own minute, so a lane can be told from another.)
			at = `Sep 8, 4:{10 + i} PM`,
			copy = `Part {i} works like this.`,
			tools = { { id = `t{i}`, kind = "read", verb = "Read", target = `part{i}.rs` } },
		})
	end
	return out
end
local TRANSCRIPTS: { [string]: { any } } = {
	s1 = long(30),
	s2 = { { id = "x1", role = "user", text = "Hello there" }, { id = "x2", role = "assistant", text = "Hi! What shall we work on?", model = "Opus" } },
}
local META: { [string]: any } = {
	s1 = { title = "Long chat", project = "atlas", model = "opus", effort = "high", access = "full", plan = false },
	s2 = { title = "Short chat", project = "atlas", model = "sonnet", effort = "medium", access = "edits", plan = false },
}

-- What the page asked of the host, in order (`calls`).
local said: { string } = {}
local calls = ui.state("calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end
-- (The composer's accounts: `agents.on_device` set by a test, what's asked of
-- the host noted.)
do
	local stub_agents = require("@account/agents") :: any
	stub_agents.watch_device = function(key: string)
		note(`watch_device:{key}`)
	end
	stub_agents.refresh_device = function(key: string)
		note(`refresh_device:{key}`)
	end
	stub_agents.switch_on = function(key: string, id: string, account: string): any
		note(`switch_on:{key}:{id}:{account}`)
		return { after = function(_: any, done: (any) -> ()) done(nil) end }
	end
end

local function publish(id: string, patch: { [string]: any }?)
	local s = table.clone(META[id] or {})
	s.id = id
	s.state = if META[id] then "ready" else "missing"
	for k, v in patch or {} do
		s[k] = v
	end
	META[id] = s
	store.session:set(s)
end

local stub = store :: any
-- The engine says something of the open session: `test_patch`'s fields.
local test_patch = ui.state("test_patch", {} :: { [string]: any })
ui.command {
	id = "patch_open",
	title = "Patch the open session",
	run = function()
		local s = store.session:peek()
		if s and s.id then
			publish(s.id, test_patch:peek())
		end
	end,
}
-- How the draft colours a fenced block (`test_fence`).
local test_fence = ui.state("test_fence", "")
ui.command {
	id = "fence_probe",
	title = "Colour a fence",
	run = function()
		local markdown = require("@ds/markdown_edit")
		local _, inside = markdown.highlight("```rust", 0)
		local spans, still = markdown.highlight("fn main() {}", inside)
		local said = {}
		for _, s in spans do
			table.insert(said, s[3])
			table.insert(said, if (s :: any).bg == "$zeron.code.wash" then "wash:yes" else "wash:no")
		end
		local _, closed = markdown.highlight("```", still)
		local _, unknown = markdown.highlight("```nosuchlanguage", 0)
		table.insert(said, `closed:{closed}`)
		table.insert(said, `unknown:{unknown}`)
		test_fence:set(table.concat(said, ","))
	end,
}
-- An agent with no plan mode (⇧Tab walks the window's controls there).
ui.command {
	id = "agent_without_plan",
	title = "An agent with no plan mode",
	run = function()
		local c = store.catalog:peek()
		local harnesses: { any } = {}
		for _, h in c.harnesses do
			local row = table.clone(h)
			row.supports_plan = false
			table.insert(harnesses, row)
		end
		store.catalog:set { state = "ready", harnesses = harnesses, models = c.models }
	end,
}
-- The open session with a dev server running in the background.
ui.command {
	id = "task_running",
	title = "A background process runs",
	run = function()
		local s = store.session:peek()
		if s and s.id then
			publish(s.id, { tasks = { { id = "bg1", kind = "process", name = "npm run dev", detail = ":5173", since = os.time() } } })
		end
	end,
}
-- The open session with subagents (the host says them, as it does from the
-- transcript's spawn chips): `subagents_running` has two working and one
-- done; opening one is noted (`open_subagent:<session>:<doc>`).
do
	local lanes = require("@app/screens/chat/lanes") :: any
	local open_subagent = lanes.open_subagent
	lanes.open_subagent = function(session: string, sub: any)
		note(`open_subagent:{session}:{sub.ref}`)
		open_subagent(session, sub)
	end
	local published = ui.shared("chat.subagents", {} :: { any })
	ui.command {
		id = "subagents_running",
		title = "Subagents run: two working, one done",
		run = function()
			published:set({
				{ ref = "d1", title = "scan the repo", status = "running" },
				{ ref = "d2", title = "read the docs", status = "running" },
				{ ref = "d3", title = "find the flaky test", status = "done" },
			})
		end,
	}
end
-- The open session, as if it ran on the box (a remote device that's up),
-- or on the old laptop (one gone quiet).
for _, d in { "box", "old" } do
	ui.command {
		id = `session_on_{d}`,
		title = `The open session runs on {d}`,
		run = function()
			local s = store.session:peek()
			if s and s.id then
				publish(s.id, { device = d })
			end
		end,
	}
end
-- The chat open, kept across a reload as the host keeps it (the fixture
-- runs again on one; the host's transcript doesn't start over).
local kept_open = ui.state("fixture_opened", nil :: string?)
local opened: string? = kept_open:peek()
stub.open = function(id: string?)
	-- (As the host: the chat open already, nothing changes.)
	if id == opened then
		return
	end
	opened = id
	kept_open:set(id)
	note(`open:{id or "none"}`)
	if not id then
		return
	end
	store.messages:set(TRANSCRIPTS[id] or {})
	store.draft:set { id = "", text = "" }
	publish(id)
end
-- A promise settled at once.
local function settled(value: any): any
	return {
		after = function(self: any, ok: (any) -> (), _err: any)
			ok(value)
		end,
	}
end
local queue_answers_held = ui.state("fixture_queue_held", false)
local queue_held: { { act: () -> any, ok: (any) -> (), err: (any) -> () } } = {}
local function queue_answer(act: () -> any): any
	if not queue_answers_held:peek() then
		return settled(act())
	end
	local p: any = {}
	function p.after(_self: any, ok: (any) -> (), err: (any) -> ())
		table.insert(queue_held, { act = act, ok = ok, err = err })
	end
	return p
end
ui.command {
	id = "queue_held",
	title = "The engine answers the queue later",
	run = function()
		queue_answers_held:set(true)
	end,
}
ui.command {
	id = "engine_answers",
	title = "The engine answers what the queue asked",
	run = function()
		local all = queue_held
		queue_held = {}
		for _, h in all do
			h.ok(h.act())
		end
	end,
}
ui.command {
	id = "engine_refuses",
	title = "The engine refuses what the queue asked",
	run = function()
		local all = queue_held
		queue_held = {}
		for _, h in all do
			h.err({ message = "failed" })
		end
	end,
}
local queued_n = 0
stub.send = function(req: any)
	if req.session and META[req.session] and META[req.session].running and not req.fresh then
		note(`send:{req.session}:{req.text}{if req.steer_now then "|steer_now" else ""}`)
		return queue_answer(function()
			queued_n += 1
			local id = `eq{queued_n}`
			local q = table.clone(META[req.session].queue or {})
			table.insert(q, { id = id, text = req.text })
			publish(req.session, { queue = q })
			return id
		end)
	end
	note(`send:{req.session or "new"}:{req.text}{if req.steer_now then "|steer_now" else ""}{if req.queued then "|queued" else ""}`)
	if req.profile then
		note(`send_profile:{req.profile}`)
	end
	for _, a in req.attachments or {} do
		note(`att:{a.path}|{if a.on_host then "host" else "upload"}`)
	end
	-- (An Appshot's: what the host writes after the words.)
	for _, a in req.appshots or {} do
		note(`appshot:{a.app}|{a.title or "-"}|{a.path}|{if a.text then "text" else "-"}`)
	end
	-- Sent with a pick (another agent, model or effort): the run takes it,
	-- and the session with it, as the host does once the run is sent.
	if req.session and not req.fresh and (req.model or req.effort or req.fast ~= nil) then
		note(`took:{req.session}:{req.harness or "-"}|{req.model or "-"}|{req.effort or "-"}`)
		local patch: { [string]: any } = {}
		for _, k in { "harness", "model", "effort", "fast" } do
			if req[k] ~= nil then
				patch[k] = req[k]
			end
		end
		publish(req.session, patch)
	end
	if req.fresh then
		note(`run:{req.cwd or "-"}|{if req.worktree then "worktree" else "local"}|{req.branch or "-"}`)
	end
	if req.fresh then
		TRANSCRIPTS[req.session] = { { id = req.message_id or `{req.session}u`, role = "user", text = req.text } }
		-- (As the host: a prompt to the chat open shows at once.)
		if opened == req.session then
			store.messages:set(TRANSCRIPTS[req.session])
		end
	end
	return settled(nil)
end
stub.retry = function(id: string, message: string)
	note(`retry:{id}:{message}`)
end
-- Each project's branches, as git where it lives would list them (the
-- box's asked of the box).
local REFS: { [string]: { any } } = {
	["/tmp/atlas"] = {
		{ name = "main", current = true },
		{ name = "fix/render-loop", current = false, worktree = "/tmp/atlas-wt/render-loop" },
		{ name = "release/2.4", current = false },
	},
	["/srv/borealis"] = {
		{ name = "trunk", current = true },
		{ name = "feature/sync", current = false },
	},
}
-- The clipboard's picture, written out (noted): a picture of the app's own
-- here, or none after `clipboard_empty`.
local CLIPBOARD_IMAGE = ui.state("fixture_clipboard_image", true)
stub.paste_image = function()
	note("paste_image")
	return settled(if CLIPBOARD_IMAGE:peek() then "assets/favicons/zgx.png" else nil)
end
ui.command {
	id = "clipboard_empty",
	title = "The clipboard holds no picture",
	run = function()
		CLIPBOARD_IMAGE:set(false)
	end,
}
-- The open session working in a worktree of Atlas, where the agent has
-- since switched to a branch of its own (the chat still names main).
REFS["/tmp/atlas-wt/pr"] = {
	{ name = "main", current = false },
	{ name = "zeron/readme-pr", current = true, worktree = "/tmp/atlas-wt/pr" },
}
ui.command {
	id = "session_in_worktree",
	title = "The open session works in a worktree, on its own branch",
	run = function()
		local s = store.session:peek()
		if s and s.id then
			publish(s.id, { cwd = "/tmp/atlas-wt/pr", branch = "main" })
		end
	end,
}
stub.set_work_tree = function(id: string, path: string?): any
	note(`move:{id}:{path or "main"}`)
	return settled(nil)
end
stub.refs = function(path: string, device: string?)
	note(`refs:{path}:{device or "here"}`)
	return settled(REFS[path] or {})
end
ui.command {
	id = "undelivered",
	title = "A send wasn't delivered",
	run = function()
		store.messages:push { id = "lost1", role = "user", text = "Lost on the way", waiting = "failed" }
	end,
}
stub.interrupt = function(id: string)
	note(`interrupt:{id}`)
end
stub.stop = function(id: string): any
	note(`interrupt:{id}`)
	return settled(nil)
end
-- A background process stopped (noted); `stop_fails` makes the next fail.
local STOP_FAILS = ui.state("fixture_stop_fails", false)
stub.stop_task = function(session: string, task: string): any
	note(`stop_task:{session}:{task}`)
	if STOP_FAILS:peek() then
		STOP_FAILS:set(false)
		return {
			after = function(_self: any, _ok: any, err: (any) -> ())
				err("failed")
			end,
		}
	end
	return settled(true)
end
ui.command {
	id = "stop_fails",
	title = "The next Stop fails",
	run = function()
		STOP_FAILS:set(true)
	end,
}
do
	local route_seen = ui.state("test_route_session", nil :: string?)
	ui.effect(function()
		local now = route.current:get()
		route_seen:set(if now.name == "session" then now.session else nil)
	end)
	local patch_given = ui.state("test_session_patch", nil :: any)
	ui.command {
		id = "patch_session",
		title = "The open session's fields change",
		run = function()
			local s = store.session:peek()
			if s and s.id then
				publish(s.id, patch_given:peek())
			end
		end,
	}
	local function mirror(state_name: string, shared_name: string, empty: any)
		local given = ui.state(state_name, nil :: any)
		local shared = ui.shared(shared_name, empty)
		ui.effect(function()
			local v = given:get()
			if v ~= nil then
				shared:set(v)
			end
		end)
	end
	mirror("test_subs", "chat.subagents", {})
	mirror("test_waiting", "chat.waiting", {})
	mirror("test_shell_output", "chat.shell_output", {})
	mirror("test_usage", "chat.roster_usage", {})
	;(require("@app/screens/chat/lanes") :: any).watch = function(slot: string, id: string?)
		note(`lane_watch:{slot}:{id or ""}`)
	end
	local host_gone = ui.state("test_host_gone", false)
	local answer_fails = ui.state("test_answer_fails", false)
	local function refused(): any
		return {
			after = function(_self: any, _ok: any, err: (any) -> ())
				err({ message = "offline" })
			end,
		}
	end
	local kids = ui.state("test_children", {} :: { [string]: any })
	local real_children = sessions.children
	;(sessions :: any).children = function(id: string?): { any }
		return kids:get()[id or ""] or real_children(id)
	end
	stub.cancel_delegate = function(session: string, task: string): any
		note(`cancel_delegate:{session}:{task}`)
		if host_gone:peek() then
			return nil
		end
		return settled(true)
	end
	stub.waiting_answer = function(args: any): any
		note(`waiting_answer:{args.id}:{args.request}:{args.decision or ""}:{table.concat(args.values or {}, "+")}:{args.text or ""}`)
		if host_gone:peek() then
			return nil
		end
		return if answer_fails:peek() then refused() else settled(nil)
	end
	stub.waiting_back = function(id: string, request: string): any
		note(`waiting_back:{id}:{request}`)
		return settled({ chosen = {}, text = "" })
	end
	stub.roster_watch = function(docs: { string })
		note(`roster_watch:{#docs}`)
	end
	stub.shell_watch = function(shells: { any })
		note(`shell_watch:{#shells}`)
	end
end
stub.configure = function(id: string, patch: any)
	local keys = {}
	for k, v in patch do
		table.insert(keys, `{k}={tostring(v)}`)
	end
	table.sort(keys)
	note(`configure:{id}:{table.concat(keys, ";")}`)
	publish(id, patch)
end
stub.approve = function(id: string, request: string, scope: string)
	note(`approve:{id}:{request}:{scope}`)
	publish(id, { ask = ui.null })
end
stub.deny = function(id: string, request: string)
	note(`deny:{id}:{request}`)
end
stub.answer = function() end
-- A queued message sent on (steered in): noted; `steer_fails` makes the
-- next one not go.
local STEER_FAILS = ui.state("fixture_steer_fails", false)
stub.queue = function(id: string, op: any, failed: (() -> ())?): any
	note(`queue:{id}:{op.op}:{op.item}`)
	if op.op == "send" and STEER_FAILS:peek() then
		STEER_FAILS:set(false)
		if failed then
			failed()
		end
		return {
			after = function(_self: any, _ok: any, err: ((any) -> ())?)
				if err then
					err({ message = "failed" })
				end
			end,
		}
	end
	return queue_answer(function()
		local q = {}
		for _, r in (META[id] or {}).queue or {} do
			if r.id ~= op.item then
				table.insert(q, r)
			end
		end
		publish(id, { queue = q })
		return nil
	end)
end
ui.command {
	id = "steer_fails",
	title = "The next steer doesn't go",
	run = function()
		STEER_FAILS:set(true)
	end,
}
stub.steer_all = function(id: string, items: { string }, _message_id: string): any
	note(`steer_all:{id}:{table.concat(items, ",")}`)
	return queue_answer(function()
		local q = {}
		for _, r in (META[id] or {}).queue or {} do
			if not table.find(items, r.id) then
				table.insert(q, r)
			end
		end
		publish(id, { queue = q })
		return nil
	end)
end
-- A queued message's files, as the host hands them to an edit (`q2`'s).
local QUEUED_FILES = {
	{ path = "/host/uploads/9e97a72f-shot.png", name = "shot.png", kind = "image", src = "/host/uploads/9e97a72f-shot.png", on_host = true },
	{ path = "/host/uploads/ea958e1e-note.txt", name = "note.txt", kind = "file", on_host = true },
}
-- One queued message, editable here.
stub.queue_edit = function(id: string, item: string): any
	note(`queue_edit:{id}:{item}`)
	local s = table.clone(META[id] or {})
	local q = table.clone(s.queue or {})
	for i, row in q do
		if row.id == item then
			q[i] = { id = row.id, text = row.text, editing = true }
		end
	end
	publish(id, { queue = q })
	if item == "q2" then
		return settled({ text = "Queued with files", attachments = QUEUED_FILES })
	end
	return settled({ text = "Queued words" })
end
stub.queue_finish = function(id: string, item: string, action: string, text: string?, attachments: { any }?): any
	local files = ""
	if attachments then
		local names = {}
		for _, a in attachments do
			table.insert(names, `{a.path}{if a.on_host then "@host" else ""}`)
		end
		files = `:files={table.concat(names, ",")}`
	end
	note(`queue_finish:{id}:{item}:{action}:{text or ""}{files}`)
	publish(id, { queue = { { id = item, text = if action == "commit" then text else "Queued words" } } })
	return settled(nil)
end
ui.command {
	id = "queue_with_files",
	title = "A message with files waits in the queue",
	run = function()
		if opened then
			publish(opened, { queue = { { id = "q2", text = "Queued with files" } } })
		end
	end,
}
ui.command {
	id = "queue_one",
	title = "A message waits in the queue",
	run = function()
		if opened then
			publish(opened, { queue = { { id = "q1", text = "Queued words" } } })
		end
	end,
}
stub.load_catalog = function() end
-- (The device whose catalog the composer reads, noted as it changes.)
stub.catalog_on = function(device: string?)
	note(`catalog_on:{device or "here"}`)
end

-- The session's files, searched for `@`.
local FILES = { "src/chat.rs", "src/main.rs", "ui/screens/chat/composer.luau", "README.md" }
local files_stub = files :: any
files_stub.search = function(target: any, query: string): any
	note(`search:{target.chat or target.space or "?"}:{query}`)
	local found = {}
	for _, path in FILES do
		if query == "" or string.find(string.lower(path), string.lower(query), 1, true) then
			table.insert(found, { path = path, dir = false })
		end
	end
	return settled(found)
end

-- The agent's `/` commands and `$` skills, as the host lists them
-- (screens/chat/invocations), each read noted.
local Invocations = require("@app/screens/chat/invocations")
local invocations_stub = Invocations :: any
invocations_stub.read = function(harness: string, target: any): any
	note(`invocations:{harness}:{target.chat or target.space or "?"}`)
	return settled({
		rows = {
			{ kind = "command", name = "review", label = "/review", detail = "Review the diff · <branch>", link = "[/review](zeron-invoke:7b226b696e64223a22636f6d6d616e64222c226e616d65223a22726576696577227d)" },
			{ kind = "command", name = "compact", label = "/compact", detail = "Summarize the conversation", link = "[/compact](zeron-invoke:7b7d)" },
			-- (The agent's own `/new`: Zeron's is `/zeron:new` beside it.)
			{ kind = "command", name = "new", label = "/new", detail = "The agent's new", link = "[/new](zeron-invoke:7b7d)" },
			{ kind = "skill", name = "code-review", label = "Code Review", detail = "Careful review — /s/code-review/SKILL.md", link = "[$code-review](zeron-invoke:7b7d)", path = "/s/code-review/SKILL.md" },
		},
	})
end

-- The engine's copy of the prompt just sent landing: the same message (its
-- id), a new table, as the host publishes it.
ui.command {
	id = "engine_copy",
	title = "The engine's copy of the prompt lands",
	run = function()
		local all = store.messages:peek()
		local list = table.clone(all)
		local last = list[#list]
		if last and last.role == "user" then
			-- (Still sending: the session's turn hasn't begun.)
			list[#list] = { id = last.id, role = "user", text = last.text, attachments = {}, waiting = "sending" }
			store.messages:set(list)
		end
	end,
}
-- The engine starts the turn: the newest prompt's line turns to working.
ui.command {
	id = "turn_starts",
	title = "The turn begins",
	run = function()
		local n = store.messages:len()
		local last = store.messages:peek()[n]
		if last and last.role == "user" then
			ui.batch(function()
				store.messages:set_key({ n, "waiting" }, "working")
				store.messages:set_key({ n, "since" }, ui.now() / 1000)
			end)
		end
	end,
}

-- The engine's side: a reply streaming into the open session, word by word.
local REPLY = "Here is a longer answer, written a few words at a time so the transcript grows in place while you read it. It keeps going for a while."
local shown = 0
ui.command {
	id = "escape_from_composer",
	title = "Escape reaches the store",
	run = function()
		store.escape()
	end,
}
ui.command {
	id = "start_reply",
	title = "A reply starts",
	run = function()
		if not opened then
			return
		end
		shown = 0
		-- The prompt's line gives way to the reply's own.
		local n = store.messages:len()
		local prompt = store.messages:peek()[n]
		if prompt and prompt.role == "user" and prompt.waiting then
			store.messages:set_key({ n, "waiting" }, ui.null)
		end
		store.messages:push {
			id = "live",
			role = "assistant",
			text = "",
			model = "Opus",
			streaming = true,
			started = ui.now() / 1000,
			tools = {},
			-- (A host sending its time early, as an older one did: no lane
			-- shows it until the reply is done.)
			at = "Sep 8, 5:00 PM",
			copy = "early",
		}
		store.draft:set { id = "live", text = "" }
		publish(opened, { running = true })
	end,
}
ui.command {
	id = "stream_words",
	title = "The reply writes a few words",
	run = function()
		local stop = string.find(REPLY, " ", shown + 12) or #REPLY
		shown = stop
		store.draft:set { id = "live", text = string.sub(REPLY, 1, shown) }
	end,
}
ui.command {
	id = "add_tool",
	title = "The reply runs a tool",
	run = function()
		local n = store.messages:len()
		local m = store.messages:peek()[n]
		if not m or m.role ~= "assistant" then
			return
		end
		local tools = table.clone(m.tools or {})
		table.insert(tools, { id = `lt{#tools + 1}`, kind = "exec" :: any, verb = "Ran", target = "cargo test" })
		store.messages:set_key({ n, "tools" }, tools)
	end,
}
ui.command {
	id = "end_reply",
	title = "The turn ends",
	run = function()
		local n = store.messages:len()
		if n == 0 then
			return
		end
		ui.batch(function()
			store.messages:set_key({ n, "text" }, store.draft:peek().text)
			store.messages:set_key({ n, "streaming" }, false)
			store.draft:set { id = "", text = "" }
		end)
		if opened then
			publish(opened, { running = false })
		end
	end,
}
ui.command {
	id = "ask_approval",
	title = "The agent asks to run a command",
	run = function()
		if opened then
			publish(opened, {
				ask = {
					request = "r1",
					kind = "approval",
					approval = { title = "Run this command?", command = "rm -rf build", cwd = "~/atlas" },
				},
			})
		end
	end,
}

-- Messages from other chats, as the host gives them: one from a chat that's
-- listed, one from a chat that isn't, one with a long title (tests/chat_sender).
ui.command {
	id = "messages_from_chats",
	title = "Other chats send messages",
	run = function()
		store.messages:push { id = "from1", role = "user", text = "The layout pass is done.", at = "Sep 8, 5:00 PM", copy = "The layout pass is done.", sender = { chat = "s2", title = "Old title" } }
		store.messages:push { id = "from2", role = "user", text = "Waiting on you.", at = "Sep 8, 5:01 PM", copy = "Waiting on you.", sender = { chat = "gone", title = "Gone chat" } }
		store.messages:push { id = "from3", role = "user", text = "And a word from far away.", at = "Sep 8, 5:02 PM", copy = "And a word from far away.", sender = { chat = "gone2", title = string.rep("A very long chat title ", 12) } }
	end,
}

-- (The engine's mock can't make most of these rows.)
local THREAD_NOW = os.time()
local function thread_children(): { any }
	return {
		{ id = "th-1", parent = "s1", title = "Write the release notes", kind = "agent", created_by = "s1", device = "mac", mark = "claude", agent = "claude", harness = "claude-code", created = THREAD_NOW - 30, updated = THREAD_NOW - 30 },
		{ id = "dg-run", parent = "s1", title = "Port the parser", kind = "agent", delegation = "running", delegated_by = "s1", harness = "codex", model = "gpt-6.1", mark = "openai", agent = "openai", device = "mac", delegated_at = THREAD_NOW - 90, created = THREAD_NOW - 90, updated = THREAD_NOW - 10, route = { source = "fell_back", short = "Claude quota", resets_at = (THREAD_NOW + 3600) * 1000, role = "builder" } },
		{ id = "dg-done", parent = "s1", title = "Review the diff", kind = "agent", delegation = "completed", delegated_by = "s1", harness = "claude-code", model = "opus", mark = "claude", agent = "claude", device = "mac", delegated_at = THREAD_NOW - 400, delegated_end = THREAD_NOW - 277, created = THREAD_NOW - 400, updated = THREAD_NOW - 277, route = { source = "routed", short = "", role = "reviewer" } },
		{ id = "dg-lim", parent = "s1", title = "Run the benchmarks", kind = "agent", delegation = "running", delegated_by = "s1", harness = "claude-code", model = "sonnet", mark = "claude", agent = "claude", device = "mac", delegated_at = THREAD_NOW - 50, created = THREAD_NOW - 50, updated = THREAD_NOW - 20 },
		{ id = "side-1", parent = "s1", title = "Side question", kind = "side_chat", device = "mac", created = THREAD_NOW - 600, updated = THREAD_NOW - 600 },
		{ id = "fork-1", parent = "s1", title = "Try the other way", kind = "fork", device = "mac", created = THREAD_NOW - 700, updated = THREAD_NOW - 700 },
	}
end
local function thread_transcript(): { any }
	return {
		{ id = "tu1", role = "user", text = "Split the work", at = "Sep 8, 4:00 PM", copy = "Split the work" },
		{
			id = "ta1",
			role = "assistant",
			text = "Done.",
			at = "Sep 8, 4:05 PM",
			copy = "Done.",
			blocks = {
				{ id = "g:t1", kind = "tools", summary = "Called 1 tool", tools = { { id = "t1", kind = "thread", verb = "Started thread", target = "Write the release notes", thread = { ref = "th-1", title = "Write the release notes" } } } },
				{
					id = "g:t2",
					kind = "tools",
					summary = "Called 3 tools",
					tools = {
						{ id = "t2", kind = "delegate", verb = "Delegated", target = "Port the parser", delegate = { ref = "dg-run", title = "Port the parser" } },
						{ id = "t3", kind = "delegate", verb = "Delegated", target = "Review the diff", delegate = { ref = "dg-done", title = "Review the diff" } },
						{ id = "t4", kind = "delegate", verb = "Delegated", target = "Run the benchmarks", delegate = { ref = "dg-lim", title = "Run the benchmarks" } },
					},
				},
				{
					id = "g:t5",
					kind = "tools",
					summary = "Ran 2 subagents",
					tools = {
						{ id = "t5", kind = "agent", verb = "Ran a subagent", target = "scan the repo", tag = "Opus 5.5", subagent = { ref = "s1--sub--1", title = "scan the repo", status = "running", agent = "claude" } },
						{ id = "t6", kind = "agent", verb = "Ran a subagent", target = "read the docs", tag = "Explore", failed = true, subagent = { ref = "s1--sub--2", title = "read the docs", status = "failed" } },
					},
				},
				{ id = "t7.m0", kind = "sent", to = "Short chat", pending = true, failed = false },
				{ id = "t8.m0", kind = "sent", to = "s2", pending = false, failed = false },
				{ id = "t9.m0", kind = "sent", to = "nobody", pending = false, failed = true },
				{ id = "t10-native", kind = "native_route", title = "Summarize the logs", route = { source = "chosen_by_agent", short = "", role = "scout" } },
				{ id = "x1", kind = "text", text = "Done." },
			},
		},
		{ id = "tu2", role = "user", text = "The parser tests pass on my side.", from_chat = "s2", clock = "4:12 PM", at = "Sep 8, 4:12 PM", copy = "The parser tests pass on my side." },
		{ id = "tu3", role = "user", text = "Delegated task dg-done (\"Review the diff\") completed.", from_chat = "dg-done", wake = { "dg-done" }, at = "Sep 8, 4:13 PM" },
		{ id = "tu4", role = "user", text = "Continue with the next step.", from_chat = "s1", at = "Sep 8, 4:14 PM", copy = "Continue with the next step." },
	}
end
-- The limit's offer: switch to Codex, acting on its own in 20 s.
local thread_limits = ui.state("test_thread_limits", nil :: any)
local function seed_threads()
	local now = table.clone(sessions.now:peek() :: any)
	now.children = table.clone(now.children or {})
	now.children.s1 = thread_children()
	sessions.now:set(now)
	TRANSCRIPTS.s1 = thread_transcript()
	store.messages:set(thread_transcript())
	local cat = table.clone(store.catalog:peek())
	cat.models = table.clone(cat.models)
	table.insert(cat.models, { value = "gpt-6.1", label = "GPT-6.1", harness = "codex" })
	store.catalog:set(cat)
	local limits = thread_limits:peek() or { { chat = "dg-lim", resets_at = (THREAD_NOW + 7200) * 1000, auto = true, to = true, to_label = "GPT-6.1", target = "Codex · GPT-6.1 · High", to_mark = "openai", preselect = "switch", act_at = ui.now() + 20000, harness = "Claude Code" } }
	publish("s1", { limits = limits })
	local lanes = require("@app/screens/chat/lanes") :: any
	lanes.opener = function(session: string, lane: any)
		note(`lane:{session}:{lane.kind}:{lane.id}`)
	end
	lanes.open_ids = function(session: string): { [string]: boolean }
		return if session == "s1" then { ["dg-done"] = true, ["s1--sub--1"] = true } else {}
	end
end
ui.command { id = "threads_seed", title = "Threads across chats", run = seed_threads }
-- What the threads' rows sent the host (a limit's choice).
stub.limit = function(id: string, op: string): any
	note(`limit:{id}:{op}`)
	return settled(nil)
end
-- s1 working, and a chat on another device (not mentionable from here).
ui.command {
	id = "threads_listed",
	title = "s1 works; a chat on the build box",
	run = function()
		local now = table.clone(sessions.now:peek() :: any)
		local chats = {}
		for _, c in now.chats do
			local x = table.clone(c)
			if x.id == "s1" then
				x.status = "working"
				x.mark = "claude"
			end
			table.insert(chats, x)
		end
		table.insert(chats, { id = "far1", title = "Far away chat", project = "borealis", device = "box", updated = 0, created = 0 })
		now.chats = chats
		sessions.now:set(now)
	end,
}
ui.command { id = "thread_started_limited", title = "The started thread hits its limit", run = function()
	publish("s1", { limits = { { chat = "th-1", resets_at = (THREAD_NOW + 7200) * 1000, auto = true, to = false, harness = "Claude Code" } } })
end }
ui.command {
	id = "thread_limit_gone",
	title = "The delegate's limit is resolved",
	run = function()
		publish("s1", { limits = {} })
	end,
}
-- Every run over: the delegates finished, the running spawn done.
ui.command {
	id = "threads_settle",
	title = "The threads' runs end",
	run = function()
		local now = table.clone(sessions.now:peek() :: any)
		now.children = table.clone(now.children or {})
		local kids = {}
		for _, c in now.children.s1 or {} do
			local x = table.clone(c)
			if x.delegation == "running" then
				x.delegation = "completed"
				x.delegated_end = THREAD_NOW
			end
			table.insert(kids, x)
		end
		now.children.s1 = kids
		sessions.now:set(now)
		publish("s1", { limits = {} })
		local list = table.clone(store.messages:peek()) :: any
		for i, m in list do
			if m.id == "ta1" then
				local r = table.clone(m)
				r.blocks = table.clone(r.blocks)
				for j, b in r.blocks do
					if b.id == "g:t5" then
						local nb = table.clone(b)
						nb.tools = table.clone(nb.tools)
						local t = table.clone(nb.tools[1])
						t.subagent = { ref = "s1--sub--1", title = "scan the repo", status = "done", agent = "claude" }
						nb.tools[1] = t
						r.blocks[j] = nb
					end
				end
				list[i] = r
			end
		end
		store.messages:set(list)
	end,
}
ui.command {
	id = "thread_sent_lands",
	title = "The message on its way lands",
	run = function()
		local list = table.clone(store.messages:peek()) :: any
		for i, m in list do
			if m.id == "ta1" then
				local r = table.clone(m)
				r.blocks = table.clone(r.blocks)
				for j, b in r.blocks do
					if b.id == "t7.m0" then
						local nb = table.clone(b)
						nb.pending = false
						r.blocks[j] = nb
					end
				end
				list[i] = r
			end
		end
		store.messages:set(list)
	end,
}
-- The engine stamps a spawn's and a delegate's chat after their call starts.
local function thread_tools_bound(bound: boolean)
	local list = table.clone(store.messages:peek()) :: any
	for i, m in list do
		if m.id == "ta1" then
			local r = table.clone(m)
			r.blocks = table.clone(r.blocks)
			for j, b in r.blocks do
				if b.id == "g:t2" or b.id == "g:t5" then
					local nb = table.clone(b)
					nb.tools = table.clone(nb.tools)
					for k, t in nb.tools do
						local x = table.clone(t)
						if x.id == "t2" then
							x.delegate = if bound then { ref = "dg-run", title = "Port the parser" } else nil
						elseif x.id == "t4" then
							x.delegate = if bound then { ref = "dg-lim", title = "Run the benchmarks" } else nil
						elseif x.id == "t5" then
							x.subagent = if bound then { ref = "s1--sub--1", title = "scan the repo", status = "running", agent = "claude" } else nil
						end
						nb.tools[k] = x
					end
					r.blocks[j] = nb
				end
			end
			list[i] = r
		end
	end
	store.messages:set(list)
end
ui.command { id = "threads_seed_unbound", title = "Threads, the spawn and delegates not yet bound", run = function()
	seed_threads()
	thread_tools_bound(false)
end }
ui.command { id = "threads_bind", title = "Spawn and delegates bound", run = function()
	thread_tools_bound(true)
end }

-- The theme, for captures in both.
ui.command { id = "theme_light", title = "Light theme", run = function() ui.theme.follow_system(false); ui.theme.current:set("zeron-light") end }
ui.command { id = "theme_dark", title = "Dark theme", run = function() ui.theme.follow_system(false); ui.theme.current:set("zeron-dark") end }
ui.command { id = "opaque", title = "Opaque surfaces", run = function() require("@ds/material").frosted:set(false) end }
ui.command { id = "frosted", title = "Frosted surfaces", run = function() require("@ds/material").frosted:set(true) end }

-- A passage of a reply attached, as the transcript's selection Attach does.
-- Text selected in the transcript, as the host reads it (`read_selection`,
-- which a fixture has none of): a test sets `fake_selection` to `{ text, top,
-- bottom }`, each line `{ x, y, w, h }` in the window.
local SelectionMenu = require("@ds/selection_menu")
local fake_selection = ui.state("fake_selection", nil :: any)
SelectionMenu.read = function(): any
	local s = fake_selection:peek()
	return if type(s) == "table" and type(s.text) == "string" and string.find(s.text, "%S") then s else nil
end
SelectionMenu.clear = function()
	fake_selection:set(nil)
end
-- Two annotations staged on s2's rows, with their badges' anchors: one
-- with a comment of two lines, one with none.
ui.command {
	id = "annotations_seed",
	title = "Two annotations staged on s2",
	run = function()
		local a = chat.shared.add_annotation("s2", { text = "What shall we work on?", anchor = { key = "x2", dy = 0, x = 10, w = 150, lh = 20 } })
		chat.shared.set_comment_of("s2", a, "why this?\nand how")
		chat.shared.add_annotation("s2", { text = "Hello there", source = "docs/plan.md", anchor = { key = "x1", dy = 0, x = 10, w = 80, lh = 20 } })
	end,
}
-- A plain selection staged with no anchor, as an older draft holds one.
ui.command {
	id = "stage_selection",
	title = "A plain selection staged",
	run = function()
		chat.shared.cargo:push { id = "plain1", kind = "quote", name = "Layout", text = "Layout dominates on both machines." }
	end,
}
-- What's selected in the panel's terminal, staged as the terminal tab stages it.
ui.command {
	id = "stage_terminal",
	title = "A terminal selection staged",
	run = function()
		chat.shared.set_terminal_quote("cargo build\nerror[E0425]: cannot find value")
	end,
}
ui.command {
	id = "ask_question",
	title = "The agent asks a question",
	run = function()
		if opened then
			publish(opened, {
				ask = {
					request = "q1",
					kind = "question",
					question = {
						header = "Question",
						text = "Which branch should the fix go on?",
						options = {
							{ value = "main", label = "main", detail = "Straight onto the default branch." },
							{ value = "fix", label = "A new branch", detail = "fix/render-loop, opened as a PR." },
						},
					},
				},
			})
		end
	end,
}

-- The same question as the second of two pages (the first answered).
ui.command {
	id = "ask_question_page2",
	title = "The agent's question, on its second page",
	run = function()
		if opened then
			publish(opened, {
				ask = {
					request = "q2",
					kind = "question",
					question = {
						header = "Question",
						text = "And which reviewer?",
						page = 2,
						pages = 2,
						options = {
							{ value = "ada", label = "Ada", detail = "Knows the render loop." },
							{ value = "alan", label = "Alan", detail = "Knows the host." },
						},
					},
				},
			})
		end
	end,
}
-- Back a page (noted): what the first page had picked.
stub.question_back = function(id: string, request: string): any
	note(`question_back:{id}:{request}`)
	-- (The host shows the first page again.)
	if opened then
		publish(opened, {
			ask = {
				request = request,
				kind = "question",
				question = {
					header = "Question",
					text = "Which branch should the fix go on?",
					page = 1,
					pages = 2,
					options = {
						{ value = "main", label = "main", detail = "Straight onto the default branch." },
						{ value = "fix", label = "A new branch", detail = "fix/render-loop, opened as a PR." },
					},
				},
			},
		})
	end
	return settled({ "fix" })
end

-- The registry icons the app has heard of (src/agents.rs `agent_icons`):
-- Factory Droid's, as the registry draws it.
local ICONS = {
	-- (Factory Droid's icon as the registry serves it.)
	["acp.factory-droid"] = [==[<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 900 900" fill="none"><path fill="currentColor" d="M622.037 192.524a10.58 10.58 0 0 1-4.056-2.001 10.573 10.573 0 0 1-3.98-7.89 10.595 10.595 0 0 1 .804-4.452c20.214-49.194 29.134-88.555 14.74-105.033-38.123-43.716-191.004 43.219-239.75 72.663a10.596 10.596 0 0 1-10.841.041 10.576 10.576 0 0 1-4.396-5.07c-20.491-49.089-42.031-83.233-63.865-84.714-57.871-3.96-104.516 165.624-118.17 220.895a10.574 10.574 0 0 1-1.993 4.057 10.572 10.572 0 0 1-7.884 3.977 10.586 10.586 0 0 1-4.447-.802c-49.194-20.215-88.572-29.134-105.033-14.74-43.717 38.123 43.202 191.005 72.644 239.75a10.553 10.553 0 0 1 1.454 4.279 10.573 10.573 0 0 1-6.481 10.958c-49.072 20.491-83.216 42.031-84.715 63.864-3.944 57.871 165.624 104.516 220.913 118.171a10.566 10.566 0 0 1 5.658 3.601 10.57 10.57 0 0 1 2.355 6.281 10.58 10.58 0 0 1-.799 4.442c-20.214 49.194-29.134 88.572-14.739 105.033 38.121 43.717 191.021-43.202 239.767-72.644a10.566 10.566 0 0 1 15.238 5.027c20.489 49.072 42.013 83.216 63.862 84.715 57.871 3.944 104.516-165.624 118.153-220.913a10.603 10.603 0 0 1 9.895-8.021 10.566 10.566 0 0 1 4.449.808c49.193 20.214 88.553 29.116 105.032 14.738 43.718-38.121-43.219-191.022-72.663-239.767a10.602 10.602 0 0 1 1.326-12.655 10.604 10.604 0 0 1 3.703-2.582c49.089-20.49 83.233-42.031 84.714-63.863 3.96-57.871-165.624-104.516-220.895-118.153Zm-66.394-55.478c11.123 19.938-46.196 152.797-88.83 245.724a8.36 8.36 0 0 1-8.239 4.855 8.374 8.374 0 0 1-7.412-6.043c-17.219-60.42-36.899-131.411-57.957-191.675a10.557 10.557 0 0 1 4.924-12.759c52.586-28.721 142.568-66.86 157.514-40.102ZM303.635 153.49c21.953 6.233 75.365 140.709 110.92 236.564a8.364 8.364 0 0 1-2.394 9.249 8.364 8.364 0 0 1-9.504.978c-54.943-30.493-119.013-66.824-176.522-94.546a10.565 10.565 0 0 1-5.528-12.501c16.926-57.44 53.532-148.095 83.028-139.744ZM137.064 343.322c19.921-11.123 152.795 46.197 245.707 88.83a8.369 8.369 0 0 1-1.189 15.652c-60.401 17.219-131.411 36.899-191.675 57.957a10.552 10.552 0 0 1-12.742-4.925c-28.668-52.584-66.876-142.568-40.101-157.514Zm16.443 252.009c6.217-21.953 140.709-75.365 236.564-110.921a8.368 8.368 0 0 1 10.227 11.898c-30.511 54.945-66.842 119.014-94.563 176.507a10.548 10.548 0 0 1-5.229 5.075 10.544 10.544 0 0 1-7.271.468c-57.441-16.822-148.095-53.531-139.728-83.027ZM343.34 761.902c-11.14-19.922 46.197-152.796 88.829-245.707a8.383 8.383 0 0 1 5.713-4.66 8.38 8.38 0 0 1 7.182 1.664 8.371 8.371 0 0 1 2.758 4.184c17.217 60.403 36.898 131.412 57.957 191.675a10.558 10.558 0 0 1-4.942 12.743c-52.568 28.668-142.568 66.875-157.445 40.101h-.052Zm252.009-16.443c-21.971-6.216-75.383-140.709-110.939-236.564a8.37 8.37 0 0 1 11.916-10.228c54.926 30.494 119.014 66.842 176.506 94.563a10.538 10.538 0 0 1 5.527 12.502c-16.909 57.526-53.515 148.094-83.01 139.727Zm166.57-189.834c-19.938 11.141-152.796-46.197-245.724-88.83a8.367 8.367 0 0 1-2.993-12.892 8.378 8.378 0 0 1 4.182-2.759c60.419-17.218 131.41-36.899 191.675-57.958a10.577 10.577 0 0 1 12.758 4.943c28.652 52.568 66.86 142.569 40.102 157.496Zm-16.444-252.009c-6.232 21.971-140.709 75.383-236.562 110.94a8.371 8.371 0 0 1-10.229-11.917c30.495-54.926 66.825-119.012 94.547-176.505a10.555 10.555 0 0 1 12.5-5.528c57.441 16.909 148.096 53.516 139.744 83.01Z"/></svg>]==],
}
local agent_icons = ui.shared("agent_icons", {} :: { [string]: string })
agent_icons:set(ICONS)
-- (Not heard of yet, then heard of.)
ui.command { id = "forget_icons", title = "Forget the registry icons", run = function()
	agent_icons:set({})
end }
ui.command { id = "learn_icons", title = "Hear of the registry icons", run = function()
	agent_icons:set(ICONS)
end }

-- The shell's sessions: one project, two chats, this computer.
local function seed()
	local chats: { any } = {}
	for id, m in META do
		table.insert(chats, { id = id, title = m.title or id, project = m.project, device = "mac", updated = 0, created = 0, agent = m.agent })
	end
	table.sort(chats, function(a, b)
		return (a.id :: string) < (b.id :: string)
	end)
	sessions.now:set(({
		state = "ready",
		here = "mac",
		-- This computer; a box that's up (seen a moment ago); one gone quiet.
		devices = {
			{ id = "mac", name = "MacBook Pro", platform = "macos", here = true, online = true },
			-- (Its beat a day ahead: up however long the fixture runs.)
			{ id = "box", name = "Build box", platform = "linux", here = false, online = true, seen = ui.now() / 1000 + 86400 },
			{ id = "old", name = "Old laptop", platform = "macos", here = false, online = false, seen = ui.now() / 1000 - 3 * 3600 },
		},
		projects = {
			{ id = "atlas", name = "Atlas", device = "mac", path = "/tmp/atlas", git = true, github = "acme/atlas" },
			{ id = "notes", name = "Notes", device = "mac", path = "/tmp/notes", git = false },
			-- (Its folder the app's own assets: a picture in it loads.)
			{ id = "kit", name = "Kit", device = "mac", path = "assets", git = false },
			{ id = "borealis", name = "Borealis", device = "box", path = "/srv/borealis", git = true },
			{ id = "attic", name = "Attic", device = "old", path = "/Users/me/attic", git = true },
		},
		chats = chats,
		places = { pinned = {}, sections = {} },
		places_ready = true,
		archived = {},
	}) :: any)
end
seed()
-- This device's connection as the shell hears it (`connection`).
for _, c in { { "connection_reconnecting", "reconnecting" }, { "connection_offline", "offline" }, { "connection_back", nil } } do
	ui.command {
		id = c[1],
		title = `The connection: {c[2] or "back"}`,
		run = function()
			local now = table.clone(sessions.now:peek() :: any)
			now.connection = c[2]
			sessions.now:set(now)
		end,
	}
end
ui.command {
	id = "project_no_git",
	title = "The open chat's project has no git",
	run = function()
		local now = table.clone(sessions.now:peek() :: any)
		now.projects = table.clone(now.projects)
		for i, p in now.projects do
			if p.id == "atlas" then
				local without = table.clone(p)
				without.git = false
				now.projects[i] = without
			end
		end
		sessions.now:set(now)
	end,
}
ui.command {
	id = "stalled_words",
	title = "What a stalled chat's send says",
	run = function()
		local LinkNotice = require("@app/screens/chat/link_notice")
		local base = { here = "mac", devices = {}, stalled = { "far", "near" } }
		ui.state("test_stalled", ""):set(table.concat({
			tostring(LinkNotice.working_says(true, base, { id = "far", device = "box" })),
			tostring(LinkNotice.working_says(true, base, { id = "near", device = "mac" })),
			tostring(LinkNotice.working_says(true, base, { id = "other", device = "box" })),
		}, "|"))
	end,
}
for _, r in { { "rate_41", 41 }, { "rate_gone", nil } } do
	ui.command {
		id = r[1],
		title = `The live rate: {r[2] or "gone"}`,
		run = function()
			store.token_rate:set(r[2])
		end,
	}
end
ui.command {
	id = "stats_say",
	title = "The session's numbers",
	run = function()
		store.stats:set {
			turns = 3,
			steps = 7,
			llm = "13.4s",
			tool = "2.0s",
			tps = "137",
			priced = false,
			unpriced = 0,
			estimated = false,
			total = "0",
			total_n = 0,
			split = {},
		}
	end,
}
for _, w in { { "says_consulting", { consulting = true } }, { "says_starting", { machine = "starting" } }, { "says_waking", { machine = "waking" } }, { "says_nothing", { consulting = false, machine = nil } } } do
	ui.command {
		id = w[1],
		title = `The working row: {w[1]}`,
		run = function()
			local s = store.session:peek()
			if s and s.id then
				local next_s = table.clone(s) :: any
				next_s.consulting, next_s.machine = (w[2] :: any).consulting, (w[2] :: any).machine
				store.session:set(next_s)
			end
		end,
	}
end
local made_chats = 0
local sessions_stub = sessions :: any
sessions_stub.create = function(req: any)
	made_chats += 1
	local id = `new{made_chats}`
	note(`create:{req.project or "none"}:{req.config and req.config.model or ""}`)
	note(`made:{req.project or "-"}|{req.device or "-"}|{req.branch or "-"}|{req.cwd or "-"}{if req.cloud then "|cloud" else ""}`)
	META[id] = { title = "New chat", project = req.project, model = req.config and req.config.model }
	TRANSCRIPTS[id] = {}
	seed()
	-- On Cloud: made on a machine of its own, its checkout of the repository.
	if req.cloud then
		return settled({ id = id, cwd = "/home/user/atlas" })
	end
	return settled(id)
end
sessions_stub.archive = function(id: string)
	note(`archive:{id}`)
end

-- Where the shell is, moved as the sidebar would.
-- A session whose config has no model (made elsewhere, with none set).
ui.command {
	id = "model_less",
	title = "A session with no model",
	run = function()
		META.s3 = { title = "No model", project = "atlas", access = "full", plan = false }
		TRANSCRIPTS.s3 = { { id = "z1", role = "user", text = "Hi" } }
		seed()
	end,
}
-- Sessions holding only a handoff: from another chat (s5), and from another
-- agent, its seam (s6); and the open session's transcript not come yet.
ui.command {
	id = "handed_over",
	title = "A session holding only a handoff",
	run = function()
		META.s5 = { title = "Handed over", project = "atlas", model = "opus", effort = "high", access = "full", plan = false }
		TRANSCRIPTS.s5 = { { id = "h1", role = "system", line = "handoff", text = "", title = "Long chat" } }
		META.s6 = { title = "Moved over", project = "atlas", model = "opus", effort = "high", access = "full", plan = false }
		TRANSCRIPTS.s6 = { { id = "h2", role = "switch", text = "", from = "GPT-5.5", to = "Opus", handoff = true } }
		seed()
	end,
}
ui.command {
	id = "transcript_coming",
	title = "The open session's transcript not come yet",
	run = function()
		local s = store.session:peek()
		if s and s.id then
			store.messages:set({})
			publish(s.id, { state = "loading" })
		end
	end,
}
-- A session of long messages (each folds to five lines).
do
	local long_chat = {}
	for n = 1, 4 do
		local lines = {}
		for l = 1, 14 do
			table.insert(lines, `Message {n}, line {l}: a sentence long enough to fill the bubble's width.`)
		end
		table.insert(long_chat, { id = `l{n}`, role = "user", text = table.concat(lines, "\n") })
		table.insert(long_chat, { id = `r{n}`, role = "assistant", text = `Reply {n}.`, model = "Opus" })
	end
	META.s4 = { title = "Long messages", project = "atlas", model = "opus", effort = "high", access = "full", plan = false }
	TRANSCRIPTS.s4 = long_chat
	seed()
end
-- A reply in blocks, as the host lays one out: text, three tools, text,
-- then two tools still running (its open group, at the tail).
local function tool_of(id: string, kind: string, verb: string, target: string): any
	return { id = id, kind = kind, verb = verb, target = target }
end
local GROUPED = {
	{ id = "q1", role = "user", text = "Look into the failing test" },
	{
		id = "w1",
		role = "assistant",
		model = "Opus",
		streaming = true,
		text = "",
		blocks = {
			{ id = "p1", kind = "text", text = "First I'll look around the tests." },
			{ id = "g:x1", kind = "tools", summary = "Ran 2 commands · read 1 file", tools = {
				tool_of("x1", "exec", "Ran", "cargo test"),
				tool_of("x2", "exec", "Ran", "git status"),
				tool_of("x3", "read", "Read", "chat.rs"),
			} },
			{ id = "p2", kind = "text", text = "The second check fails. Reading the code next." },
			{ id = "g:x4", kind = "tools", summary = "Read 2 files", live = true, tools = {
				tool_of("x4", "read", "Read", "lib.rs"),
				tool_of("x5", "read", "Read", "main.rs"),
			} },
		},
	},
}
META.s5 = { title = "Grouped tools", project = "atlas", model = "opus", effort = "high", access = "full", plan = false }
TRANSCRIPTS.s5 = GROUPED
-- Settled replies and a move to another agent, as the shipping app's
-- reply-footer fixture has them: each message's time and Copy under it on
-- hover, the switch seam over the prompt that moved the chat.
local function line(id: string, role: string, minute: number, text: string, extra: any?): any
	local m: any = { id = id, role = role, text = text, at = `Sep 8, 1:{minute + 40} PM`, copy = text, model = "GPT-5.5" }
	if extra then
		for k, v in extra do
			m[k] = v
		end
	end
	return m
end
TRANSCRIPTS.s6 = {
	line("f0", "user", 0, "Profile the render loop and tell me where the frame time goes."),
	line("f1", "assistant", 1, "Layout is 61% of the frame and paint 22%. The rest is present and input.", {
		blocks = {
			{ id = "g:fr", kind = "tools", summary = "Thought process", tools = {
				-- (As the host gives a thought: its words to open to, as prose.)
				{
					id = "fr",
					kind = "thought",
					verb = "Thought process",
					call = "**Reading the frame**\n\nLayout runs twice a frame. Paint is cheap by comparison, and present is a fixed cost whatever the scene holds, so the frame's shape is set by layout alone.\n\n**Where the time goes**\n\nThe second layout pass comes from the sidebar measuring itself after the transcript has settled, so every frame pays for two trees. Moving the sidebar's measure ahead of the transcript's would take one pass out, and the paint cost follows the layout's size, so it would fall too.\n\nI'll profile the present step next, in case the swap chain is waiting on the compositor.",
					prose = true,
				},
			} },
			{ id = "ft", kind = "text", text = "Layout is 61% of the frame and paint 22%. The rest is present and input." },
		},
	}),
	{ id = "switch:1", role = "switch", text = "", from = "Claude Code", to = "Codex", from_mark = "claude", to_mark = "openai" },
	line("f2", "user", 2, "Carry on from here and draft the summary table."),
	line("f3", "assistant", 3, "Here is the summary. Layout dominates on both machines, so that is where the work should go first.\n\nPaint is second, and present is small everywhere."),
	line("f4", "user", 4, "Thanks, ship it."),
	line("f5", "assistant", 5, "Done. The table is in the release notes."),
}
META.s6 = { title = "Switched agents", project = "atlas", model = "sonnet", effort = "medium", access = "full", plan = false }
seed()
ui.command {
	id = "group_grows",
	title = "A third tool joins the open group",
	run = function()
		local n = store.messages:len()
		store.messages:set_key({ n, "blocks", 4, "tools", 3 }, tool_of("x6", "search", "Searched", "fold"))
		store.messages:set_key({ n, "blocks", 4, "summary" }, "Read 2 files · searched 1 time")
	end,
}
-- Tall replies, far taller than the list's estimate for a row not yet
-- measured (as a real agent's are): what a view put back after a reload
-- has to find its place among.
do
	local tall = {}
	for i = 1, 20 do
		local paragraphs = {}
		for k = 1, 10 do
			table.insert(paragraphs, `Reply {i}, paragraph {k}: the layout pass runs first, then paint, then present.`)
		end
		local text = table.concat(paragraphs, "\n\n")
		table.insert(tall, { id = `tu{i}`, role = "user", text = `Turn {i}: tell me more.`, at = `Sep 9, 3:{10 + i} PM`, copy = `Turn {i}: tell me more.` })
		table.insert(tall, {
			id = `ta{i}`,
			role = "assistant",
			text = text,
			model = "Opus",
			at = `Sep 9, 4:{10 + i} PM`,
			copy = text,
			blocks = { { id = "t", kind = "text", text = text } },
		})
	end
	META.s7 = { title = "Tall replies", project = "atlas", model = "opus", effort = "high", access = "full", plan = false }
	TRANSCRIPTS.s7 = tall
	seed()
end
-- The content of the shipping app's compare fixture (a reply's text, a
-- tool group and a code block, then a settled turn), for side by sides.
do
	local fix = "The second check fails: the engine drops before the write lands. The fix:\n\n```rust\nlet ack = engine.write(doc, change).await?;\nack.persisted().await?;\n```\n\nIt passes now, **200 runs** in a row."
	TRANSCRIPTS.s8 = {
		{ id = "cu0", role = "user", text = "Look into the failing test", at = "Sep 8, 1:40 PM", copy = "Look into the failing test" },
		{
			id = "ca0",
			role = "assistant",
			text = "First I'll look around the tests.\n\n" .. fix,
			model = "Opus",
			at = "Sep 8, 1:41 PM",
			copy = "First I'll look around the tests.\n\n" .. fix,
			blocks = {
				{ id = "p1", kind = "text", text = "First I'll look around the tests." },
				{ id = "g:k1", kind = "tools", summary = "Ran 1 command · read 1 file", tools = {
					{ id = "k1", kind = "exec", verb = "Ran", target = "cargo test" },
					{ id = "k2", kind = "read", verb = "Read", target = "chat.rs" },
				} },
				{ id = "p2", kind = "text", text = fix },
			},
			-- Compact (as the host lays it out): its work folded, done in
			-- 12 s, the reply it ends on shown.
			compact = {
				{ id = "work", kind = "work", summary = "Worked for 12s", tools = {
					{ id = "k1", kind = "exec", verb = "Ran", target = "cargo test" },
					{ id = "k2", kind = "read", verb = "Read", target = "chat.rs" },
				}, blocks = {
					{ id = "p1", kind = "text", text = "First I'll look around the tests." },
					{ id = "g:k1", kind = "tools", summary = "Ran 1 command · read 1 file", tools = {
						{ id = "k1", kind = "exec", verb = "Ran", target = "cargo test" },
						{ id = "k2", kind = "read", verb = "Read", target = "chat.rs" },
					} },
				} },
				{ id = "p2", kind = "text", text = fix },
			},
		},
		{ id = "cu1", role = "user", text = "Thanks, ship it.", at = "Sep 8, 1:42 PM", copy = "Thanks, ship it." },
		{ id = "ca1", role = "assistant", text = "Done. The release workflow is running.", model = "Opus", at = "Sep 8, 1:43 PM", copy = "Done. The release workflow is running.", blocks = { { id = "t", kind = "text", text = "Done. The release workflow is running." } } },
	}
	META.s8 = { title = "Fix the failing test", project = "atlas", model = "opus", effort = "high", access = "full", plan = false }
	-- An agent from the ACP registry: a chat moved to it, named by its
	-- harness alone everywhere (its mark found by that: `agent_icons`).
	TRANSCRIPTS.s9 = {
		line("g0", "user", 0, "Look at the flaky upload test."),
		line("g1", "assistant", 1, "It races the progress callback; I'll pin the order."),
		{ id = "switch:9", role = "switch", text = "", from = "Devin", to = "Factory Droid", from_mark = "devin", to_mark = "acp.factory-droid" },
		line("g2", "user", 2, "hello"),
	}
	META.s9 = { title = "Moved to Factory Droid", project = "atlas", model = "droid", effort = "high", access = "full", plan = false, agent = "acp.factory-droid" }
	seed()
end
-- A reply as the engine starts one: no blocks yet (its first part an
-- empty thought), then its text, then done with its time.
ui.command {
	id = "bare_reply",
	title = "A reply starts with nothing to show",
	run = function()
		store.messages:push {
			id = "e1",
			role = "assistant",
			text = "",
			model = "Opus",
			streaming = true,
			started = ui.now() / 1000,
			blocks = {},
		}
	end,
}
-- The files pane's "Add to chat" on two of the project's files.
ui.command {
	id = "add_to_chat",
	title = "Add two project files to the chat",
	run = function()
		require("@app/screens/chat/shared").add_to_chat({ "src/lib.rs", "docs/notes.md" }, "/tmp/atlas")
	end,
}
-- Files dropped from outside: a GIF and a HEIC (no thumbnail zgx can
-- draw yet), and a PNG of the app's own.
ui.command {
	id = "drop_pictures",
	title = "Drop a GIF, a HEIC and a PNG",
	run = function()
		require("@app/screens/chat/shared").attach_files({
			{ name = "spin.gif", path = "assets/spin.gif" },
			{ name = "photo.heic", path = "assets/photo.heic" },
			{ name = "zgx.png", path = "assets/favicons/zgx.png" },
		})
	end,
}
-- Two review comments left on files in the files pane.
ui.command {
	id = "leave_comments",
	title = "Leave two review comments",
	run = function()
		local shared = require("@app/screens/chat/shared")
		local id = shared.attach_comment({ file = "src/app.rs", line = 12, body = "Rename this." })
		shared.attach_comment({ file = "docs/guide.md", line = 3, body = "Typo here.\nAnd here." })
		shared.update_comment(id, "Rename this.\nIt reads badly.")
	end,
}
-- A review comment as the files pane leaves it (screens/files/comments:
-- `{ path, line, body, root }`), then carried two lines down by edits.
ui.command {
	id = "comment_from_files",
	title = "Leave a comment from the files pane",
	run = function()
		local shared = require("@app/screens/chat/shared")
		local id = shared.attach_comment({ path = "/proj/src/app.rs", line = 12, body = "Rename this.", root = "/proj" })
		ui.state("test_comment_id", ""):set(id)
	end,
}
-- Two comments a plugin left on a diff (Git's Changes, through
-- `zeron.compose_comment`; screens/plugins hands them on as this does): a
-- range of the changed file's lines with its code, and an old line.
-- `test_plugin_comments` says which of the plugin's are still carried.
ui.command {
	id = "plugin_diff_comments",
	title = "A plugin leaves two comments on a diff",
	run = function()
		local shared = require("@app/screens/chat/shared")
		shared.attach_comment({ file = "src/graph.rs", line = 10, to = 12, side = "R", snippet = " fn lanes() {}\n-fn rows() {}\n+fn rows(n: usize) {}", body = "Keep the old name.", plugin = "git", key = "k1", session = "s2" })
		shared.attach_comment({ file = "src/old.rs", line = 4, side = "L", body = "Why did this go?", plugin = "git", key = "k2", session = "s2" })
	end,
}
-- Asked again under k1 (its card's Edit), then k2 taken off by the plugin.
ui.command {
	id = "plugin_comment_edited",
	title = "The plugin asks k1 again with new words",
	run = function()
		local shared = require("@app/screens/chat/shared")
		shared.attach_comment({ file = "src/graph.rs", line = 10, to = 12, side = "R", body = "Keep the old name, it's public.", plugin = "git", key = "k1", session = "s2" })
	end,
}
ui.command {
	id = "plugin_comment_withdrawn",
	title = "The plugin takes k2 off",
	run = function()
		require("@app/screens/chat/shared").remove_plugin_comment("git", "k2")
	end,
}
local plugin_comments = ui.state("test_plugin_comments", "")
local plugin_notes = ui.state("test_plugin_notes", "")
ui.effect(function()
	local shared = require("@app/screens/chat/shared")
	local keys: { string } = {}
	local notes: { string } = {}
	for key, c in shared.plugin_comments().git or {} do
		table.insert(keys, `{key}@{c.session}`)
		table.insert(notes, `{key}={c.body}`)
	end
	table.sort(keys)
	table.sort(notes)
	plugin_comments:set(table.concat(keys, ","))
	plugin_notes:set(table.concat(notes, ","))
end)
ui.command {
	id = "comment_moves",
	title = "The comment is carried two lines down",
	run = function()
		local shared = require("@app/screens/chat/shared")
		shared.move_comment(ui.state("test_comment_id", ""):peek(), 14)
	end,
}
-- One long reply (about 400 top-level blocks: paragraphs, lists, code,
-- quotes), as a long agent answer comes: what a message's own windowing is
-- measured on.
local function long_reply_text(): string
	local parts: { string } = {}
	for i = 1, 100 do
		table.insert(parts, `## Part {i}`)
		table.insert(parts, `Part {i} explains how the render loop spends its frame: layout first, then paint, then present. The numbers below come from a trace of section {i}, read over several runs so one slow frame doesn't skew them.`)
		table.insert(parts, `- layout takes **{40 + i % 20}%**\n- paint takes {20 + i % 10}%\n- present takes the rest`)
		table.insert(parts, "```rust\nfn part_" .. i .. "(frame: &Frame) -> Duration {\n    frame.layout + frame.paint\n}\n```")
	end
	return table.concat(parts, "\n\n")
end
ui.command {
	id = "long_reply",
	title = "A long reply arrives",
	run = function()
		local text = long_reply_text()
		store.messages:push {
			id = "long1",
			role = "assistant",
			text = text,
			model = "Opus",
			at = "Sep 8, 2:00 PM",
			copy = text,
			blocks = { { id = "t", kind = "text", text = text } },
		}
	end,
}
-- The shell's ⌘/: the composer's model picker opened.
ui.command {
	id = "open_model_picker",
	title = "Open the model picker",
	run = function()
		note(`model_picker:{tostring(require("@app/screens/chat").open_model_picker())}`)
	end,
}
-- A picture of a project on this computer (Kit's), added to the chat.
ui.command {
	id = "add_picture_to_chat",
	title = "Add a picture of a project here to the chat",
	run = function()
		require("@app/screens/chat/shared").add_to_chat({ "favicons/zgx.png" }, "assets")
	end,
}
-- A picture of a project on another computer (the build box's).
ui.command {
	id = "add_far_picture_to_chat",
	title = "Add a picture of a project elsewhere to the chat",
	run = function()
		require("@app/screens/chat/shared").add_to_chat({ "ui/logo.png" }, "/srv/borealis")
	end,
}
-- A turn with emoji and an inline code span long enough to wrap: what a
-- look at colour glyphs and wrapped code corners wants.
ui.command {
	id = "emoji_turn",
	title = "A turn with emoji and wrapped code",
	run = function()
		local reply = "Shipped 🚀 and the tests are green ✅. Dark text keeps its colours: 👍🏽 ❤️ 🇯🇵 👩‍💻 🌚.\n\n"
			.. "The fix lives in `crates/zgx-script/src/reactive/ownership_of_effect_runs_and_their_timers.rs`, which a narrow column wraps mid-span, and `short` stays one piece."
		store.messages:push { id = "eu1", role = "user", text = "Did it ship? 🤞 Tell me 😅", at = "Sep 8, 1:50 PM", copy = "Did it ship?" }
		store.messages:push {
			id = "ea1",
			role = "assistant",
			text = reply,
			model = "Opus",
			at = "Sep 8, 1:50 PM",
			copy = reply,
			blocks = { { id = "t", kind = "text", text = reply } },
		}
	end,
}
-- A turn whose prompt mentions a file and whose reply links files (one at
-- a line, one outside the chat's folder, one on the web), in a chat whose
-- folder is /work/atlas; what opens a file there noted (`open_file:`).
local panel_stub: any = (require :: any)("@app/screens/panel")
panel_stub.open_file = function(session: any, path: string, _project: any?, at: any?)
	local line = if type(at) == "table" then at.line elseif type(at) == "number" then at else nil
	note(`open_file:{if type(session) == "table" then session.id else session}:{path}{if line then `@{line}` else ""}`)
end
-- The review host (src/chat/review.rs), stood in for: each call noted; Fix hands back the open finding's comment.
local Review = require("@app/screens/chat/review")
Review.host = function(name: string, args: { [string]: any }): any
	note(`{name}:{args.chat or ""}{if args.scope then ":" .. args.scope elseif args.id then ":" .. args.id else ""}`)
	if name == "review.fix" then
		return settled({ { path = "src/render.rs", line = 42, body = "Bug: the frame is drawn twice\nThe loop redraws on every tick." } })
	end
	return settled(nil)
end
local function review_state(look: string): any
	local scopes = {
		{ key = "uncommitted", label = "Uncommitted", add = 12, del = 3, empty = false, picked = true },
		{ key = "latest_turn", label = "Latest turn", add = 4, del = 1, empty = false, picked = false },
	}
	if look == "idle" then
		return { scope = "uncommitted", scopes = scopes, start_label = "Review uncommitted", label = "Review", tip = "Review uncommitted with a fresh agent", action = "start", busy = false, add = 12, del = 3, findings = {} }
	elseif look == "busy" then
		return { scope = "uncommitted", scopes = scopes, start_label = "Review uncommitted", label = "Reviewing", tip = "The reviewer is working.", action = "inert", busy = true, findings = {} }
	end
	return {
		scope = "uncommitted",
		scopes = scopes,
		start_label = "Review uncommitted",
		label = "1 finding",
		tip = "Open the findings",
		action = "open",
		busy = false,
		fix = "Fix 1 remaining",
		outcomes = "1 fixed · 1 open",
		findings = {
			{ id = "F1", severity = "Bug", title = "the frame is drawn twice", body = "The loop redraws on every tick.", location = "src/render.rs:42", path = "src/render.rs", line = 42, state = "Open", open = true },
			{ id = "F2", severity = "Nit", title = "a name", body = "", location = "src/app.rs:7", path = "src/app.rs", line = 7, state = "Fixed · abcdef1", open = false },
		},
	}
end
for _, look in { "idle", "busy", "done" } do
	ui.command {
		id = `review_{look}`,
		title = `Review: {look}`,
		run = function()
			local all = ui.shared("review.chats", {} :: any)
			local now = table.clone(all:peek())
			now.s2 = review_state(look)
			all:set(now)
		end,
	}
end
store.answer_secret = function(id: string, request: string, secret: string?): any
	note(`secret:{id}:{request}:{secret or "declined"}`)
	return settled(nil)
end
-- A computer card's answer (src/chat.rs `chat.answer_computer`); `computer_fail_next` makes the next one fail offline.
local computer_fails = false
store.answer_computer = function(id: string, request: string, question: string, answer: string): any
	note(`computer:{id}:{request}:{question}:{answer}`)
	if computer_fails then
		computer_fails = false
		return { after = function(_self: any, _ok: any, bad: (any) -> ())
			bad({ message = "offline" })
		end }
	end
	return settled(nil)
end
ui.command {
	id = "computer_fail_next",
	title = "The next computer answer fails",
	run = function()
		computer_fails = true
	end,
}
-- Computer use's rows, as src/chat.rs writes each (zeron_proto's words): a consent card for Ghostty, a foreground card for Safari, an answered call, the denied notice; the engine's notes.
local function consent_choice(key: string, label: string, answer: string, settled_words: string, primary: boolean?): any
	return { key = key, label = label, answer = answer, settled = settled_words, primary = primary == true }
end
ui.command {
	id = "computer_turn",
	title = "Computer use's cards and notes",
	run = function()
		store.messages:push({ id = "cu1", role = "user", text = "/computer-use open Ghostty", at = "Sep 8, 2:00 PM", copy = "/computer-use open Ghostty" })
		store.messages:push({
			id = "ca1",
			role = "assistant",
			text = "",
			model = "Opus",
			at = "Sep 8, 2:01 PM",
			copy = "",
			blocks = {
				{
					id = "c1",
					kind = "line",
					line = "computer_consent",
					request = "req-c1",
					question = "q1",
					scope = "task",
					app = "Ghostty",
					title = "Allow Zeron to use Ghostty?",
					detail = "Open a new tab and run the tests.",
					risk = "Zeron could run commands that change files and settings on this Mac.",
					disclosure_text = "Computer use lets Zeron use apps on your computer. It may take screenshots and act in the background. You can stop at any time from the menu bar.",
					choices = {
						consent_choice("deny", "Deny", "Deny", "Declined Ghostty"),
						consent_choice("always", "Always allow", "Always allow Ghostty", "Always allowed Ghostty"),
						consent_choice("allow", "Allow this conversation", "Allow", "Allowed Ghostty for this task", true),
					},
				},
				{
					id = "f1",
					kind = "line",
					line = "computer_foreground",
					request = "req-f1",
					question = "qf",
					app = "Safari",
					title = "Bring Safari to the front?",
					detail = "Zeron needs it in front to type. It will cover your screen until the turn ends.",
					choices = {
						{ key = "no", label = "No", answer = "No", primary = false },
						{ key = "allow", label = "Allow for this turn", answer = "Allow for this turn", primary = true },
					},
				},
				{ id = "c0", kind = "line", line = "computer_consent", request = "req-c0", question = "q0", scope = "call", title = "Approve this tool call?", outcome = "allowed", text = "Allowed once", choices = {} },
				{ id = "d1", kind = "line", line = "computer_notice", notice = "control_denied", text = "Computer is off for ordinary turns", lines = { "Send this request with /computer-use to let the agent use the desktop once." }, action = "Use for this request" },
			},
		})
		store.messages:push({ id = "cs1", role = "system", line = "computer_notice", notice = "revoked", text = "Desktop access revoked by you (Esc)" })
		store.messages:push({ id = "cs2", role = "system", line = "computer_notice", notice = "paused", text = "Paused while you used Safari" })
		store.messages:push({ id = "cs3", role = "system", line = "computer_notice", notice = "left_open", text = "Left 2 Ghostty windows open", app = "Ghostty" })
		store.messages:push({ id = "cs4", role = "system", line = "computer_notice", notice = "unavailable", text = "Computer is unavailable for this turn", lines = { "The desktop host is offline." } })
		store.messages:push({ id = "cs5", role = "system", line = "computer_notice", notice = "setup", text = "Computer control needs setup", lines = {} })
	end,
}
-- The engine's answer lands: the foreground card becomes its row.
ui.command {
	id = "computer_settle",
	title = "The engine settles the foreground card",
	run = function()
		for i, m in store.messages:peek() do
			for j, b in m.blocks or {} do
				if b.request == "req-f1" then
					store.messages:set_key({ i, "blocks", j }, { id = "f1", kind = "line", line = "computer_notice", notice = "foreground", text = "You allowed Safari to come to the front" })
				end
			end
		end
	end,
}
-- The app taking web links (ds/links, as the shell sets it for "Open links
-- in Zeron"): each link opened is noted, nothing else happens.
ui.command {
	id = "set_opener",
	title = "Take web links",
	run = function()
		local Links = require("@ds/links")
		Links.opener = function(url: string): boolean
			note(`opener:{url}`)
			return true
		end
	end,
}
-- "Open links in" as the shell has it (`test_open_links`: "zeron" or
-- "external"), the panel's browser tab noting what it's sent (`panel:`); a
-- chat's Zeron link and Reveal in Finder noted as asked (ds/links' menus).
local open_links = ui.state("test_open_links", "zeron")
ui.command {
	id = "set_links",
	title = "Links as the shell sets them",
	run = function()
		local Links = require("@ds/links")
		Links.panel = function(url: string): boolean
			note(`panel:{url}`)
			return true
		end
		Links.in_app = function(): boolean
			return open_links:peek() == "zeron"
		end
		Links.opener = function(url: string): boolean
			return open_links:peek() == "zeron" and (Links.panel :: any)(url)
		end
	end,
}
sessions_stub.link = function(id: string): any
	return settled(`zeron://open/chat/{id}?workspace=w1`)
end
local files_stub = (require :: any)("@app/screens/files/api")
files_stub.reveal = function(target: any, path: string): any
	note(`reveal:{target.chat}:{target.root}:{path}`)
	return settled(nil)
end
ui.command {
	id = "link_turn",
	title = "A turn with file links",
	run = function()
		local s = table.clone(store.session:peek() or {})
		s.cwd = "/work/atlas"
		store.session:set(s)
		store.messages:push {
			id = "lu1",
			role = "user",
			text = "Look at [chat.rs](zeron-file:src/chat.rs) please, as in [Long chat](zeron-chat:s1), and [the site](https://example.com/)",
			at = "Sep 8, 1:50 PM",
			copy = "Look at chat.rs please",
		}
		local reply = "The fix is in [the parser](src/parse.rs#L12) and [the host](/work/atlas/src/main.rs:40). "
			.. "Not [this one](/etc/hosts.conf), nor [the docs](https://example.com/a.rs). "
			.. "PR: **https://github.com/zeronsh/example/pull/1**."
		store.messages:push {
			id = "la1",
			role = "assistant",
			text = reply,
			model = "Opus",
			at = "Sep 8, 1:50 PM",
			copy = reply,
			blocks = { { id = "t", kind = "text", text = reply } },
		}
	end,
}
-- One of every transcript row kind, as src/chat.rs writes each, in a chat whose folder is /work/atlas.
local function kinds_messages(): { any }
	local reply = "The parser is [parse.rs](src/parse.rs:12) and the host [main.rs](/work/atlas/src/main.rs#L40), not [this one](/etc/hosts.conf) nor [the docs](https://example.com/a.rs).\n\n"
		.. "[chat.rs](src/chat.rs:7)\n\nCargo.toml\n\nversion 1.2"
	local function tool(id: string, kind: string, verb: string, target: string?, extra: { [string]: any }?): any
		local t: { [string]: any } = { id = id, kind = kind, verb = verb, target = target }
		for k, v in extra or {} do
			t[k] = v
		end
		return t
	end
	return {
		{ id = "ku1", role = "user", text = "Ask [Long chat](zeron-chat:s1) about [chat.rs](zeron-file:src/chat.rs)", at = "Sep 8, 1:50 PM", copy = "Ask Long chat", attachments = { { kind = "file", name = "Pasted text 2.txt", path = "/tmp/pasted/x.txt" } } },
		{
			id = "ka1",
			role = "assistant",
			text = reply,
			model = "Opus",
			at = "Sep 8, 1:51 PM",
			copy = reply,
			blocks = {
				{
					id = "g:k1",
					kind = "tools",
					summary = "Read 1 file · called 4 tools",
					tools = {
						tool("k1", "read", "Read", "parse.rs", { path = "src/parse.rs", output = "fn parse_all() {}" }),
						tool("k2", "mcp", "Used", "zeron.device_screenshot", { images = { "assets/composer/shell.png", "assets/composer/transcript.png" } }),
						tool("k3", "wait", "Waited for agents"),
						tool("k4", "skill", "Used", "zgx-port", { path = "/work/atlas/skills/zgx-port/SKILL.md" }),
						tool("k5", "todo", "Updated the plan", "1/3 done", { call = "[x] a\n[ ] b\n[ ] c" }),
						tool("k6", "edit", "Patched", "workspace"),
					},
				},
				{ id = "t1", kind = "text", text = reply },
				{ id = "q1", kind = "line", line = "input", outcome = "answered", text = "Scope → Branch · Reviewer → Codex", answers = { { label = "Scope", answer = "Branch", skipped = false }, { label = "Reviewer", answer = "Codex", skipped = false } } },
				{ id = "q2", kind = "line", line = "input", outcome = "answered", text = "Plan → Skipped", answers = { { label = "Plan", answer = "Skipped", skipped = true } } },
				{ id = "q3", kind = "line", line = "input", outcome = "pending", title = "Deploy" },
				{ id = "x1", kind = "line", line = "secret", request = "req-1", title = "Tailscale auth key", reason = "Create one at https://login.tailscale.com/admin/settings/keys.", status = "pending" },
				{ id = "x2", kind = "line", line = "secret", request = "req-0", title = "Old key", reason = "", status = "saved" },
				{ id = "e1", kind = "line", line = "error", text = "Codex exited with status 101: thread 'main' panicked at src/lib.rs:4" },
				{ id = "e2", kind = "line", line = "error", text = "Claude Code is not logged in. Please run /login.", problem = "signed_out" },
			},
		},
		{ id = "ks1", role = "system", line = "claude_command", text = "Claude Code ran /cost", card = { command = "/cost", text = "Total cost: 1.2M tokens\nTotal duration: 4m", pending = false } },
		{ id = "ks2", role = "system", line = "claude_command", text = "Claude Code ran /doctor", card = { command = "/doctor", text = "", pending = true, started = os.time() * 1000, stale_ms = 120000 } },
		{ id = "ks3", role = "system", line = "claude_command", text = "Claude Code ran /status", card = { command = "/status", text = "", pending = false, terminal = "status", action = "Open Status" } },
		{ id = "ks6", role = "system", line = "claude_command", text = "Claude Code /model", card = { command = "/model", text = "", pending = false, model = "opus", models = { { id = "sonnet", label = "Sonnet" }, { id = "opus", label = "Opus" }, { id = "haiku", label = "Haiku" } } } },
		{ id = "ks4", role = "system", line = "claude_config", text = "Claude Code settings · Theme → dark, Verbose → on" },
		{ id = "ks5", role = "system", line = "divider", text = "Branch switched to feat/kinds" },
	}
end
ui.command {
	id = "kinds_turn",
	title = "One of every row kind",
	run = function()
		local s = table.clone(store.session:peek() or {})
		s.cwd = "/work/atlas"
		store.session:set(s)
		for _, m in kinds_messages() do
			store.messages:push(m)
		end
	end,
}
-- Forty turns for the reading place, each changed above the view by the commands after it.
ui.command {
	id = "kinds_long",
	title = "Forty turns, a question waiting early on",
	run = function()
		local list = {}
		for i = 1, 40 do
			table.insert(list, { id = `lu{i}`, role = "user", text = `Prompt {i} asks a thing.`, at = "Sep 8, 1:50 PM", copy = `Prompt {i}` })
			local blocks: { any } = { { id = "t", kind = "text", text = `Reply {i} says a line of words.` } }
			if i == 5 then
				table.insert(blocks, { id = "q", kind = "line", line = "input", outcome = "pending", title = "Pick" })
			end
			table.insert(list, { id = `la{i}`, role = "assistant", text = `Reply {i} says a line of words.`, model = "Opus", at = "Sep 8, 1:51 PM", copy = "x", blocks = blocks })
		end
		store.messages:set(list)
	end,
}
-- A steer as the engine writes it: the reply so far settles, its prompt lands, the reply goes on in a new entry.
ui.command {
	id = "kinds_steer",
	title = "A steer lands mid-turn",
	run = function()
		local n = store.messages:len()
		store.messages:set_key({ n, "streaming" }, false)
		store.messages:set_key({ n, "text" }, "First half of the answer, before the steer.")
		store.messages:push { id = "steer", role = "user", text = "Also check the tests while you're at it.", at = "Sep 8, 5:01 PM", copy = "Also check" }
		store.messages:push { id = "live2", role = "assistant", text = "", model = "Opus", streaming = true, started = ui.now() / 1000, tools = {} }
		store.draft:set { id = "live2", text = "Going on with the tests" }
	end,
}
-- (Which reply: `kinds_at`, the nth.)
local kinds_at = ui.state("kinds_at", 3)
ui.command {
	id = "kinds_grow_above",
	title = "A reply grows",
	run = function()
		local long = string.rep("A paragraph that grew in while the reader was further down.\n\n", 12)
		store.messages:set_key({ kinds_at:peek() * 2, "blocks" }, { { id = "t", kind = "text", text = long } })
	end,
}
ui.command {
	id = "kinds_image_above",
	title = "A reply above shows a picture still coming",
	run = function()
		local n = kinds_at:peek()
		store.messages:set_key({ n * 2, "blocks" }, { { id = "t", kind = "text", text = `Reply {n} says a line of words.\n\n![late](/work/atlas/late.png)` } })
	end,
}
ui.command {
	id = "kinds_image_lands",
	title = "The picture above is read",
	run = function()
		local all = image_waits
		image_waits = {}
		for _, ok in all do
			ok({ src = "assets/pictures/wide.png" })
		end
	end,
}
-- A tool group in the reply above the view, folded; `kinds_fold_above` opens and closes it.
ui.command {
	id = "kinds_tools_above",
	title = "A reply above has its tools",
	run = function()
		local n = kinds_at:peek()
		local tools = {}
		for i = 1, 6 do
			table.insert(tools, { id = `kt{i}`, kind = "read", verb = "Read", target = `file{i}.rs`, output = "fn x() {}" })
		end
		store.messages:set_key({ n * 2, "blocks" }, {
			{ id = "t", kind = "text", text = `Reply {n} says a line of words.` },
			{ id = "g", kind = "tools", summary = "Read 6 files", tools = tools },
		})
	end,
}
-- A read of a file outside the chat's folder, and links to names with no `.`, in and out of it.
ui.command {
	id = "kinds_outside",
	title = "Files the panel can't open",
	run = function()
		local text = "Edit [Makefile](file:///work/atlas/Makefile:3) and not [LICENSE](file:///etc/LICENSE)."
		store.messages:push {
			id = "ko1",
			role = "assistant",
			text = text,
			model = "Opus",
			at = "Sep 8, 1:52 PM",
			copy = text,
			blocks = {
				{ id = "g", kind = "tools", summary = "Read 2 files · outside", tools = {
					{ id = "ko-r", kind = "read", verb = "Read", target = "hosts.conf", path = "/etc/hosts.conf", output = "127.0.0.1 localhost" },
					{ id = "ko-s", kind = "skill", verb = "Used", target = "review", path = "/home/me/.claude/skills/review/SKILL.md" },
				} },
				{ id = "t", kind = "text", text = text },
			},
		}
	end,
}
-- The secret card on a host without `chat.answer_secret`, or one whose chat is gone (no answer comes back).
ui.command {
	id = "kinds_old_host",
	title = "A host without the secret action",
	run = function()
		store.answer_secret = function(id: string, request: string, secret: string?): any
			return ui.call("chat.answer_secret", { id = id, request = request, secret = secret })
		end
	end,
}
ui.command {
	id = "kinds_no_answer",
	title = "The secret's answer goes nowhere",
	run = function()
		store.answer_secret = function(): any
			return nil
		end
	end,
}
-- The pending secret settled elsewhere (timed out), or asking again; and the chat's rows made again as they're scrolled back.
ui.command {
	id = "kinds_secret_timed_out",
	title = "The secret request times out, or asks again",
	run = function()
		for i, m in store.messages:peek() do
			for j, b in m.blocks or {} do
				if b.request == "req-1" then
					store.messages:set_key({ i, "blocks", j, "status" }, if b.status == "pending" then "timedout" else "pending")
				end
			end
		end
	end,
}
local kinds_kept: { any }? = nil
ui.command {
	id = "kinds_rows_away",
	title = "The rows leave",
	run = function()
		kinds_kept = store.messages:peek()
		store.messages:set({})
	end,
}
ui.command {
	id = "kinds_rows_back",
	title = "The rows come back",
	run = function()
		if kinds_kept then
			store.messages:set(kinds_kept)
		end
	end,
}
ui.command {
	id = "kinds_answer_above",
	title = "A reply's question is answered",
	run = function()
		local n = kinds_at:peek()
		store.messages:set_key({ n * 2, "blocks" }, {
			{ id = "t", kind = "text", text = `Reply {n} says a line of words.` },
			{ id = "q", kind = "line", line = "input", outcome = "answered", text = "", answers = { { label = "A", answer = "1" }, { label = "B", answer = "2" }, { label = "C", answer = "3" } } },
		})
	end,
}
-- A live turn whose tools include a command of five lines (a shell script
-- run as one call), a tool named only, and a thought: as the user's turn
-- had it. `call` is the whole command, as the host gives it.
local SCRIPT = table.concat({
	"echo '--- extension-ish in main'; git grep -n 'extension' -- '*.rs' | head -20",
	"echo '--- plugin dirs'; ls -d crates/*plugin* apps/*/plugins 2>/dev/null",
	"echo '--- branches'; git branch -a | grep -i plugin",
	"echo '--- PRs'; gh pr list --search plugin --state all --limit 10",
	"echo '--- issues'; gh issue list --search plugin --state all --limit 10",
}, "\n")
ui.command {
	id = "script_turn",
	title = "A live turn that ran a five-line command",
	run = function()
		local one = string.gsub(SCRIPT, "%s+", " ")
		store.messages:push {
			id = "st1",
			role = "assistant",
			text = "",
			model = "Opus",
			streaming = true,
			started = ui.now() / 1000,
			blocks = {
				{ id = "g:st", kind = "tools", summary = "Thought process · ran 1 command · called 1 tool", live = true, tools = {
					{ id = "st-x", kind = "exec", verb = "Ran", target = one, call = SCRIPT },
					{ id = "st-t", kind = "exec", verb = "Used", target = "ToolSearch" },
					{ id = "st-r", kind = "thought", verb = "Thought" },
				} },
			},
		}
	end,
}
-- The same, with the command's newlines left in its line (what the host
-- sent before it made each tool's line one line): the row must still hold
-- it to one line.
ui.command {
	id = "script_turn_raw",
	title = "A live turn whose command's line has its newlines",
	run = function()
		store.messages:push {
			id = "st2",
			role = "assistant",
			text = "",
			model = "Opus",
			streaming = true,
			started = ui.now() / 1000,
			blocks = {
				{ id = "g:st2", kind = "tools", summary = "Thought process · ran 1 command · called 1 tool", live = true, tools = {
					{ id = "st2-x", kind = "exec", verb = "Ran", target = SCRIPT },
					{ id = "st2-t", kind = "exec", verb = "Used", target = "ToolSearch" },
					{ id = "st2-r", kind = "thought", verb = "Thought" },
				} },
			},
		}
	end,
}
-- ⌘F's find in the chat shown (what the shell's Find runs).
ui.command {
	id = "find_open",
	title = "Find in the chat",
	run = function()
		require("@app/screens/chat/find").open()
	end,
}
-- A live turn still thinking (its thought being written, as the host gives
-- it: live), then the thought settling.
ui.command {
	id = "thinking_turn",
	title = "A live turn, thinking",
	run = function()
		store.messages:push {
			id = "th1",
			role = "assistant",
			text = "",
			model = "Opus",
			streaming = true,
			started = ui.now() / 1000,
			blocks = {
				{ id = "g:th", kind = "tools", summary = "Thought process", live = true, tools = {
					{ id = "th-r", kind = "thought", verb = "Thought process", call = "Weighing the parser first.", prose = true, live = true },
				} },
			},
		}
	end,
}
-- The thought going on (its words past the fold, the latest still coming).
ui.command {
	id = "thought_grows",
	title = "The thought grows past the fold",
	run = function()
		local n = store.messages:len()
		store.messages:set_key(
			{ n, "blocks", 1, "tools", 1, "call" },
			"Weighing the parser first.\n\n**The parser**\n\nIt reads the whole file on every keystroke, and the file is the transcript, so the cost grows with the chat. A cursor into the last block would read only what changed, which is what the host already keeps.\n\n**The host**\n\nIt sends each block whole as it grows, so the parser's input is the bottleneck twice over. The last line is the freshest thought."
		)
	end,
}
-- More of it, as the host streams one: the draft's words alone, its row's
-- own (`call`) left as they were.
ui.command {
	id = "thought_streams",
	title = "More of the thought arrives",
	run = function()
		store.draft:set { id = "th1", text = "", thought = "th-r", thought_text = "Weighing the parser first. Then the host." }
	end,
}
-- Written: the row takes its words, then the draft lets them go.
ui.command {
	id = "thought_written",
	title = "The thought is written",
	run = function()
		local n = store.messages:len()
		store.messages:set_key({ n, "blocks", 1, "tools", 1, "call" }, "Weighing the parser first. Then the host.")
		store.messages:set_key({ n, "blocks", 1, "tools", 1, "live" }, false)
		store.messages:set_key({ n, "blocks", 1, "live" }, false)
		store.draft:set { id = "", text = "" }
	end,
}
ui.command {
	id = "thought_settles",
	title = "The thought settles",
	run = function()
		local n = store.messages:len()
		store.messages:set_key({ n, "blocks", 1, "tools", 1, "live" }, false)
		store.messages:set_key({ n, "blocks", 1, "live" }, false)
	end,
}
ui.command {
	id = "bare_reply_text",
	title = "Its text arrives",
	run = function()
		local n = store.messages:len()
		store.messages:set_key({ n, "blocks" }, { { id = "t", kind = "text", text = "The moon knocked. I opened the window. It came home." } })
		store.messages:set_key({ n, "text" }, "The moon knocked. I opened the window. It came home.")
	end,
}
ui.command {
	id = "bare_reply_done",
	title = "It is done",
	run = function()
		local n = store.messages:len()
		store.messages:set_key({ n, "streaming" }, false)
		store.messages:set_key({ n, "at" }, "Sep 8, 2:00 PM")
		store.messages:set_key({ n, "copy" }, "The moon knocked. I opened the window. It came home.")
	end,
}
-- (A session dropped from the sidebar on the chat: s1 mentioned.)
ui.command {
	id = "mention_s9",
	title = "Mention s9",
	run = function()
		require("@app/screens/chat/shared").mention_chat("s9")
	end,
}
ui.command {
	id = "mention_s2",
	title = "Mention s2",
	run = function()
		require("@app/screens/chat/shared").mention_chat("s2")
	end,
}
ui.command {
	id = "mention_s1",
	title = "Mention s1",
	run = function()
		require("@app/screens/chat/shared").mention_chat("s1")
	end,
}
for _, id in { "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9" } do
	ui.command {
		id = `go_{id}`,
		title = `Open {id}`,
		run = function()
			route.open_session(id)
		end,
	}
end
ui.command {
	id = "go_home",
	title = "A new chat",
	run = function()
		route.new_chat("atlas")
	end,
}

-- The catalog changing: an agent arriving with its models, and none at all
-- (signed out of every agent, or still looking).
ui.command {
	id = "catalog_grows",
	title = "Another agent arrives",
	run = function()
		local c = store.catalog:peek()
		local harnesses: { any } = table.clone(c.harnesses)
		local models: { any } = table.clone(c.models)
		table.insert(harnesses, { id = "codex", label = "Codex", mark = "openai" })
		table.insert(models, { value = "gpt", label = "GPT-5.5", harness = "codex", efforts = EFFORT, effort = "medium" })
		store.catalog:set { state = "ready", harnesses = harnesses, models = models }
	end,
}
-- An agent from the ACP registry in the catalog, as the host lists it: its
-- mark its harness (no icon carried: found by that).
ui.command {
	id = "catalog_registry",
	title = "Factory Droid arrives",
	run = function()
		local c = store.catalog:peek()
		local harnesses: { any } = table.clone(c.harnesses)
		local models: { any } = table.clone(c.models)
		table.insert(harnesses, { id = "acp.factory-droid", label = "Factory Droid", mark = "acp.factory-droid" })
		table.insert(models, { value = "droid", label = "Droid Auto", harness = "acp.factory-droid", efforts = EFFORT, effort = "medium" })
		store.catalog:set { state = "ready", harnesses = harnesses, models = models }
	end,
}
-- The engine naming models new for the first time (src/chat.rs `announce`):
-- one, then two.
ui.command {
	id = "catalog_announces",
	title = "New models arrive",
	run = function()
		local c: any = table.clone(store.catalog:peek())
		local announced: { any } = table.clone(c.announced or {})
		local seq = #announced + 1
		table.insert(announced, { seq = seq, agent = "Codex", models = if seq == 1 then { "GPT-6 Sol" } else { "GPT-6 Sol mini", "GPT-6 Luna" } })
		c.announced = announced
		store.catalog:set(c)
	end,
}
ui.command {
	id = "catalog_empty",
	title = "No agents",
	run = function()
		store.catalog:set { state = "ready", harnesses = {}, models = {} }
	end,
}

-- A plugin putting words into the composer (`zeron.compose.insert`).
ui.command {
	id = "insert_words",
	title = "Insert words into the composer",
	run = function()
		store.insert("src/chat.rs")
	end,
}

-- The session page made again, as a hot reload or a saved template does.
local remade = ui.state("remade", 0)
ui.command {
	id = "remake_session",
	title = "Make the session page again",
	run = function()
		remade:add(1)
	end,
}

-- Agent profiles (src/agent_profiles.rs), as the host answers
-- (profiles_stub.luau): s2 on Planner after `s2_on_planner`.
require("./profiles_stub").install {
	note = note,
	meta = META,
	publish = publish,
	store = stub,
	settled = settled,
}

-- Compaction (tests/chat_compaction): the rows a test sets pushed as the
-- host publishes them, one row's fields replaced, and how many of the
-- dividers' clocks tick.
local test_rows = ui.state("test_rows", {} :: { any })
local test_row_patch = ui.state("test_row_patch", {} :: { [string]: any })
local test_row_drop = ui.state("test_row_drop", "")
local test_clocks = ui.state("test_clocks", -1)
ui.command {
	id = "push_rows",
	title = "Push the test's rows",
	run = function()
		for _, m in test_rows:peek() do
			store.messages:push(m)
		end
	end,
}
ui.command {
	id = "patch_row",
	title = "Patch a row of the transcript",
	run = function()
		local patch = test_row_patch:peek()
		for i = 1, store.messages:len() do
			local m = store.messages:at(i)
			if m and m.id == patch.id then
				local next_m = table.clone(m)
				for k, v in patch do
					next_m[k] = v
				end
				store.messages:set_key({ i }, next_m)
			end
		end
	end,
}
ui.command {
	id = "drop_row",
	title = "Drop a row of the transcript",
	run = function()
		local id = test_row_drop:peek()
		local kept = {}
		for i = 1, store.messages:len() do
			local m = store.messages:at(i)
			if m and m.id ~= id then
				table.insert(kept, m)
			end
		end
		store.messages:set(kept)
	end,
}
ui.command {
	id = "compaction_clocks",
	title = "How many compaction clocks tick",
	run = function()
		local c = require("@ds/compaction")
		test_clocks:set(c.clocks)
	end,
}

-- The shell, as small as can hold the page: the route's page, and the
-- composer over it.
local test_fan = ui.state("test_fan", false)
-- Agent pages (ds/html_card) without a host: a view is a box that hears what a test posts (`page_post`).
do
	local html_api = require("@ds/html_api") :: any
	local heard: { [string]: (any, string, boolean) -> () } = {}
	local navigators: { [string]: (any) -> () } = {}
	local runs = ui.state("page_runs", {} :: { string })
	local loaded = ui.state("page_loaded", {} :: { string })
	local early = ui.state("page_early_script", false)
	html_api.web = function(id: string): any
		return { id = id }
	end
	local real_load = html_api.load
	html_api.load = function(web: any, html: string, ...)
		real_load({
			html = function(_self: any, h: string, base: string?)
				local list = table.clone(loaded:peek())
				table.insert(list, `{web.id}|{base or "about:blank"}|{h}`)
				loaded:set(list)
			end,
		}, html, ...)
	end
	html_api.run = function(web: any, js: string)
		local list = table.clone(runs:peek())
		table.insert(list, `{web.id}|{js}`)
		runs:set(list)
	end
	html_api.close = function(_web: any) end
	html_api.forget = function(_args: any) end
	html_api.page = function(args: any, done: (any, string?) -> ())
		if string.find(args.path or "", "gone", 1, true) then
			done(nil, "not found")
			return
		end
		done({ html = `<p>{args.path}</p>`, nonce = `n:{args.path}`, picks = if early:peek() then nil else `p:{args.path}` }, nil)
	end
	html_api.view = function(props: any): Node
		if props.id then
			heard[props.id] = props.on_message
			navigators[props.id] = props.on_navigate
		end
		return ui.column { id = props.id, label = props.label, w_full = true, h = props.h, min_h = props.min_h, grow = props.grow }
	end
	html_api.retheme = function(_args: any): string
		return "retheme()"
	end
	-- (zeron_proto html_render `frame_height`, `overflows`.)
	html_api.frame = function(agent: number, content: number?): any
		local a = math.clamp(math.floor(agent + 0.5), 80, 2000)
		local over = content ~= nil and content > a + 64
		local h = if content ~= nil and not over then content else a
		h = math.clamp(math.floor(h + 0.5), 80, 2000)
		return { height = h, agent = a, overflows = over, scrolls = content ~= nil and content > h + 1 }
	end
	html_api.live = function(url: string): any
		local server = string.match(url, "^https?://([^/?#]+)")
		if not server or not (string.find(server, "^localhost") or string.find(server, "^127%.0%.0%.1")) then
			return nil
		end
		local rest = string.match(url, "^https?://([^?#]*)") or server
		return { label = (string.gsub(rest, "/+$", "")), server = server, url = url }
	end
	local reach_up = ui.state("page_reach_up", true)
	html_api.reach = function(_url: string, done: (boolean) -> ())
		note("reach")
		done(reach_up:peek())
	end
	html_api.source = function(args: any): string
		return `<p>{args.path}</p>`
	end
	html_api.save = function(args: any, done: (boolean) -> ())
		note(`save:{args.path}:{args.to}`)
		done(true)
	end
	html_api.file_name = function(title: string): string
		return `{title}.html`
	end
	local titles = ui.state("page_titles", { ["/tmp/atlas/out/demo.html"] = "Demo page" } :: { [string]: string })
	html_api.title = function(path: string, done: (any) -> ())
		local t = titles:peek()[path]
		done(if t then { title = t } else { missing = true })
	end
	html_api.picks = function(state: any): string
		return ui.json.encode(state)
	end
	html_api.open_url = function(url: string)
		note(`open_url:{url}`)
	end
	local page_nav = ui.state("page_nav", nil :: any)
	ui.command {
		id = "page_navigate",
		title = "A page navigates",
		run = function()
			local n = page_nav:peek()
			local f = n and navigators[n.id]
			if f then
				f({ kind = n.kind, url = n.url })
			end
		end,
	}
	-- `{ id = <view id>, value, main? }`.
	local page_message = ui.state("page_message", nil :: any)
	ui.command {
		id = "page_post",
		title = "A page posts a message",
		run = function()
			local m = page_message:peek()
			local f = m and heard[m.id]
			if f then
				f(m.value, "https://zeron.page", m.main ~= false)
			end
		end,
	}
	local function page_reply(id: string, blocks: { any }): any
		return { id = id, role = "assistant", text = "", model = "Opus", at = "Sep 8, 1:43 PM", copy = "", blocks = blocks }
	end
	local function seed(list: { any })
		TRANSCRIPTS.s2 = list
		store.messages:set(list)
		publish("s2", { cwd = "/tmp/atlas" })
	end
	ui.command {
		id = "pages_seed",
		title = "s2 shows agent pages",
		run = function()
			seed {
				{ id = "x2", role = "user", text = "And pictures?" },
				page_reply("p2", {
					{ id = "tc2-html", kind = "html", path = "/u/html/s2/pictures.html", title = "Ảnh", height = 120 },
					{ id = "tc3-html", kind = "html", path = "/u/html/s2/gone.html", title = "Gone", height = 100 },
				}),
				{ id = "x1", role = "user", text = "Design the frame" },
				page_reply("p1", {
					{ id = "tc1-html", kind = "html", path = "/u/html/s2/frame.html", title = "Khung app", height = 600 },
					{ id = "t", kind = "text", text = "Ba chỗ cần chọn." },
				}),
			}
		end,
	}
	ui.command {
		id = "pages_seed_spans",
		title = "s2 names three .html files not on disk in code, then links one that is",
		run = function()
			seed {
				{ id = "x4", role = "user", text = "Which?" },
				page_reply("p4", {
					{ id = "t", kind = "text", text = "`a.html` `b.html` `c.html` [demo](out/demo.html)" },
					{ id = "html-files", kind = "html_files", targets = { { dest = "a.html", linked = false }, { dest = "b.html", linked = false }, { dest = "c.html", linked = false }, { dest = "out/demo.html", linked = true } } },
				}),
			}
		end,
	}
	ui.command {
		id = "pages_seed_live",
		title = "s2 shows a dev server's page and names .html files",
		run = function()
			seed {
				{ id = "x3", role = "user", text = "Run it" },
				page_reply("p3", {
					{ id = "tc4-html", kind = "html", url = "http://localhost:5391/app/?x=1", title = "Tray", height = 300 },
					{ id = "t", kind = "text", text = "Made [demo](out/demo.html) and `out/other.html`." },
					{ id = "html-files", kind = "html_files", targets = { { dest = "out/demo.html", linked = true }, { dest = "out/other.html", linked = false }, { dest = "out/missing.html", linked = true } } },
				}),
			}
		end,
	}
	-- chat.rs `page_picks`: only trailing blocks in exactly the sent form are chips.
	local function block_of(par: string): (string?, { any })
		local lines = string.split(par, "\n")
		local card = string.match(lines[1] or "", '^Đã chọn trong "(.+)":$')
		local chips, missing = {}, false
		if not card then
			return nil, chips
		end
		for i = 2, #lines do
			local line = lines[i]
			local free = string.match(line, "^%- (.+)$")
			local g, code, label = string.match(line, "^(%S+) = (%S+) %((.*)%)$")
			if missing then
				return nil, {}
			elseif free then
				table.insert(chips, { kind = "pick", name = free, card = card, group = "", code = free, label = free })
			elseif g then
				table.insert(chips, { kind = "pick", name = label, card = card, group = g, code = code, label = label })
			elseif #chips > 0 and string.sub(line, 1, #"Chưa chọn:") == "Chưa chọn:" then
				missing = true
			else
				return nil, {}
			end
		end
		return if #chips > 0 then card else nil, chips
	end
	ui.command {
		id = "picks_echo",
		title = "s2's last prompt comes back with its picks",
		run = function()
			local text: string? = nil
			for i = #said, 1, -1 do
				local m = string.match(said[i], "^send:s2:(.*)$")
				if m then
					text = m
					break
				end
			end
			if not text then
				return
			end
			local pars = string.split((string.gsub(text, "%s+$", "")), "\n\n")
			local blocks = {}
			while #pars > 0 do
				local card, found = block_of(pars[#pars])
				if not card then
					break
				end
				table.insert(blocks, 1, found)
				table.remove(pars)
			end
			local chips = {}
			for _, b in blocks do
				for _, c in b do
					table.insert(chips, c)
				end
			end
			local words = { table.concat(pars, "\n\n") }
			local list = table.clone(TRANSCRIPTS.s2 or {})
			table.insert(list, { id = `echo{#list}`, role = "user", text = (string.gsub(table.concat(words, "\n"), "%s+$", "")), attachments = chips })
			TRANSCRIPTS.s2 = list
			store.messages:set(list)
		end,
	}
end

return ui.overlay {
	on_press = function()
		require("@app/screens/chat/pills").press_outside()
	end,
	w_full = true,
	h_full = true,
	ui.column {
		w_full = true,
		h_full = true,
		pt = ui.raw(38, "the title bar's height"),
		ui.presence(function(): string
			-- (Keyed by the remake count too: the test's stand-in for a reload.)
			return `{route.current:get().name}:{remade:get()}`
		end, function(key: string): Node
			local name = string.match(key, "^(%a+)")
			if name == "session" then
				return chat.Session()
			end
			return chat.Home()
		end, { w_full = true, grow = 1, min_h = 0 }),
	},
	chat.Composer(),
	-- A link's context menu, as the shell hangs it (ds/menu_at).
	require("@ds/menu_at").Host {},
	-- What's said for a moment, as the shell has it (ds/toast).
	ds.toast.Host {},
	ui.show(function(): boolean
		return test_fan:get()
	end, ui.div {
		absolute = true,
		left = 400,
		top = 6,
		require("@app/screens/chat/pills").Fan {
			name = "test-fan",
			chat = "s2",
			tiles = function(): { any }
				return require("@app/screens/chat/pills").fan_of("s2", { { key = "row", asking = false, count = 2 } }) or {}
			end,
		},
	}),
	-- In the title bar's strip, what a drag may come from: the files pane's
	-- files (their paths), and something else (a sidebar section).
	ui.row {
		absolute = true,
		left = ui.raw(80, "past the traffic lights"),
		top = ui.raw(6, "in the title bar's strip"),
		gap = "$space.2",
		ui.div {
			id = "fixture-drag-files",
			w = 24,
			h = 24,
			bg = "$hover",
			on_drag = function(): any
				return { tree_files = { 1 }, paths = { "src/lib.rs" }, root = "/tmp/atlas" }
			end,
		},
		ui.div {
			id = "fixture-drag-section",
			w = 24,
			h = 24,
			bg = "$hover",
			on_drag = function(): any
				return { section = "later" }
			end,
		},
	},
}
