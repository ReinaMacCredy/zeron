-- The shell fixture with a stub task host that edits `tasks.list` and records `tasks_calls`.

local node = require("./shell_fixture")
local store = require("@app/screens/tasks/store")
local sessions = require("@account/sessions")

local real_host = table.clone(store.host)
local refuse: string? = nil

local NOW = os.time()
local DAY = 86400
local said: { string } = {}
local calls = ui.state("tasks_calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local function task(id: string, title: string, order: string, extra: { [string]: any }?): any
	local t: { [string]: any } = { id = id, title = title, notes = "", order = order, created = (NOW - 3600) * 1000, images = {}, depends = {} }
	for k, v in extra or {} do
		t[k] = v
	end
	return t
end

local tasks: { any } = {
	task("t1", "Fix the login redirect", "a1", { chat = "b1", project = "borealis" }),
	task("t2", "Port the sidebar fold", "a2", { chat = "a1", project = "atlas", images = { "assets/pictures/circles.png", "assets/pictures/wide.png" } }),
	task("t3", "Renew the signing certificate", "a3", { due = os.date("%Y-%m-%d", NOW - 3 * DAY) }),
	task("t4", "Draft the onboarding copy", "a4", { project = "atlas" }),
	task("t5", "Review the sidebar PR", "a5", { depends = { "t2" } }),
	task("t6", "Write the release notes", "a6", { chat = "a2" }),
	task("t7", "Tell the docs channel", "a7", { chat = "gone" }),
	task("t8", "Ask the design team", "a8", { chat = "a3", send_error = "The thread's device is offline" }),
	task("t9", "Answer the design review", "a9", { completed = (NOW - 60) * 1000 }),
	task("t10", "Upgrade the build image", "b0", { completed = (NOW - 2 * DAY) * 1000 }),
}
local function as_host(t: any, chats: { [string]: any }): any
	local out = table.clone(t)
	out.plan = nil
	if t.completed then
		out.column = "done"
		return out
	end
	local c = if t.chat then chats[t.chat] else nil
	local lead = if c then c.status or "idle" else nil
	local p = t.plan or {}
	local has: { [string]: boolean } = {}
	local done, total = 0, 0
	for _, s in p.states or {} do
		has[s] = true
		done += if s == "done" then 1 else 0
		total += if s == "skipped" then 0 else 1
	end
	local steps = #(p.states or {})
	local reason: string? = nil
	if t.send_error then
		reason = "not_sent"
	elseif lead == "input" then
		reason = "asks"
	elseif lead == "failed" then
		reason = "failed"
	elseif has.waitingOnYou then
		reason = "step_asks"
	elseif lead ~= "working" and has.failed then
		reason = "step_failed"
	elseif lead ~= "working" and p.pending and steps > 0 then
		reason = "plan_ready"
	elseif c and not (lead == "working" or (p.has_plan and steps == 0) or has.working) then
		reason = "review"
	end
	out.needs = reason
	out.column = if reason then "needs" elseif c or has.working then "working" else "todo"
	out.progress = if steps > 0 then { done = done, total = total } else nil
	return out
end
local old_host = false
local function publish()
	if old_host then
		return
	end
	local chats = store.chats:peek()
	local out = {}
	for _, t in tasks do
		table.insert(out, as_host(t, chats))
	end
	store.list:set({ state = "ready", tasks = out })
end
ui.effect(function()
	sessions.now:get()
	publish()
end)
local function at(id: string): number?
	for i, t in tasks do
		if t.id == id then
			return i
		end
	end
	return nil
end
local function done(v: any, then_: (() -> ())?): any
	return {
		after = function(_self: any, ok: ((any) -> ())?, err: ((any) -> ())?)
			if refuse then
				if err then
					err(refuse)
				end
				return
			end
			if ok then
				ok(v)
			end
			if then_ then
				then_()
			end
		end,
	}
end

local next_id = 100
store.host.create = function(args: any): any
	next_id += 1
	local id = `n{next_id}`
	local images = if args.images and #args.images > 0 then ` images={table.concat(args.images, "+")}` else ""
	local depends = if args.depends and #args.depends > 0 then ` depends={table.concat(args.depends, "+")}` else ""
	note(`create:{args.title}{if args.notes and args.notes ~= "" then "|" .. args.notes else ""}{if args.due then "@" .. args.due else ""}{images}{depends}`)
	if refuse then
		return done(nil)
	end
	-- (The engine answers before its watch brings the task, as the real one may.)
	return done(id, function()
		ui.after(50, function()
			table.insert(tasks, 1, task(id, args.title, "a0", { notes = args.notes or "", due = args.due, created = NOW * 1000 }))
			publish()
		end)
	end)
end
store.host.update = function(args: any): any
	local parts = {}
	for _, k in { "title", "notes", "due", "project", "completed", "images" } do
		if args[k] ~= nil then
			local v = args[k]
			table.insert(parts, `{k}={if type(v) == "table" then table.concat(v, "+") else tostring(v)}`)
		end
	end
	note(`update:{args.id}:{table.concat(parts, ";")}`)
	local i = at(args.id)
	if i then
		local t = table.clone(tasks[i])
		for _, k in { "title", "notes", "images" } do
			if args[k] ~= nil then
				t[k] = args[k]
			end
		end
		for _, k in { "due", "project" } do
			if args[k] ~= nil then
				t[k] = if args[k] == "" then nil else args[k]
			end
		end
		if args.completed ~= nil then
			t.completed = if args.completed then NOW * 1000 else nil
		end
		tasks[i] = t
		publish()
	end
	return done(nil)
end
store.host.delete = function(args: any): any
	note(`delete:{args.id}`)
	local i = at(args.id)
	if i then
		table.remove(tasks, i)
		publish()
	end
	return done(nil)
end
store.host.assign = function(args: any): any
	note(`assign:{args.id}:{args.chat}:{tostring(args.send)}`)
	local i = at(args.id)
	if i then
		local t = table.clone(tasks[i])
		t.chat = if args.chat == "" then nil else args.chat
		t.send_error = nil
		tasks[i] = t
		publish()
	end
	return done(nil)
end
store.host.start = function(args: any): any
	note(`start:{args.id}:{args.picks and args.picks.model or ""}{if args.orchestrate then ":plan" else ""}{if args.same_agents then ":same" else ""}{if args.approve == false then ":runs" else ""}{if args.project then ` in {args.project}` else ""}`)
	return done("new-chat")
end
store.host.today = function(): any
	note("today")
	local ms = NOW * 1000
	return done({
		runs = {
			{ chatId = "a2", title = "Write the docs", startedAtMs = ms - 600000, durationMs = 60000, status = "complete" },
			{ chatId = "a3", startedAtMs = ms - 1200000, durationMs = 1000, status = "aborted" },
			{ chatId = "a1", startedAtMs = ms - 60000, status = "running" },
		},
		waits = { { chatId = "b1", startedAtMs = ms - 900000, endedAtMs = ms - 300000 } },
	})
end
store.host.image = function(args: any): any
	return done({ src = args.path })
end
local slow_upload = false
store.host.upload = function(args: any): any
	note(`upload:{args.path}`)
	if slow_upload then
		return {
			after = function(_self: any, ok: ((any) -> ())?)
				ui.after(300, function()
					if ok then
						ok(`/uploads/{args.path}`)
					end
				end)
			end,
		}
	end
	return done(`/uploads/{args.path}`)
end

local Invocations = require("@app/screens/chat/invocations")
local invocations_stub = Invocations :: any
invocations_stub.read = function(harness: string, target: any): any
	note(`invocations:{harness}:{if target.home then "home" else target.chat or target.space or "?"}`)
	return done({
		rows = {
			{ kind = "command", name = "review", label = "/review", detail = "Review the diff", link = "[/review](zeron-invoke:7b7d)", task = true },
			{ kind = "command", name = "btw", label = "/btw", detail = "Side chat", link = "/btw", plain = true },
			{ kind = "command", name = "clear", label = "/clear", detail = "Clear the conversation", link = "[/clear](zeron-invoke:7b7d)" },
			{ kind = "skill", name = "code-review", label = "Code Review", detail = "Careful review", link = "[$code-review](zeron-invoke:7b7d)", path = "/s/code-review/SKILL.md", task = true },
		},
	})
end
local capture = require("@app/screens/tasks/capture")
ui.command { id = "fixture.capture_image", title = "Fixture: a picture pasted into quick capture", menu = "Fixture", run = function()
	capture.add_files({ { name = "circles.png", path = "assets/pictures/circles.png" }, { name = "notes.txt", path = "assets/notes.txt" } })
end }
ui.command { id = "fixture.capture_image_slow", title = "Fixture: a picture pasted, its upload slow", menu = "Fixture", run = function()
	slow_upload = true
	capture.add_files({ { name = "wide.png", path = "assets/pictures/wide.png" } })
	slow_upload = false
end }

ui.command { id = "fixture.tasks_many", title = "Fixture: a thousand tasks", menu = "Fixture", run = function()
	tasks = {}
	for i = 1, 1000 do
		table.insert(tasks, task(`m{i}`, `Task {i}`, string.format("k%05d", i)))
	end
	publish()
end }
ui.command { id = "fixture.tasks_old_host", title = "Fixture: a host without tasks", menu = "Fixture", run = function()
	for k, f in real_host do
		store.host[k] = f
	end
	local call = store.raw_call
	store.raw_call = function(name: string, ...: any): any
		if string.sub(name, 1, 6) == "tasks." then
			error(`no host action named {name}`)
		end
		return call(name, ...)
	end
	old_host = true
	store.list:set({ state = "idle", tasks = {} })
end }
ui.command { id = "fixture.t4_linked", title = "Fixture: the onboarding copy's title holds a command's link", menu = "Fixture", run = function()
	local i = at("t4") :: number
	local t = table.clone(tasks[i])
	t.title = "[/review](zeron-invoke:7b7d) the copy"
	tasks[i] = t
	publish()
end }
ui.command { id = "fixture.tasks_refuse", title = "Fixture: the engine refuses", menu = "Fixture", run = function()
	refuse = "Start the task on the device that owns its project"
end }
ui.command { id = "fixture.tasks_accept", title = "Fixture: the engine accepts again", menu = "Fixture", run = function()
	refuse = nil
end }
-- (A sessions change the shell fixture does not make: a chat's fields set as the host would publish them.)
local function chat_set(id: string, fields: { [string]: any })
	local now = table.clone(sessions.now:peek()) :: any
	for _, key in { "chats", "archived" } do
		now[key] = table.clone(now[key] or {})
		for i, c in now[key] do
			if c.id == id then
				local next = table.clone(c)
				for k, v in fields do
					next[k] = if v == false then nil else v
				end
				now[key][i] = next
			end
		end
	end
	sessions.now:set(now)
end
ui.command { id = "fixture.a2_errored_seen", title = "Fixture: Write the docs' last turn errored, seen", menu = "Fixture", run = function()
	chat_set("a2", { errored = true, status = false })
end }
ui.command { id = "fixture.a1_finished", title = "Fixture: Fix the build finishes, seen", menu = "Fixture", run = function()
	chat_set("a1", { status = false })
end }
ui.command { id = "fixture.archive_a1", title = "Fixture: Fix the build archived", menu = "Fixture", run = function()
	sessions.archive("a1")
end }
ui.command { id = "fixture.b1_question", title = "Fixture: Ship the release asks a question to answer here", menu = "Fixture", run = function()
	ui.shared("chat.waiting", {} :: { any }):set({ { key = "b1:r1", chat = "b1", request = "r1", kind = "ask", ask = {}, steps = {} } })
end }
local function plan_t6(plan: any)
	for i, t in tasks do
		if t.id == "t6" then
			local m = table.clone(t)
			m.plan = plan
			tasks[i] = m
		end
	end
	publish()
end
ui.command { id = "fixture.mission", title = "Fixture: Write the release notes is a mission", menu = "Fixture", run = function()
	plan_t6({ has_plan = true, pending = true, states = {} })
end }
ui.command { id = "fixture.mission_steps", title = "Fixture: the mission's plan has steps, one working", menu = "Fixture", run = function()
	plan_t6({ has_plan = true, pending = false, states = { "working", "done" } })
end }
ui.command { id = "fixture.mission_plan_ready", title = "Fixture: the mission's plan waits on you", menu = "Fixture", run = function()
	plan_t6({ has_plan = true, pending = true, states = { "readyForReview" } })
end }
local t6_read = ui.state("tasks_t6", "")
ui.command { id = "fixture.t6_read", title = "Fixture: read Write the release notes' state", menu = "Fixture", run = function()
	local t = store.find("t6") :: any
	local s = store.state(t)
	local p = t.progress
	t6_read:set(`{s.column}|{s.reason or ""}|{if p then `{p.done}/{p.total}` else ""}`)
end }
ui.command { id = "fixture.t4_eleven_images", title = "Fixture: Draft the onboarding copy holds eleven images", menu = "Fixture", run = function()
	for i, t in tasks do
		if t.id == "t4" then
			local m = table.clone(t)
			m.images = {}
			for n = 1, 11 do
				table.insert(m.images, `/uploads/{n}.png`)
			end
			tasks[i] = m
		end
	end
	publish()
end }
ui.command { id = "fixture.t2_broken_images", title = "Fixture: Port the sidebar fold holds a file that isn't a picture and one that's gone", menu = "Fixture", run = function()
	for i, t in tasks do
		if t.id == "t2" then
			local m = table.clone(t)
			m.images = { "tasks_fixture.luau", "assets/pictures/gone.png" }
			tasks[i] = m
		end
	end
	publish()
end }
ui.command { id = "fixture.mention_title", title = "Fixture: a task titled with a mention", menu = "Fixture", run = function()
	for i, t in tasks do
		if t.id == "t4" then
			local m = table.clone(t)
			m.title = "Draft [Fix \\[PR\\] copy](zeron-chat:a3) for [Atlas](zeron-task:t2)"
			tasks[i] = m
		end
	end
	publish()
end }

-- (The day sums at a moment, in a zone `tasks_days_offset` seconds east of UTC, or the real one at nil: read back as `tasks_days`.)
local days_offset = ui.state("tasks_days_offset", nil :: number?)
local days_at = ui.state("tasks_days_at", 1794200000)
local days = ui.state("tasks_days", "")
ui.command { id = "fixture.days", title = "Fixture: the day sums", menu = "Fixture", run = function()
	local off = days_offset:peek()
	local now = days_at:peek()
	if off then
		store.date = function(fmt: string, at: number?): any
			local t = at or os.time()
			if string.sub(fmt, 1, 1) == "!" then
				return os.date(fmt, t)
			end
			return os.date("!" .. fmt, t + (off :: number))
		end
	end
	local start = store.day_start(now) / 1000
	local s = store.date("*t", start) :: any
	local due = store.due_label({ due = "2026-12-03" } :: any, now)
	days:set(`{s.hour}:{s.min}|{tostring(start <= now and now < start + 86400)}|{store.today(now)}|{store.tomorrow(now)}|{store.work_day(now)}|{due}`)
	store.date = os.date
end }

publish()
return node
