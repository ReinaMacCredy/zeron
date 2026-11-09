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
local function publish()
	store.list:set({ state = "ready", tasks = table.clone(tasks) })
end
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
	note(`create:{args.title}{if args.notes and args.notes ~= "" then "|" .. args.notes else ""}{if args.due then "@" .. args.due else ""}`)
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
	note(`start:{args.id}:{args.picks and args.picks.model or ""}`)
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
store.host.upload = function(args: any): any
	note(`upload:{args.path}`)
	return done(`/uploads/{args.path}`)
end

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
	store.list:set({ state = "idle", tasks = {} })
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
ui.command { id = "fixture.mission", title = "Fixture: Write the release notes is a mission", menu = "Fixture", run = function()
	for i, t in tasks do
		if t.id == "t6" then
			local m = table.clone(t)
			m.plan = true
			tasks[i] = m
		end
	end
	publish()
end }
ui.command { id = "fixture.mission_steps", title = "Fixture: the mission's plan has steps, one working", menu = "Fixture", run = function()
	for i, t in tasks do
		if t.id == "t6" then
			local m = table.clone(t)
			m.plan, m.plan_pending = true, false
			m.steps = { { chat = "a1" }, { outcome = "done" } }
			tasks[i] = m
		end
	end
	publish()
end }
ui.command { id = "fixture.mission_plan_ready", title = "Fixture: the mission's plan waits on you", menu = "Fixture", run = function()
	for i, t in tasks do
		if t.id == "t6" then
			local m = table.clone(t)
			m.plan, m.plan_pending = true, true
			m.steps = { { chat = "a3", ended = "completed" } }
			tasks[i] = m
		end
	end
	publish()
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
