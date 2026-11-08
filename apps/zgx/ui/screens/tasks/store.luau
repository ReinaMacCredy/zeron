-- Tasks from src/tasks.rs `tasks.list`; each host action under `pcall`, as an old host has none.

local sessions = require("@account/sessions")
local locale = require("@ds/locale")

ui.theme.tokens {
	zeron = {
		tasks = {
			wash = "alpha({accent}, 0.12)",
			wash_hover = "alpha({accent}, 0.18)",
			remove = "alpha(#000000, 0.67)",
			remove_hover = "alpha(#000000, 0.8)",
		},
	},
}

local M = {}

export type Task = {
	id: string,
	title: string,
	notes: string,
	project: string?,
	due: string?,
	chat: string?,
	completed: number?,
	order: string,
	created: number,
	images: { string },
	send_error: string?,
	plan: boolean?,
	depends: { string },
}

M.list = ui.shared("tasks.list", { state = "idle", tasks = {} } :: { state: string, tasks: { Task } })

local function call(name: string, args: any): any
	local ok, p = pcall(ui.call, name, args)
	return if ok then p else nil
end

M.host = {
	create = function(args: any): any
		return call("tasks.create", args)
	end,
	update = function(args: any): any
		return call("tasks.update", args)
	end,
	delete = function(args: any): any
		return call("tasks.delete", args)
	end,
	assign = function(args: any): any
		return call("tasks.assign", args)
	end,
	start = function(args: any): any
		return call("tasks.start", args)
	end,
	today = function(args: any): any
		return call("tasks.today", args)
	end,
	image = function(args: any): any
		return call("tasks.image", args)
	end,
	upload = function(args: any): any
		return call("tasks.upload_image", args)
	end,
}

M.absent = ui.state("tasks_absent", false)
M.failed = ui.state("tasks_failed", nil :: string?)

-- (A write the host has no action for, or the engine refused: said once.)
local function settle(p: any, done: ((any) -> ())?): any
	if p == nil then
		M.absent:set(true)
		return nil
	end
	p:after(function(v: any)
		if done then
			done(v)
		end
	end, function(err: any)
		M.failed:set(tostring(err))
	end)
	return p
end

function M.create(args: { title: string, notes: string?, project: string?, due: string?, images: { string }?, depends: { string }? }, done: ((string) -> ())?): any
	local title = string.match(args.title or "", "^%s*(.-)%s*$") or ""
	if title == "" then
		return nil
	end
	local a = table.clone(args) :: any
	a.title = title
	return settle(M.host.create(a), done)
end

function M.update(id: string, patch: { [string]: any }): any
	local a = table.clone(patch)
	a.id = id
	return settle(M.host.update(a))
end

function M.set_done(id: string, done: boolean): any
	return M.update(id, { completed = done })
end

function M.delete(id: string): any
	return settle(M.host.delete({ id = id }))
end

function M.assign(id: string, chat: string, send: boolean): any
	return settle(M.host.assign({ id = id, chat = chat, send = send }))
end

function M.start(id: string, picks: any, project: string?, done: ((string) -> ())?): any
	return settle(M.host.start({ id = id, picks = picks, project = project }), done)
end

-- Open first in their order, then done, latest first (proto `split_tasks`).
function M.split(tasks: { Task }): ({ Task }, { Task })
	local open, done = {}, {}
	for _, t in tasks do
		table.insert(if t.completed then done else open, t)
	end
	table.sort(open, function(a: Task, b: Task): boolean
		if a.order ~= b.order then
			return a.order < b.order
		elseif a.created ~= b.created then
			return a.created < b.created
		end
		return a.id < b.id
	end)
	table.sort(done, function(a: Task, b: Task): boolean
		if a.completed ~= b.completed then
			return (a.completed :: number) > (b.completed :: number)
		end
		return a.id < b.id
	end)
	return open, done
end

M.parts = ui.computed(function(): { open: { Task }, done: { Task }, by_id: { [string]: Task } }
	local all = M.list:get().tasks or {}
	local open, done = M.split(all)
	local by_id: { [string]: Task } = {}
	for _, t in all do
		by_id[t.id] = t
	end
	return { open = open, done = done, by_id = by_id }
end)

function M.find(id: string?): Task?
	if not id then
		return nil
	end
	return M.parts:get().by_id[id]
end

local function chat_of(id: string?): any
	if not id then
		return nil
	end
	for _, c in sessions.now:get().chats do
		if c.id == id then
			return c
		end
	end
	return nil
end
M.chat_of = chat_of

function M.project_name(id: string?): string?
	if not id then
		return nil
	end
	for _, p in sessions.now:get().projects do
		if p.id == id then
			return p.name
		end
	end
	return nil
end

-- A task's one state, as the shipping app's `task_status` and `needs_you`: "todo", "working", "needs" or "done", the linked chat (nil if none or gone) and why it needs you.
export type State = { column: string, chat: any, reason: string?, gone: boolean }
function M.state(t: Task): State
	if t.completed then
		return { column = "done", chat = chat_of(t.chat), gone = false }
	end
	local c = chat_of(t.chat)
	local gone = t.chat ~= nil and c == nil
	if t.send_error then
		return { column = "needs", chat = c, reason = "not_sent", gone = gone }
	end
	if not c then
		return { column = "todo", chat = nil, gone = gone }
	end
	if c.status == "working" then
		return { column = "working", chat = c, gone = false }
	elseif c.status == "input" then
		return { column = "needs", chat = c, reason = "asks", gone = false }
	elseif c.status == "failed" then
		return { column = "needs", chat = c, reason = "failed", gone = false }
	end
	return { column = "needs", chat = c, reason = "review", gone = false }
