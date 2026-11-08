-- Tasks from src/tasks.rs `tasks.list`; each host action under `pcall`, as an old host has none.

local sessions = require("@account/sessions")
local locale = require("@ds/locale")
local toast = require("@ds/toast")

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
	plan_pending: boolean?,
	steps: { { chat: string?, outcome: string?, ended: string? } }?,
	depends: { string },
}

M.list = ui.shared("tasks.list", { state = "idle", tasks = {} } :: { state: string, tasks: { Task } })

-- (The host's call, which a test swaps for an old host's.)
M.raw_call = ui.call
local function call(name: string, args: any): any
	local ok, p = pcall(M.raw_call, name, args)
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

local function words(err: any): string
	return if type(err) == "table" or type(err) == "userdata" then tostring(err.message) else tostring(err)
end

local told_absent = false
-- (A host without the action says so once; an engine's refusal is toasted in its own words.)
local function settle(p: any, done: ((any) -> ())?, fail: ((any) -> ())?): any
	if p == nil then
		M.absent:set(true)
		if not told_absent then
			told_absent = true
			toast.show({ title = locale.s("tasks.no_host"), tone = "warning" })
		end
		if fail then
			fail(nil)
		end
		return nil
	end
	p:after(function(v: any)
		if done then
			done(v)
		end
	end, function(err: any)
		local said = words(err)
		M.failed:set(said)
		toast.show({ title = said, tone = "danger" })
		if fail then
			fail(err)
		end
	end)
	return p
end

function M.create(args: { title: string, notes: string?, project: string?, due: string?, images: { string }?, depends: { string }? }, done: ((string) -> ())?, fail: ((any) -> ())?): any
	local title = string.match(args.title or "", "^%s*(.-)%s*$") or ""
	if title == "" then
		return nil
	end
	local a = table.clone(args) :: any
	a.title = title
	return settle(M.host.create(a), done, fail)
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

function M.start(id: string, picks: any, project: string?, done: ((string) -> ())?, fail: ((any) -> ())?): any
	return settle(M.host.start({ id = id, picks = picks, project = project }), done, fail)
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

-- Every chat by id, archived and side chats too, as the shipping app's `linked_chat` looks in all of them; built once a publish.
M.chats = ui.computed(function(): { [string]: any }
	local now = sessions.now:get()
	local by: { [string]: any } = {}
	for _, c in now.archived or {} do
		by[c.id] = c
	end
	for _, list in now.children or {} do
		for _, c in list do
			by[c.id] = c
		end
	end
	for _, c in now.chats or {} do
		by[c.id] = c
	end
	return by
end)
local projects = ui.computed(function(): { [string]: string }
	local by: { [string]: string } = {}
	for _, p in sessions.now:get().projects or {} do
		by[p.id] = p.name
	end
	return by
end)

local function chat_of(id: string?): any
	return if id then M.chats:get()[id] else nil
end
M.chat_of = chat_of

function M.project_name(id: string?): string?
	return if id then projects:get()[id] else nil
end

-- A step's state (proto `step_status`): "done", "skipped", "planned", "working", "needs" or "failed".
local function step_status(s: any, chats: { [string]: any }): string
	if s.outcome == "done" or s.outcome == "skipped" then
		return s.outcome
	end
	local c = if s.chat then chats[s.chat] else nil
	if not c then
		return "planned"
	elseif c.status == "working" then
		return "working"
	elseif c.status == "input" then
		return "needs"
	elseif c.delegation then
		return if c.delegation == "running" then "working" elseif c.delegation == "completed" then "planned" else "needs"
	elseif s.ended == "completed" then
		return "planned"
	elseif s.ended == "failed" then
		return "failed"
	elseif s.ended == "stopped" then
		return "needs"
	end
	return "working"
end

-- A task's one state, as proto `task_status` and `needs_you`: "todo", "working", "needs" or "done", the linked chat (nil if none or gone) and why it needs you.
export type State = { column: string, chat: any, reason: string?, gone: boolean }
function M.state(t: Task): State
	local chats = M.chats:get()
	local c = if t.chat then chats[t.chat] else nil
	if t.completed then
		return { column = "done", chat = c, gone = false }
	end
	local gone = t.chat ~= nil and c == nil
	local lead = if c then c.status or "idle" else nil
	local has: { [string]: boolean } = {}
	local steps = 0
	for _, s in t.steps or {} do
		has[step_status(s, chats)] = true
		steps += 1
	end
	local reason: string? = nil
	if t.send_error then
		reason = "not_sent"
	elseif lead == "input" then
		reason = "asks"
	elseif lead == "failed" then
		reason = "failed"
	elseif has.needs then
		reason = "step_asks"
	elseif lead ~= "working" and has.failed then
		reason = "step_failed"
	elseif lead ~= "working" and t.plan_pending and steps > 0 then
		reason = "plan_ready"
	elseif c and not (lead == "working" or (t.plan and steps == 0) or has.working) then
		reason = "review"
	end
	if reason then
		return { column = "needs", chat = c, reason = reason, gone = gone }
	elseif c or has.working then
		return { column = "working", chat = c, gone = false }
	end
	return { column = "todo", chat = nil, gone = gone }
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

-- (Local time, which a test swaps for a zone east or west of UTC.)
M.date = os.date
local function offset(at: number): number
	local t = M.date("*t", at) :: any
	return os.time({ year = t.year, month = t.month, day = t.day, hour = t.hour, min = t.min, sec = t.sec }) - at
end

-- The calendar day, as the shipping app's Tasks page and card.
function M.today(now: number?): string
	return M.date("%Y-%m-%d", now or os.time()) :: string
end

-- The work day, as the shipping app's Inbox: it turns at 4 in the morning.
local DAY_STARTS = 4
function M.work_day(now: number?): string
	return M.date("%Y-%m-%d", (now or os.time()) - DAY_STARTS * 3600) :: string
end
function M.day_start(now: number?): number
	local at = (now or os.time()) - DAY_STARTS * 3600
	local t = M.date("*t", at) :: any
	return (os.time({ year = t.year, month = t.month, day = t.day, hour = DAY_STARTS }) - offset(at)) * 1000
end

-- A date's noon in UTC, read back in UTC, so a day sum never crosses a zone.
local function day_of(s: string): number?
	local y, m, d = string.match(s, "^(%d+)-(%d+)-(%d+)$")
	if not y then
		return nil
	end
	return os.time({ year = tonumber(y) :: number, month = tonumber(m) :: number, day = tonumber(d) :: number, hour = 12 })
end
function M.tomorrow(now: number?): string
	return os.date("!%Y-%m-%d", (day_of(M.today(now)) :: number) + 86400) :: string
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
	local d = os.date("!*t", day) :: any
	local month = locale.s(`tasks.month.m{d.month}`)
	if d.year == (os.date("!*t", today) :: any).year then
		return locale.s("tasks.due.on", { month = month, day = tostring(d.day) }), late
	end
	return locale.s("tasks.due.on_year", { month = month, day = tostring(d.day), year = tostring(d.year) }), late
end

-- Today's tasks (proto `today_tasks`, on the work day): open ones handed to a chat or due by today, in their order, then those done since the day began.
function M.today_tasks(now: number?): { Task }
	local p = M.parts:get()
	local today = M.work_day(now)
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

-- A title with its mentions as their words (composer.rs `plain_references`): `[x](zeron-task:…)` reads "x".
function M.plain(s: string): string
	if not string.find(s, "](zeron-", 1, true) then
		return s
	end
	local out: { string } = {}
	local i = 1
	while true do
		local close, after = string.find(s, "%]%(zeron%-[%w%-]*:[^%)]*%)", i)
		if not close then
			break
		end
		local open: number? = nil
		local j = close - 1
		while j >= i do
			local ch = string.sub(s, j, j)
			if (ch == "[" or ch == "]") and string.sub(s, j - 1, j - 1) ~= "\\" then
				open = if ch == "[" then j else nil
				break
			end
			j -= 1
		end
		if open then
			table.insert(out, string.sub(s, i, open - 1))
			table.insert(out, (string.gsub(string.sub(s, open + 1, close - 1), "\\([%[%]\\])", "%1")))
		else
			table.insert(out, string.sub(s, i, after :: number))
		end
		i = (after :: number) + 1
	end
	table.insert(out, string.sub(s, i))
	return table.concat(out)
end

-- A task's title on one line, its mentions as their words.
function M.title_of(t: Task): string
	local s = string.match(string.gsub(M.plain(t.title or ""), "%s+", " "), "^%s*(.-)%s*$") or ""
	return if s == "" then locale.s("tasks.untitled") else s
end

return M
