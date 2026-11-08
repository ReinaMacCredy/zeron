-- The shell fixture with a stub task host that edits `tasks.list` and records `tasks_calls`.

local node = require("./shell_fixture")
local store = require("@app/screens/tasks/store")

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
local function done(v: any): any
	return {
		after = function(_self: any, ok: ((any) -> ())?)
			if ok then
				ok(v)
			end
		end,
	}
end

local next_id = 100
store.host.create = function(args: any): any
	next_id += 1
	local id = `n{next_id}`
	note(`create:{args.title}{if args.notes and args.notes ~= "" then "|" .. args.notes else ""}{if args.due then "@" .. args.due else ""}`)
	table.insert(tasks, 1, task(id, args.title, "a0", { notes = args.notes or "", due = args.due, created = NOW * 1000 }))
	publish()
	return done(id)
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
	for k in store.host do
		store.host[k] = function(): any
			return nil
		end
	end
	store.list:set({ state = "idle", tasks = {} })
end }

publish()
return node