end

M.COLUMNS = { "todo", "working", "needs", "done" }
M.TONES = { todo = "$text.tertiary", working = "$zeron.status.working", needs = "$warning", done = "$success" }

-- A linked thread's word (zeron_doc `linked_label`).
function M.linked_word(c: any): string
	local s = if c then c.status else nil
	if s == "working" then
		return locale.s("tasks.linked.working")
	elseif s == "input" then
		return locale.s("tasks.linked.asks")
	elseif s == "failed" then
		return locale.s("tasks.linked.failed")
	end
	return locale.s("tasks.linked.ready")
end

-- The work day, as the shipping app's: it turns at 4 in the morning.
local DAY_STARTS = 4
function M.today(now: number?): string
	local at = (now or os.time()) - DAY_STARTS * 3600
	return os.date("%Y-%m-%d", at) :: string
end
function M.day_start(now: number?): number
	local t = os.date("*t", (now or os.time()) - DAY_STARTS * 3600) :: any
	return os.time({ year = t.year, month = t.month, day = t.day, hour = DAY_STARTS }) * 1000
end
local function day_of(s: string): number?
	local y, m, d = string.match(s, "^(%d+)-(%d+)-(%d+)$")
	if not y then
		return nil
	end
	return os.time({ year = tonumber(y) :: number, month = tonumber(m) :: number, day = tonumber(d) :: number, hour = 12 })
end
function M.tomorrow(now: number?): string
	return os.date("%Y-%m-%d", (day_of(M.today(now)) :: number) + 86400) :: string
end

-- "due today", "due tomorrow", "due yesterday", "due Nov 3", "due Nov 3, 2025" (logic.rs `due_label`), and whether it's late.
function M.due_label(t: Task, now: number?): (string?, boolean)
	if not t.due then
		return nil, false
	end
	local day, today = day_of(t.due), day_of(M.today(now))
	if not day or not today then
		return nil, false
	end
	local diff = math.floor((day - today) / 86400 + 0.5)
	local late = diff < 0 and not t.completed
	if diff == 0 then
		return locale.s("tasks.due.today"), false
	elseif diff == 1 then
		return locale.s("tasks.due.tomorrow"), false
	elseif diff == -1 then
		return locale.s("tasks.due.yesterday"), late
	end
	local d = os.date("*t", day) :: any
	local month = locale.s(`tasks.month.m{d.month}`)
	if d.year == (os.date("*t", today) :: any).year then
		return locale.s("tasks.due.on", { month = month, day = tostring(d.day) }), late
	end
	return locale.s("tasks.due.on_year", { month = month, day = tostring(d.day), year = tostring(d.year) }), late
end

-- Today's tasks (proto `today_tasks`): open ones handed to a chat or due by today, in their order, then those done since the day began.
function M.today_tasks(now: number?): { Task }
	local p = M.parts:get()
	local today = M.today(now)
	local start = M.day_start(now)
	local out: { Task } = {}
	for _, t in p.open do
		if chat_of(t.chat) ~= nil or (t.due ~= nil and t.due <= today) then
			table.insert(out, t)
		end
	end
	for _, t in p.done do
		if (t.completed :: number) >= start then
			table.insert(out, t)
		end
	end
	return out
end

-- Quick capture's tasks, "New" until Tasks has shown them: `unseen` until it opens, `seen` while it shows, gone once it's left.
M.captured = ui.persist("tasks_captured", {} :: { [string]: string })
function M.note_captured(id: string)
	local c = table.clone(M.captured:peek())
	c[id] = "unseen"
	M.captured:set(c)
end
M.unseen = ui.computed(function(): number
	local n = 0
	local by_id = M.parts:get().by_id
	for id, s in M.captured:get() do
		if s == "unseen" and by_id[id] and not by_id[id].completed then
			n += 1
		end
	end
	return n
end)
function M.saw_captured(leaving: boolean)
	local c = M.captured:peek()
	local next: { [string]: string } = {}
	local changed = false
	for id, s in c do
		if leaving then
			changed = true
		else
			next[id] = "seen"
			changed = changed or s ~= "seen"
		end
	end
	if changed then
		M.captured:set(next)
	end
end
function M.is_new(id: string): boolean
	return M.captured:get()[id] ~= nil
end

-- A task's images here, as files (`tasks.image`), each asked for once.
local images = ui.state("tasks_images", {} :: { [string]: string })
local asked: { [string]: boolean } = {}
function M.image_src(path: string): string?
	local got = images:get()[path]
	if got then
		return if got == "" then nil else got
	end
	if not asked[path] then
		asked[path] = true
		local p = M.host.image({ path = path })
		if p then
			p:after(function(v: any)
				images:set_key(path, if v and v.src then v.src else "")
			end, function()
				images:set_key(path, "")
			end)
		end
	end
	return nil
end
function M.image_failed(path: string): boolean
	return images:get()[path] == ""
end

M.IMAGES_MAX = 12

return M
