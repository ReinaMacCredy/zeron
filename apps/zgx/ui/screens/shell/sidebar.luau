-- The main window's sidebar: the design system's (ds.Sidebar) on the
-- engine's sessions (@account/sessions). What it shows follows them in
-- place: a session's state, title or time moving reaches its row, one
-- made or archived moves the rest aside. Where sessions are put by hand
-- (dragged to Pinned, into a section of your own) goes to the engine, and
-- comes back as every device has it.
--
-- `Sidebar { collapsed = state, island = bool? }`: the sidebar, or
-- (`island`) its rail. The two share their view options, their places and
-- the session chosen, which is the route's (screens/shell/route).

local ds = require("@ds")
local ContextMenu = require("@ds/context_menu")
local sessions = require("@account/sessions")
local project_calls = require("@account/projects")
-- The project editor (screens/shell/project_editor, its emoji picker and
-- the emoji's data): loaded as it's first opened, not with the window, and
-- its dialog placed from then on (`project_editor_used`, kept across a
-- reload, so a dialog open then is there again).
local project_editor_used = ui.state("shell_project_editor_used", false)
local function project_editor(): any
	return require("@app/screens/shell/project_editor")
end
local auth = require("@account/auth")
local status = require("@ds/status")
local Pills = require("@app/screens/chat/pills")
local route = require("@app/screens/shell/route")
local rail = require("@app/screens/shell/rail")
local view = require("@app/screens/shell/view")
local fit = require("@app/screens/shell/fit")
local motion = require("@ds/motion")
local boundary = require("@app/screens/shell/boundary")
local top_bar = require("@app/screens/shell/top_bar")
local locale = require("@ds/locale")
local ds_session_row = require("@ds/session_row")
local Glass = require("@ds/glass")
local activity = require("@app/screens/shell/activity")

export type Props = {
	collapsed: State<boolean>,
	-- The most it may take, and whether a change follows at once (the
	-- window's fit: screens/shell/fit).
	limit: (() -> number)?,
	direct: (() -> boolean)?,
	island: boolean?,
	-- Room left at its top for the title bar laid over it.
	top: number?,
}

-- What the sidebar and its rail share: the view options (Zeron's Organize,
-- Sort and Show), its width, where sessions are put, and the session open.
-- Kept across launches, but the session open (the route's) and the places
-- (the engine's).
local shared = {
	compact = ui.persist("shell_sidebar_compact", false),
	inbox = ui.persist("shell_sidebar_inbox", false),
	active_first = ui.persist("shell_sidebar_active_first", false),
	-- (One list, "Sessions", as Zeron's; by project is a view option.)
	-- How it groups, as levels in order (`{ "project", "device" }`: by
	-- project, then device; none, one list). By project, then device, until
	-- changed. Kept as one choice before ("project"): read and kept as its
	-- levels from now (below).
	organize = ui.persist("shell_sidebar_organize", { "project", "device" } :: any),
	-- Both levels as the view menu's rows stand (dragged by their handles):
	-- Organize is those on, in this order.
	organize_order = ui.persist("shell_sidebar_organize_order", { "project", "device" }),
	sort = ui.persist("shell_sidebar_sort", "updated"),
	width = ui.persist("shell_sidebar_width", fit.SIDEBAR_DEFAULT),
	details = {
		branch = ui.persist("shell_sidebar_show_branch", true),
		pr = ui.persist("shell_sidebar_show_pr", true),
		harness = ui.persist("shell_sidebar_show_harness", true),
		project_icon = ui.persist("shell_sidebar_show_project_icon", true),
		location = ui.persist("shell_sidebar_show_location", true),
		-- (Zeron's "Show archived": off until asked for.)
		archived = ui.persist("shell_sidebar_show_archived", true),
		-- (Zeron's project filter over the list: off until asked for.)
		project_filter = ui.persist("shell_sidebar_show_project_filter", true),
		working = ui.persist("shell_sidebar_show_working", false),
		-- The projects with none but archived sessions, folded into one row
		-- at the projects' end: listed there (closed until it's opened), or
		-- not at all (until asked for).
		empty_projects = ui.persist("shell_sidebar_show_empty_projects", false),
		-- A collapsed group still lists its sessions working, asking or done
		-- unseen: on, so folding a group away never hides what needs you.
		keep_active = ui.persist("shell_sidebar_keep_active", true),
	},
	-- The groups showing all their sessions ("Show N more"): until the app
	-- quits (a reload keeps them).
	expanded = ui.state("shell_sidebar_expanded", {} :: { [string]: boolean }),
	-- The project the list is narrowed to (nil: all of them), while the
	-- filter shows.
	project = ui.persist("shell_sidebar_project", nil :: string?),
	places = ui.state("shell_places", { pinned = {}, sections = {} } :: sessions.Places),
	selected = ui.state("shell_selected", nil :: string?),
	-- Its view, kept here so a sidebar made again comes back as it was.
	offset = ui.state("shell_sidebar_offset", 0),
	folded = ui.persist("shell_sidebar_folded", { ["group:inbox:working"] = true, ["group:inbox:settled"] = true } :: { [string]: boolean }),
}

-- Scroll probe (for now: "it jumps back a step every few steps"): the
-- list's offset moving back within a moment of moving down is printed
-- (`sidebar-probe:` on the app's stderr), with how often the sidebar was
-- built and the sessions changed. It reads and prints; it writes nothing.
local probe = { last = 0, at = 0, dir = 0, builds = 0, built_at = 0, sessions_at = 0 }
ui.effect(function()
	sessions.now:get()
	probe.sessions_at = ui.now()
end)
ui.effect(function()
	local offset = shared.offset:get()
	local t = ui.now()
	local d = offset - probe.last
	if d < -0.5 and probe.dir > 0 and t - probe.at < 300 then
		print(
			`sidebar-probe: back {math.floor(-d + 0.5)} pt to {math.floor(offset)} ({math.floor(t - probe.at)} ms after a step down);`
				.. ` sidebar built {probe.builds}x, last {math.floor(t - probe.built_at)} ms ago;`
				.. ` sessions changed {math.floor(t - probe.sessions_at)} ms ago`
		)
	end
	if math.abs(d) > 0.5 then
		probe.dir = if d > 0 then 1 else -1
	end
	probe.last, probe.at = offset, t
end)

-- The same, field by field.
local function same(a: any, b: any): boolean
	if type(a) ~= "table" or type(b) ~= "table" then
		return a == b
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

-- The places, as the sidebar edits them (no fold of the engine's).
local function plain(p: sessions.Places): sessions.Places
	local sections = {}
	for _, s in p.sections do
		table.insert(sections, { id = s.id, name = s.name, sessions = table.clone(s.sessions) })
	end
	return { pinned = table.clone(p.pinned), sections = sections }
end

-- The engine's places reach the sidebar; the sidebar's edits reach the
-- engine (the host sends only what differs from what it has, in order).
-- Made with the module, once for the window (the script owns them, not
-- whichever sidebar or rail came first).
--
-- An edit made here stands until the engine says it back: its places as
-- they were (a session's state moving republishes them), or half way
-- there (the host's changes land one by one), never undo it, or a dropped
-- row would jump back and forward again. What the engine says otherwise
-- wins once it has had time to answer (another device's edit).
local heard: sessions.Places? = nil
local sent: sessions.Places? = nil
local sent_at = 0
local ANSWER_MS = 5000
ui.effect(function()
	local theirs = plain(sessions.places:get())
	-- (Their places as they were: nothing to take.)
	if heard and same(theirs, heard) then
		return
	end
	heard = theirs
	if sent then
		if same(theirs, sent) then
			sent = nil
		elseif ui.now() - sent_at < ANSWER_MS then
			return
		else
			sent = nil
			ds.toast.show { title = locale.s("shell.sidebar.notice.unconfirmed"), tone = "danger" }
		end
	end
	if not same(theirs, plain(shared.places:peek())) then
		shared.places:set(theirs)
	end
end)
-- (Sent once the sessions are there, whether or not the engine has
-- heard its places yet: what goes is only what differs from what it
-- said, so nothing it hasn't said is ever taken out.)
-- (Nothing sent that the engine said already, or that went last: an echo
-- coming back, or the scripts reloading, sends nothing.)
local last_placed: sessions.Places? = nil
ui.effect(function()
	local mine = plain(shared.places:get())
	if sessions.now:peek().state ~= "ready" then
		return
	end
	if (heard and same(mine, heard)) or (last_placed and same(mine, last_placed)) then
		return
	end
	sent, sent_at, last_placed = mine, ui.now(), mine
	sessions.place(mine)
end)
local place_failures = 0
ui.effect(function()
	local failed = (sessions.now:get() :: any).place_failed
	if type(failed) == "table" and failed.n ~= place_failures then
		place_failures = failed.n
		ds.toast.show { title = locale.s("shell.sidebar.notice.not_saved", { why = failed.words }), tone = "danger" }
	end
end)
-- Your sections' folds are the engine's, on every device: what it says
-- folds them here, and folding one here tells it.
ui.effect(function()
	local sections = sessions.places:get().sections
	local mine = shared.folded:peek()
	local next: { [string]: boolean }? = nil
	for _, s in sections do
		local folded = s.collapsed == true
		if (mine[s.id] == true) ~= folded then
			next = next or table.clone(mine)
			;(next :: any)[s.id] = if folded then true else nil
		end
	end
	if next then
		shared.folded:set(next)
	end
end)
ui.effect(function()
	local mine = shared.folded:get()
	for _, s in sessions.now:peek().places.sections do
		local folded = mine[s.id] == true
		if folded ~= (s.collapsed == true) then
			sessions.collapse(s.id, folded)
		end
	end
end)
-- The session chosen is the route's: a choice moves the route, and the
-- route (back, forward, a new chat) moves the choice.
-- (Only a choice moves it: as the sidebar is made, after a reload say,
-- the session kept chosen is the route's already, or settings is open
-- over it, and nothing should move.)
local made = false
ui.effect(function()
	local id = shared.selected:get()
	if not made then
		made = true
		return
	end
	if id and id ~= route.chat:peek().session then
		route.open_session(id)
	end
end)
ui.effect(function()
	-- (The chat route's: settings over the window leaves it chosen.)
	local s = route.chat:get().session
	if shared.selected:peek() ~= s then
		shared.selected:set(s)
	end
end)

-- Device names, by id; this Mac's.
local function devices(): ({ [string]: string }, string?)
	local names: { [string]: string } = {}
	local here: string? = nil
	local here_id = sessions.here:get()
	for _, d in sessions.devices:get() do
		names[d.id] = d.name
		if d.here or d.id == here_id then
			here = d.name
		end
	end
	return names, here
end

-- A device's id by the name its group shows (ds.Sidebar groups by name):
-- the project's own device where it's named so, else the first of yours
-- named so, else one of Cloud's; nil for none (Unknown device).
local function device_named(name: string, project: string?): string?
	local now = sessions.now:peek()
	local own: string? = nil
	for _, p in now.projects do
		if p.id == project then
			own = p.device
		end
	end
	local first, cloud = nil :: string?, nil :: string?
	for _, d in now.devices do
		if d.name == name then
			if d.id == own then
				return d.id
			elseif d.cloud then
				cloud = cloud or d.id
			else
				first = first or d.id
			end
		end
	end
	return first or cloud
end

-- The sessions as the sidebar groups them: under each project (in the
-- engine's order, empty ones too: a new chat starts in one), those in none
-- under Chats.
--
-- A project is empty when every session it has had is archived: none
-- listed, one or more on the Archived shelf. The sidebar folds those into
-- one row at the projects' end. One with no sessions at all (just added)
-- isn't: it's where a new chat is about to start, listed with the rest, until
-- Show → Empty projects is off, which hides it too (it's still in the project
-- selector). Counted on all of them,
-- not what a filter shows, so narrowing never moves a project in or out
-- (the status filter hides every group with none anyway); the project the
-- project filter is narrowed to is never folded away. Archived sessions
-- stay on the Archived shelf, as everyone's do, so Show → Archived changes
-- nothing here.
local NO_PROJECT = "none"
-- What an agent asks of you, by chat (the chat page's, `chat.asks`): it
-- says which of "input" a session waiting on you is, an approval or a
-- question.
local asks = ui.shared("chat.asks", {} :: { [string]: string })
local ASKED: { [string]: string } = { approval = "approval", question = "input" }

-- Which sessions show, as a plugin allowed to change how the sidebar shows
-- asks (`sidebar.filter`, the plugin system's to grant): nil, all of them;
-- or `{ statuses = { "working", … } }`, those in these states ("idle", in
-- none). Written by a plugin, so read with care: what isn't that shape is
-- nil, and a word it doesn't know counts for nothing. Who's narrowing it is
-- the host's word (`sidebar.filter_by`: the name of the plugin allowed to,
-- src/plugins), never the plugin's own `by`, which could claim to be Zeron.
local KNOWN = { working = true, waiting = true, input = true, approval = true, failed = true, done = true, woke = true, idle = true }
local filter = ui.shared("sidebar.filter", nil :: any)
local filter_by = ui.shared("sidebar.filter_by", nil :: any)
type Narrow = { statuses: { [string]: boolean }, words: { string }, by: string }
local narrow = ui.computed(function(): Narrow?
	local f = filter:get()
	if type(f) ~= "table" or type(f.statuses) ~= "table" then
		return nil
	end
	local statuses: { [string]: boolean } = {}
	local words: { string } = {}
	for _, w in f.statuses do
		if type(w) == "string" and KNOWN[w] and not statuses[w] then
			statuses[w] = true
			table.insert(words, w)
		end
	end
	if #words == 0 then
		return nil
	end
	local named = filter_by:get()
	local by = if type(named) == "string" and named ~= "" then string.sub(named, 1, 40) else locale.s("shell.sidebar.filter.a_plugin")
	return { statuses = statuses, words = words, by = by }
end)
-- How the notice names each state (shell.sidebar.filter.state.<word>).
local function say(word: string): string
	return locale.s(`shell.sidebar.filter.state.{word}`)
end

-- The Archived shelf's sessions (the most recently moved first), as the
-- list's rows read them.
local shelf = ui.computed(function(): { any }
	local names = devices()
	local out = {}
	for _, c in sessions.archived:get() do
		table.insert(out, {
			id = c.id,
			title = c.title,
			time = c.updated,
			created = c.created,
			agent = c.agent,
			harness = c.harness,
			agent_icon = c.agent_icon,
			device = names[c.device] or locale.s("shell.sidebar.unknown_device"),
			branch = c.branch,
		})
	end
	return out
end)

-- The project the list is narrowed to (the filter showing, and the
-- project still there), else nil.
local function only_project(): string?
	if not shared.details.project_filter:get() then
		return nil
	end
	local id = shared.project:get()
	if not id then
		return nil
	end
	for _, p in sessions.projects:get() do
		if p.id == id then
			return id
		end
	end
	return nil
end
-- (The filter put away: it shows everything again, as Zeron's clears it.)
ui.effect(function()
	if not shared.details.project_filter:get() and shared.project:peek() ~= nil then
		shared.project:set(nil)
	end
end)

-- Each session's row as last made: kept while it says the same, so the
-- row (ds/sidebar follows its session's own table) stays asleep as others
-- change.
local made_rows: { [string]: { [string]: any } } = {}
local function kept(row: { [string]: any }): { [string]: any }
	local was = made_rows[row.id]
	if was then
		local same = true
		for k, v in row do
			if was[k] ~= v then
				same = false
				break
			end
		end
		if same then
			for k in was do
				if row[k] == nil then
					same = false
					break
				end
			end
		end
		if same then
			return was
		end
	end
	made_rows[row.id] = row
	return row
end

-- `p` and the other checkouts of its repository, on any device, the oldest first
-- (its name and picture stand for them all, the same on every device). A project
-- with no repository is its own.
local function checkouts_of(p: any, all: { any }): { any }
	local out: { any } = {}
	for _, q in all do
		if q.id == p.id or (p.repository and q.repository == p.repository) then
			table.insert(out, q)
		end
	end
	table.sort(out, function(a: any, b: any): boolean
		local x, y = a.created or math.huge, b.created or math.huge
		if x ~= y then
			return x < y
		end
		return a.id < b.id
	end)
	return out
end
local function oldest_checkout(p: any, all: { any }): any
	return checkouts_of(p, all)[1]
end
-- Whether `project` falls under the filter on `just`: a repository's checkouts are one.
local function in_filter(project: string?, just: string): boolean
	local all = sessions.now:peek().projects
	local mine, theirs = nil :: any, nil :: any
	for _, p in all do
		if p.id == project then
			mine = oldest_checkout(p, all)
		end
		if p.id == just then
			theirs = oldest_checkout(p, all)
		end
	end
	return mine ~= nil and theirs ~= nil and mine.id == theirs.id
end
-- Where a new chat in project `id`'s repository starts: its clone on this device,
-- else `id`.
local function checkout_here(id: string?): string?
	local now = sessions.now:peek()
	for _, p in now.projects do
		if p.id == id then
			for _, q in checkouts_of(p, now.projects) do
				if q.device == now.here then
					return q.id
				end
			end
		end
	end
	return id
end

local composers = ui.shared("chat.composer.drafts", {} :: { [string]: any })
local unsent_ids = ui.computed(function(): { [string]: boolean }
	local out: { [string]: boolean } = {}
	for id, d in composers:get() do
		if id ~= "" and type(d) == "table" and ((type(d.text) == "string" and d.text ~= "") or (type(d.cargo) == "table" and #d.cargo > 0)) then
			out[id] = true
		end
	end
	return out
end)

local function settled_at(c: any, status: string?, days: number, at: number): number?
	if status == "working" or status == "input" or status == "approval" then
		return nil
	end
	local activity = c.last_message
	local settle = c.settle
	if settle and (activity == nil or activity <= settle.at) then
		return if settle.settled then settle.at else nil
	end
	return if activity and activity < at - days * 86400 then activity else nil
end
local inbox_marks: { [string]: { working: boolean, since: number? } } = {}
local function inbox_place(c: any, status: string?, days: number, working_shelf: boolean, at: number): (string, number)
	local working = status == "working"
	local mark = inbox_marks[c.id]
	if not mark then
		mark = { working = working, since = nil }
		inbox_marks[c.id] = mark
	elseif mark.working ~= working then
		mark.working = working
		mark.since = at
	end
	local settled = settled_at(c, status, days, at)
	if settled then
		return "settled", settled
	end
	local anchor = math.max(c.created or 0, c.unsettled or 0)
	if not working_shelf then
		return "inbox", anchor
	end
	local base = math.max(c.last_message or anchor, anchor)
	if working then
		return "working", c.started or mark.since or base
	end
	return "inbox", math.max(mark.since or base, base)
end
local function work_words(tasks: any): string?
	if tasks == nil then
		return nil
	end
	local parts: { string } = {}
	if tasks.agents > 0 then
		table.insert(parts, locale.s("ds.sidebar.work.subagents", { n = tasks.agents }))
	end
	if tasks.shells > 0 then
		table.insert(parts, locale.s("ds.sidebar.work.shells", { n = tasks.shells }))
	end
	if tasks.others > 0 then
		table.insert(parts, locale.s("ds.sidebar.work.tasks", { n = tasks.others }))
	end
	return table.concat(parts, " · ")
end

-- peeked, not tracked: read from a shortcut or a click, never a computed
route.narrowed_project(function(): string?
	if not shared.details.project_filter:peek() then
		return nil
	end
	local id = shared.project:peek()
	for _, p in sessions.now:peek().projects do
		if p.id == id then
			return checkout_here(id)
		end
	end
	return nil
end)

local projects = ui.computed(function(): { ds.SidebarProject }
	local now = sessions.now:get()
	local asked = asks:get()
	local only = narrow:get()
	local just = only_project()
	local names = devices()
	local unsent = unsent_ids:get()
	local in_inbox = shared.inbox:get()
	local working_shelf = shared.details.working:get()
	local days = if (now :: any).settle_after_days and (now :: any).settle_after_days > 0 then (now :: any).settle_after_days else 3
	local at = os.time()
	local paths: { [string]: any } = {}
	for _, p in now.projects do
		paths[p.id] = p
	end
	local out: { ds.SidebarProject } = {}
	local by: { [string]: ds.SidebarProject } = {}
	-- One group for each repository, whichever device its clones are on: it stands
	-- where the first of them does, and reads as the oldest of them.
	local of_repository: { [string]: ds.SidebarProject } = {}
	for _, p in now.projects do
		local group = if p.repository then of_repository[p.repository] else nil
		if not group then
			local first = oldest_checkout(p, now.projects)
			local l = route.look(first.id) or { hue = route.hue(first.id) }
			group = { id = first.id, name = first.name, hue = l.hue, src = l.src, emoji = l.emoji, mono = (first :: any).icon == nil and l.src == nil and l.emoji == nil, seed = first.path, sessions = {} }
			table.insert(out, group :: any)
			if p.repository then
				of_repository[p.repository] = group :: any
			end
		end
		by[p.id] = group :: any
	end
	local loose: ds.SidebarProject = { id = NO_PROJECT, name = locale.s("shell.sidebar.chats"), icon = "side-chat", mono = true, mono_name = "Home", seed = "home", sessions = {} }
	-- (Which have sessions listed, and which archived ones.)
	-- (By group: a repository isn't empty while any clone of it has a session.)
	local just_group = if just then by[just] else nil
	local listed: { [any]: boolean } = {}
	for _, c in now.chats do
		local group = if c.project then by[c.project] else nil
		if group then
			listed[group] = true
		end
	end
	local archived: { [any]: boolean } = {}
	for _, c in now.archived or {} do
		local group = if c.project then by[c.project] else nil
		if group then
			archived[group] = true
			if not listed[group] and group ~= just_group then
				group.empty = true
			end
		end
	end
	-- (And which have never had one: gone with the empty ones when those are off.)
	for _, group in out do
		if not listed[group] and not archived[group] and group ~= just_group then
			group.unused = true
		end
	end
	for _, c in now.chats do
		if just and not (just_group and c.project and by[c.project] == just_group) then
			continue
		end
		local group = (c.project and by[c.project]) or loose
		-- (Waiting on you beats working: an approval, then a question.)
		local status = ASKED[asked[c.id] or ""] or c.status
		-- (The Activity view's: woken from sleep and not looked at since, the
		-- alarm, unless it's asking for you or under way. Which shelf it's
		-- on, and its place; asleep, its corner says when it wakes. Off, none
		-- of it.)
		local act = activity.on:get()
		if act and c.woke and (status == nil or status == "done") then
			status = "woke"
		end
		if only and not only.statuses[status or "idle"] then
			continue
		end
		local room = c :: any
		local on_shelf = activity.shelf(c, status)
		local shelf: string?, rank: number? = on_shelf, nil
		if act then
			rank = activity.rank(c, on_shelf)
		elseif in_inbox then
			shelf, rank = inbox_place(room, status, days, working_shelf, at)
		end
		local tasks = room.tasks
		local project = if c.project then paths[c.project] else nil
		table.insert(group.sessions, kept({
			id = c.id,
			title = c.title,
			status = status :: any,
			time = if on_shelf == "asleep" and c.sleep_until then activity.wakes_in(c.sleep_until) else c.updated,
			created = c.created,
			agent = c.agent :: any,
			harness = room.harness,
			agent_icon = room.agent_icon,
			device = names[c.device] or locale.s("shell.sidebar.unknown_device"),
			branch = c.branch,
			base = if room.tree then room.source_branch else nil,
			-- (Its side chats working count as its subagents do, as its
			-- lanes' badge counts them: screens/chat/lanes.)
			subagents = (c.running_subagents or 0) + (c.working_side_chats or 0),
			fan = Pills.foreign(Pills.fan_of(c.id, room.fan), room),
			started = room.started,
			ring = if tasks and (tasks.held and status == "working" or status == nil or status == "done") then tasks.agents + tasks.shells + tasks.others else nil,
			work_label = if tasks and tasks.held and status == "working" then work_words(tasks) else nil,
			marks = if room.marks and #room.marks > 0 then table.concat(room.marks, ",") else nil,
			tree_kind = if room.tree then room.tree.kind else nil,
			tree_commits = if room.tree then room.tree.commits else nil,
			tree_dirty = if room.tree then room.tree.dirty else nil,
			worktree = project ~= nil and project.git == true and room.cwd ~= nil and room.cwd ~= project.path,
			remote = now.here ~= nil and c.device ~= now.here,
			created_by = room.created_by,
			unsent = unsent[c.id] == true,
			shelf = shelf :: any,
			rank = rank,
		}) :: any)
	end
	if #loose.sessions > 0 then
		table.insert(out, loose)
	end
	-- Narrowed: the groups left with none fold away.
	if only then
		local kept: { ds.SidebarProject } = {}
		for _, g in out do
			if #g.sessions > 0 then
				table.insert(kept, g)
			end
		end
		return kept
	end
	return out
end)

-- The project filter (Zeron's, its view options' "Project filter"): all
-- projects, or one, searched for, the list narrowed to it.
local ALL = "::all"
local function add_a_project()
	local add: any = boundary.load("@app/screens/shell/add_project")
	if add then
		add.open()
	end
end
local function offline(p: any): boolean
	local now = sessions.now:get()
	for _, d in now.devices do
		if d.id == p.device then
			return not d.online
		end
	end
	return true
end
local project_menu_of: (id: string) -> { any }? = function()
	return nil
end
local function project_filter(): Node
	local chosen = ui.computed(function(): string
		return only_project() or ALL
	end)
	local value = {
		get = function(_): string
			return chosen:get()
		end,
		peek = function(_): string
			return chosen:peek()
		end,
		set = function(_, v: string)
			shared.project:set(if v == ALL then nil else v)
		end,
	}
	return ui.show(
		function(): boolean
			return shared.details.project_filter:get()
		end,
		ui.div {
			id = "sidebar-project-filter",
			shrink = 0,
			ds.Dropdown {
				value = value :: any,
				icon = "folder",
				width = 280,
				wrap_item = function(item: any, row: Node): Node
					if item.value == ALL then
						return row
					end
					return ContextMenu {
						label = locale.t("ds.sidebar.session_actions"),
						w_full = true,
						items = function()
							return project_menu_of(item.value) or {}
						end,
						row,
					}
				end,
				trigger = function(m: any): Node
					local function says(): string
						local id = only_project()
						for _, p in sessions.now:peek().projects do
							if p.id == id then
								return locale.s("ds.sidebar.filter_is", { name = p.name })
							end
						end
						return locale.s("ds.sidebar.filter_by")
					end
					return ui.presence(says, function(tip: string): Node
						return ds.IconButton {
							icon = "lucide-list-filter",
							label = tip,
							tooltip = tip,
							tone = "quiet",
							lit = function(): boolean
								return m.open:get() or only_project() ~= nil
							end,
							on_click = m.toggle,
						}
					end)
				end,
				search = function(): string
					return locale.s("shell.sidebar.project_filter.search")
				end,
				-- (Zeron's, pinned under the projects.)
				actions = {
					{
						label = ui.t("shell.sidebar.project_filter.new_project") :: any,
						icon = "plus",
						on_click = add_a_project,
					},
				},
				items = function(): { any }
					local out: { any } = { { value = ALL, label = locale.s("shell.sidebar.project_filter.all_projects"), icon = "folder-with-files" } }
					-- (A repository once, as the list has it.)
					local all = sessions.projects:get()
					local seen: { [string]: boolean } = {}
					for _, p in all do
						local first = oldest_checkout(p, all)
						if not seen[first.id] then
							seen[first.id] = true
							local l = route.look(first.id) or { hue = route.hue(first.id) }
							table.insert(out, {
								value = first.id,
								label = first.name,
								hue = l.hue,
								src = l.src,
								emoji = l.emoji,
								warn = offline(first),
								project = first.id,
							})
						end
					end
					return out
				end,
			},
		}
	)
end

-- What the filter shows, said once at the list's top, and the way back to
-- all of it: "Focus is showing working sessions only · Show all".
local function filter_notice(): Node
	local function says(): string
		local only = narrow:get()
		if not only then
			return ""
		end
		local names: { string } = {}
		for _, w in only.words do
			table.insert(names, say(w))
		end
		local list = if #names == 1
			then names[1]
			else locale.s("shell.sidebar.filter.list_last", {
				rest = table.concat(names, locale.s("shell.sidebar.filter.list_sep"), 1, #names - 1),
				last = names[#names],
			})
		return locale.s("shell.sidebar.filter.notice", { by = only.by, list = list })
	end
	return ui.show(
		function(): boolean
			return narrow:get() ~= nil
		end,
		ui.row {
			id = "sidebar-filter",
			w_full = true,
			shrink = 0,
			px = "$space.2",
			gap = "$space.2",
			items = "start",
			-- (Said whole, over two lines if it needs them: it's why the
			-- list is short.)
			ui.text { says, grow = 1, min_w = 0, wrap = true, text_size = "$font.size.xs", color = "$text.tertiary" },
			ds.Link {
				ui.t("shell.sidebar.filter.show_all") :: any,
				tone = "quiet",
				size = "sm",
				on_click = function()
					filter:set(nil)
				end,
			},
		}
	)
end

-- What the connection line says (production's connection pill: no
-- surface, a faint caption; the amber dot when offline or a change wasn't
-- saved, the working ring while reconnecting). Healthy: nothing.
local CONNECTION: { [string]: string } = {
	unsaved = "shell.sidebar.connection.unsaved",
	offline = "shell.sidebar.connection.offline",
	reconnecting = "shell.sidebar.connection.reconnecting",
}
local function connection(): string?
	local c: any = sessions.connection:get()
	return if type(c) == "string" and CONNECTION[c] then c else nil
end
local function connection_line(): Node
	return ui.show(
		function(): boolean
			return connection() ~= nil
		end,
		ui.row {
			id = "connection-line",
			w_full = true,
			shrink = 0,
			-- (Its dot on the foot's left edge: Star on GitHub's glyph's, the
			-- filter's and the empty list's words'. Its words on Star on
			-- GitHub's: the dot or the ring in a slot as wide as the star,
			-- as far from them.)
			px = "$space.2",
			pb = "$space.2",
			gap = "$zeron.space.tight",
			items = "center",
			ui.row {
				shrink = 0,
				w = 12,
				items = "center",
				ui.presence(function(): string
					return if connection() == "reconnecting" then "ring" else "dot"
				end, function(glyph: string): Node
					if glyph == "ring" then
						return ui.div { shrink = 0, status.StatusIcon { kind = "connecting", size = 10 } }
					end
					return ui.div { shrink = 0, w = 5, h = 5, rounded = "$radius.full", bg = "$warning" }
				end),
			},
			ds.FadeText {
				function(): string
					local key = CONNECTION[connection() or ""]
					return if key then locale.s(key) else ""
				end,
				grow = 1,
				text_size = "$font.size.xs",
				color = "$text.tertiary",
			},
		}
	)
end

-- An engine too old for this interface: why the app can't work, and the way
-- back to Stable. The rail's Update entry has the rest.
local function foot_notes(): Node
	return ui.column {
		w_full = true,
		shrink = 0,
		require("@app/screens/shell/whats_new").Card(),
		connection_line(),
		boundary.screen(boundary.named("shell.boundary.screen.update_notice"), "@app/screens/engine/update", function(update): Node
			return update.Notice()
		end, { pass = true }),
	}
end

-- Renaming a session: which, and the name being typed.
local renaming = ui.state("shell_renaming", nil :: string?)
local rename_open = ui.state("shell_rename_open", false)
-- The session renamed in its row (Zeron's inline rename), while it is.
local renaming_row = ui.state("shell_renaming_row", nil :: string?)
-- The session asked about deleting, and its dialog.
local deleting = ui.state("shell_deleting", nil :: string?)
local delete_open = ui.state("shell_delete_open", false)
-- What the delete takes with it (its subagents and side chats, theirs in
-- turn), as the host counted them when it was asked.
local delete_subagents = ui.state("shell_delete_subagents", 0)
local delete_sides = ui.state("shell_delete_sides", 0)
-- Sleep → Custom…: the chat, the moment chosen, and whether it's asking.
local sleep_custom = ui.state("shell_sleep_custom", nil :: string?)
local sleep_custom_at = ui.state("shell_sleep_custom_at", nil :: number?)
local sleep_custom_open = ui.state("shell_sleep_custom_open", false)
local new_title = ui.state("shell_rename_title", "")

local function chat(id: string): sessions.Chat?
	local now = sessions.now:peek()
	for _, c in now.chats do
		if c.id == id then
			return c
		end
	end
	-- A side chat (the panel's lists and tabs take this menu too).
	for _, list in (now :: any).children or {} do
		for _, c in list do
			if c.id == id then
				return c :: any
			end
		end
	end
	return nil
end

-- Whether `id` is a side chat (under another): the sidebar lists none, so
-- pinning it or putting it in a section doesn't apply.
local function is_side(id: string): boolean
	for _, c in sessions.now:peek().chats do
		if c.id == id then
			return false
		end
	end
	return chat(id) ~= nil
end

-- Where a session is put by hand: pinned, a section, or back in its group.
local function put(id: string, where: string?)
	local p = plain(shared.places:peek())
	local pinned = {}
	for _, other in p.pinned do
		if other ~= id then
			table.insert(pinned, other)
		end
	end
	if where == "pinned" then
		table.insert(pinned, id)
	end
	for _, s in p.sections do
		local i = table.find(s.sessions, id)
		if i then
			table.remove(s.sessions, i)
		end
		if s.id == where then
			table.insert(s.sessions, id)
		end
	end
	shared.places:set({ pinned = pinned, sections = p.sections })
end

-- What else asks for these (the shell's keys, the palette).
local M = {}

-- The view options, kept across launches (how it's grouped, sorted, …).
M.options = shared
-- The sidebar's view, whole, for other lists of sessions to draw as it
-- draws them (the panel home's side chats and subagents, screens/panel/
-- home, through ds/session_row): `{ compact, organize, sort, details }`,
-- `details` every one of its Show options by name (`ds.SidebarDetails`'s,
-- as ds/sidebar reads them). Made from the options themselves, so one the
-- sidebar gains reaches them with no change here.
local sidebar_view = ui.shared("sidebar.view", {} :: { [string]: any })
-- (Organize kept as one choice before, "project": its levels from now.)
ui.effect(function()
	local v = shared.organize:get()
	if type(v) ~= "table" then
		shared.organize:set(ds_session_row.levels(v))
	end
end)
-- (`same`: the places' field-by-field comparison, above.)
ui.effect(function()
	local details: { [string]: any } = {}
	for name, st in shared.details :: any do
		details[name] = (st :: any):get()
	end
	local v = {
		compact = shared.compact:get(),
		organize = ds_session_row.levels(shared.organize:get()),
		sort = shared.sort:get(),
		details = details,
	}
	if not same(v, sidebar_view:peek()) then
		sidebar_view:set(v)
	end
end)
-- The sessions the list shows, in its order (those folded away, and those
-- past a group's "Show N more", left out): ⌘1–9 go to its first nine.
M.order = ui.state("shell_sidebar_order", {} :: { string })
-- ⌘ held alone: those nine wear their keys (Zeron's jump hints): shown at
-- once, as Zeron's are, kept through the ⌘1–9 they offer, and only while
-- this window is the one typed in and no palette has the keys. Any other
-- key pressed meanwhile is a shortcut (⌘C), not a look at them: they go
-- until the modifiers held change again (`M.key_pressed`, from the shell's
-- root and its commands).
-- ⌘ is the key `cmd+1` is pressed with: Command on macOS, Ctrl elsewhere
-- (zgx's `cmd`), where Super (`meta`) is just another modifier.
local held = ui.modifiers
local mac = string.find(ui.shortcut_text("cmd"), "⌘", 1, true) ~= nil
local primary = if mac then held.meta else held.control
local other = if mac then held.control else held.meta
local pressed = ui.state("shell_jump_hints_pressed", false)
ui.effect(function()
	-- (Every change of what's held starts afresh, as the shipping app's.)
	local _ = { held.meta:get(), held.shift:get(), held.control:get(), held.alt:get() }
	if pressed:peek() then
		pressed:set(false)
	end
end)
-- A key went down (the shell root's `on_key`; a shell command run, its menu
-- key equivalent never reaching `on_key`): the hints go, unless it's the
-- ⌘1–9 they offer (`digit`).
function M.key_pressed(digit: boolean?)
	if primary:peek() and not digit and not pressed:peek() then
		pressed:set(true)
	end
end
M.jump_hints = ui.state("shell_jump_hints", false)
ui.effect(function()
	local alone = primary:get() and not (held.shift:get() or other:get() or held.alt:get())
	local show = alone
		and not pressed:get()
		and ui.window.active:get() ~= false
		and not view.palette:get()
	if not show then
		if M.jump_hints:peek() then
			M.jump_hints:set(false)
		end
		return
	end
	if not M.jump_hints:peek() then
		M.jump_hints:set(true)
	end
end)
-- The session card showing (a row rested on: its id and the row's box in
-- the window), drawn over the window by the shell; nil, none.
export type Card = { id: string, x: number, y: number, w: number, h: number }
M.card = ui.state("shell_session_card", nil :: Card?)
-- (The window's fit narrows it from what it would be: this.)
fit.sidebar_width(function(): number
	return shared.width:get()
end)

function M.archive(id: string)
	if route.current:peek().session == id then
		route.new_chat()
	end
	sessions.archive(id)
end

-- A session's menu, as the gallery's (Zeron's sidebar chat menu): rename,
-- pin, copy, move into one of your sections, archive (kept: Settings →
-- Archived lists it), delete (for good, asked once more); then plugins'
-- items, under their names (screens/plugins/menus: the app's registry of
-- them, read as the menu opens).
-- (Copied, said so, as Zeron's: "Path copied".)
local function copy(text: string?, what: string)
	if text then
		ui.clipboard.write(text)
		ds.toast.show { title = locale.s("shell.sidebar.copied", { what = what }), tone = "success" }
	end
end
-- A string as a URL's part (as production's `encode_component`).
local function encoded(v: string): string
	return (string.gsub(v, "[^%w%-_%.~]", function(c: string): string
		return string.format("%%%02X", string.byte(c))
	end))
end

local function can_place(what: string): boolean
	local now = sessions.now:peek()
	if now.state ~= "ready" then
		ds.toast.show { title = locale.s("shell.sidebar.notice.not_connected"), tone = "danger" }
		return false
	end
	if auth.signed_in:peek() and not now.places_ready then
		local key = if what == "pinned" then "shell.sidebar.notice.pins_syncing" else "shell.sidebar.notice.syncing"
		ds.toast.show { title = locale.s(key), tone = "danger" }
		return false
	end
	if what == "pinned" and #shared.places:peek().pinned >= 200 then
		ds.toast.show { title = locale.s("shell.sidebar.notice.pin_cap"), tone = "danger" }
		return false
	end
	return true
end
M.pin = function(id: string)
	local pinned = table.find(shared.places:peek().pinned, id) ~= nil
	if not can_place(if pinned then "section" else "pinned") then
		return
	end
	put(id, if pinned then nil else "pinned")
end
local function is_settled(c: any): boolean
	local now = sessions.now:peek() :: any
	local days = if now.settle_after_days and now.settle_after_days > 0 then now.settle_after_days else 3
	return settled_at(c, c.status, days, os.time()) ~= nil
end
M.is_settled = is_settled
local warmed: string? = nil
ui.effect(function()
	local c = route.chat:get() :: any
	local id = if c.name == "session" then c.session else nil
	if id ~= warmed then
		warmed = id
		if id then
			-- (Not in the load's own turn: a reload brings the scripts back first, the host's or a fixture's actions after.)
			ui.after(0, function()
				sessions.warm(id)
			end)
		end
	end
end)
function M.settle(id: string, settled: boolean)
	sessions.settle(id, settled):after(function() end, function()
		ds.toast.show { title = locale.s("shell.sidebar.notice.syncing"), tone = "danger" }
	end)
end

local function menu(id: string): { any }?
	local c = chat(id)
	if not c then
		return nil
	end
	local p = shared.places:peek()
	local pinned = table.find(p.pinned, id) ~= nil
	local into: { any } = { { heading = locale.s("shell.sidebar.menu.sections") } }
	for _, s in p.sections do
		local here = table.find(s.sessions, id) ~= nil
		table.insert(into, {
			label = s.name,
			icon = "folder",
			checked = function(): boolean
				return here
			end,
			-- (Chosen where it is: back to its project.)
			on_select = function()
				if can_place("section") then
					put(id, if here then nil else s.id)
				end
			end,
		})
	end
	table.insert(into, { separator = true })
	table.insert(into, {
		label = locale.s("shell.sidebar.menu.new_section"),
		icon = "plus",
		on_select = function()
			if not can_place("section") then
				return
			end
			local sid = `section-{os.time()}-{math.random(1, 1e6)}`
			local now = plain(shared.places:peek())
			table.insert(now.sections, { id = sid, name = locale.s("shell.sidebar.menu.new_section"), sessions = {} })
			shared.places:set(now)
			put(id, sid)
		end,
	})
	-- What Copy offers: its folder, its harness's link and id where it has
	-- them (a Codex thread's), and its own id.
	local copies: { any } = {
		{
			label = locale.s("shell.sidebar.menu.path"),
			icon = "folder",
			disabled = if c.cwd then nil else locale.s("shell.sidebar.menu.no_folder"),
			on_select = function()
				copy(c.cwd, locale.s("shell.sidebar.menu.path"))
			end,
		},
	}
	table.insert(copies, {
		label = locale.s("shell.sidebar.menu.zeron_link"),
		icon = "arrow-up-right",
		on_select = function()
			sessions.link(id):after(function(link: any)
				copy(if type(link) == "string" then link else nil, locale.s("shell.sidebar.menu.zeron_link"))
			end, function()
				ds.toast.show { title = locale.s("shell.sidebar.menu.link_not_ready"), tone = "danger" }
			end)
		end,
	})
	if c.harness == "codex" and c.harness_session then
		table.insert(copies, {
			label = locale.s("shell.sidebar.menu.codex_link"),
			icon = "arrow-up-right",
			on_select = function()
				copy(`codex://threads/{encoded(c.harness_session :: string)}`, locale.s("shell.sidebar.menu.codex_link"))
			end,
		})
	end
	if c.harness_session then
		table.insert(copies, {
			label = locale.s("shell.sidebar.menu.harness_session_id"),
			icon = "document",
			on_select = function()
				copy(c.harness_session, locale.s("shell.sidebar.menu.harness_session_id"))
			end,
		})
	end
	table.insert(copies, {
		label = locale.s("shell.sidebar.menu.session_id"),
		icon = "document",
		on_select = function()
			copy(c.id, locale.s("shell.sidebar.menu.session_id"))
		end,
	})
	local side = is_side(id)
	-- The Activity view's Sleep (its times, each saying when, and one of
	-- your own), or, asleep, Wake.
	local sleep: any = false
	if activity.on:peek() and not side then
		if c.sleep_until then
			sleep = {
				label = locale.s("shell.activity.wake"),
				icon = "sun",
				on_select = function()
					activity.wake(id)
				end,
			}
		else
			local times: { any } = {}
			for _, p in activity.presets() do
				table.insert(times, {
					label = locale.s("shell.activity.sleep.choice", { name = p.label, when = p.detail }),
					on_select = function()
						activity.sleep(id, p.until_at)
					end,
				})
			end
			table.insert(times, { separator = true })
			table.insert(times, {
				label = locale.s("shell.activity.sleep.custom"),
				on_select = function()
					-- (It opens on tomorrow at 9: a time already chosen,
					-- moved from there.)
					local d = os.date("*t") :: any
					sleep_custom:set(id)
					sleep_custom_at:set(activity.local_time({ year = d.year, month = d.month, day = d.day + 1, hour = 9, min = 0, sec = 0 }))
					sleep_custom_open:set(true)
				end,
			})
			sleep = { label = locale.s("shell.activity.sleep.title"), icon = "moon", submenu = times }
		end
	end
	-- Waiting on background work it said it waits on: the person can say
	-- otherwise (a dev server marked by mistake). Its background agents
	-- aren't its say, so there's nothing to take back there.
	local stop_waiting: any = false
	if c.status == "waiting" then
		for _, w in c.waiting or {} do
			if w.kind == "process" then
				stop_waiting = {
					label = locale.s("shell.activity.stop_waiting"),
					icon = "close-circle",
					on_select = function()
						sessions.stop_waiting(id):after(function() end, function()
							ds.toast.show { title = locale.s("shell.activity.stop_waiting_failed"), tone = "danger" }
						end)
					end,
				}
				break
			end
		end
	end
	local entries = {
		{
			label = locale.s("common.rename"),
			icon = "pen",
			on_select = function()
				M.rename(id)
			end,
		},
		if side
			then false
			else {
				label = locale.s(if pinned then "shell.sidebar.menu.unpin" else "shell.sidebar.menu.pin"),
				icon = "pin",
				shortcut = "cmd+shift+p",
				on_select = function()
					M.pin(id)
				end,
			},
		{
			-- (No key: ⌘⌫ is a text field's, deleting to the line's start.)
			label = locale.s("common.archive"),
			icon = "archive-minimalistic",
			on_select = function()
				M.archive(id)
			end,
		},
		if side or not shared.inbox:peek()
			then false
			else {
				label = locale.s(if is_settled(c) then "shell.sidebar.menu.unsettle" else "shell.sidebar.menu.settle"),
				icon = if is_settled(c) then "lucide-undo-2" else "lucide-circle-check",
				on_select = function()
					M.settle(id, not is_settled(c))
				end,
			},
		{
			label = locale.s("common.copy"),
			icon = "copy",
			submenu = copies,
		},
		-- (Activity lists no sections: nothing to move into.)
		if side or activity.on:peek() then false else { label = locale.s("shell.sidebar.menu.move_to"), icon = "folder-with-files", submenu = into },
		sleep,
		stop_waiting,
		{ separator = true },
		{
			label = locale.s("shell.sidebar.menu.delete"),
			icon = "trash-bin-minimalistic",
			danger = true,
			-- (Asked once more, as Zeron's: "Delete session?")
			-- (Once it knows what goes with it, to say so.)
			on_select = function()
				local function ask(subagents: number, sides: number)
					delete_subagents:set(subagents)
					delete_sides:set(sides)
					deleting:set(id)
					delete_open:set(true)
				end
				sessions.deleted_with(id):after(function(with: any)
					ask(tonumber(with and with.subagents) or 0, tonumber(with and with.sides) or 0)
				end, function()
					ask(0, 0)
				end)
			end,
		},
	}
	-- (A side chat's: without what doesn't apply to it.)
	local kept: { any } = {}
	for _, e in entries :: { any } do
		if e ~= false then
			table.insert(kept, e)
		end
	end
	entries = kept :: any
	-- (The registry is the plugin system's: without it, the app's alone.)
	local menus: { extend: ((entries: { any }, kind: string, context: { [string]: any }) -> { any })? }? =
		boundary.load("@app/screens/plugins/menus")
	local extend = menus and menus.extend
	if extend then
		return extend(entries, "session", { id = id })
	end
	return entries
end

-- The same menu, for a session shown elsewhere (the panel's side chat rows
-- and tabs): a side chat's without Pin and Move to.
M.session_menu = menu

-- The rename dialog, one for the window.
-- A project edited (its name and its icon: screens/shell/project_editor),
-- or removed with its sessions ("Remove project?"), from its header's
-- right click.
local project_acting = ui.state("shell_project_acting", nil :: string?)
local project_remove_open = ui.state("shell_project_remove_open", false)
-- Of a repository on several devices, the checkouts to remove, by id.
local project_remove_chosen = ui.state("shell_project_remove_chosen", {} :: { [string]: boolean })
local function project_of(id: string?): sessions.Project?
	for _, p in sessions.projects:get() do
		if p.id == id then
			return p
		end
	end
	return nil
end
local function project_menu(id: string): { any }?
	local p = project_of(id)
	if not p then
		return nil
	end
	return {
		{
			label = locale.s("shell.sidebar.menu.edit_project"),
			icon = "pen",
			on_select = function()
				M.edit_project(id)
			end,
		},
		{
			label = locale.s("shell.sidebar.menu.remove"),
			icon = "trash-bin-minimalistic",
			danger = true,
			on_select = function()
				-- (Of a repository on several devices: this device's, to begin with.)
				local now = sessions.now:peek()
				local chosen: { [string]: boolean } = {}
				for _, q in checkouts_of(p, now.projects) do
					if q.device == now.here then
						chosen[q.id] = true
					end
				end
				project_remove_chosen:set(chosen)
				project_acting:set(id)
				project_remove_open:set(true)
			end,
		},
	}
end
project_menu_of = project_menu
local function project_dialogs(): Node
	-- The checkouts of the project being removed (one, for most).
	local function acting_checkouts(): { any }
		local p = project_of(project_acting:get())
		return if p then checkouts_of(p, sessions.projects:get()) else {}
	end
	local function device_of(q: any): string
		for _, d in sessions.now:peek().devices do
			if d.id == q.device then
				return d.name
			end
		end
		return locale.s("shell.sidebar.project.its_device")
	end
	local function chats_in(id: string): number
		local n = 0
		for _, c in sessions.now:peek().chats do
			if c.project == id then
				n += 1
			end
		end
		return n
	end
	-- (Its sessions as Zeron counts them: those in it, not archived.)
	local function removing(): string
		local id = project_acting:get()
		local p = project_of(id)
		local name = if p then p.name else locale.s("shell.sidebar.project.this_project")
		local n = 0
		for _, c in sessions.now:get().chats do
			if c.project == id then
				n += 1
			end
		end
		local device = locale.s("shell.sidebar.project.its_device")
		for _, d in sessions.devices:get() do
			if p and d.id == p.device then
				device = d.name
			end
		end
		return locale.s("shell.sidebar.project.removing", { name = name, n = n, device = device })
	end
	return ui.column {
		-- Its name and its icon (Edit project…), once first opened.
		ui.presence(function(): string?
			return if project_editor_used:get() then "editor" else nil
		end, function(): Node
			return project_editor().Dialog()
		end),
		ds.Dialog {
			open = project_remove_open,
			title = ui.t("shell.sidebar.project.remove_title") :: any,
			tone = "danger",
			size = "sm",
			confirm = {
				label = ui.t("common.remove") :: any,
				on_click = function()
					local id = project_acting:peek()
					project_acting:set(nil)
					if not id then
						return
					end
					-- The one project, or the checkouts ticked.
					local going: { string } = { id }
					local list = acting_checkouts()
					if #list > 1 then
						going = {}
						for _, q in list do
							if project_remove_chosen:peek()[q.id] then
								table.insert(going, q.id)
							end
						end
					end
					for _, gone in going do
						-- (Showing it, or one of its sessions: a new chat instead.)
						local shown = route.session:peek()
						if route.chat:peek().project == gone or (shown and shown.project == gone) then
							route.new_chat()
						end
						project_calls.delete(gone):after(function() end, function(err: any)
							ds.toast.show { title = locale.s("shell.sidebar.project.couldnt_remove"), body = if type(err) == "string" then err else nil, tone = "danger" }
						end)
					end
				end,
			},
			ui.presence(function(): string
				return if #acting_checkouts() > 1 then "several" else "one"
			end, function(kind: string): Node
				if kind == "one" then
					return ui.text { removing, w_full = true, wrap = true, text_size = "$font.size.md", line_height = 20, color = "$text.secondary" }
				end
				return ui.column {
					w_full = true,
					gap = "$space.3",
					ui.text {
						function(): string
							local p = project_of(project_acting:get())
							return locale.s("shell.sidebar.project.removing_where", { name = if p then p.name else "" })
						end,
						w_full = true,
						wrap = true,
						text_size = "$font.size.md",
						line_height = 20,
						color = "$text.secondary",
					},
					ui.each(ui.computed(acting_checkouts), function(q: any): Node
						return ds.Checkbox {
							label = device_of(q),
							description = locale.s("shell.sidebar.project.checkout", { n = chats_in(q.id), path = q.path }),
							value = function(): boolean
								return project_remove_chosen:get()[q.id] == true
							end,
							on_change = function(on: boolean)
								local next = table.clone(project_remove_chosen:peek())
								next[q.id] = if on then true else nil
								project_remove_chosen:set(next)
							end,
						}
					end, {
						key = function(q: any): string
							return q.id
						end,
					}),
				}
			end, { w_full = true }),
		},
	}
end

-- Deleting a session, asked once more (Zeron's "Delete session?"): for
-- good, so its red Delete waits on Cancel.
local function delete_dialog(): Node
	local function says(): string
		local id = deleting:get()
		local c = if id then chat(id) else nil
		local t = if c and c.title ~= "" then (string.gsub(c.title, "%s+", " ")) else locale.s("shell.sidebar.session.new_session")
		local cloud = false
		for _, d in sessions.now:get().devices do
			if c and d.id == c.device and (d :: any).cloud then
				cloud = true
			end
		end
		local will = locale.s(if cloud then "shell.sidebar.session.will_delete_cloud" else "shell.sidebar.session.will_delete", { title = t })
		local subagents, sides = delete_subagents:get(), delete_sides:get()
		local also = if subagents > 0 and sides > 0
			then locale.s("shell.sidebar.session.also_deletes_both", { subagents = subagents, sides = sides })
			elseif subagents > 0 then locale.s("shell.sidebar.session.also_deletes_subagents", { n = subagents })
			elseif sides > 0 then locale.s("shell.sidebar.session.also_deletes_side", { n = sides })
			else nil
		if not also then
			return will
		end
		return locale.s("shell.sidebar.session.will_delete_with", { will = will, also = also })
	end
	return ds.Dialog {
		open = delete_open,
		title = ui.t("shell.sidebar.session.delete_title") :: any,
		tone = "danger",
		size = "sm",
		confirm = {
			label = ui.t("common.delete") :: any,
			on_click = function()
				local id = deleting:peek()
				deleting:set(nil)
				if not id then
					return
				end
				-- (Its title now: refused, the window has already left it.)
				local c = chat(id)
				local title = if c and c.title ~= "" then (string.gsub(c.title, "%s+", " ")) else locale.s("shell.sidebar.session.new_session")
				if route.current:peek().session == id then
					route.new_chat()
				end
				sessions.delete(id):after(function() end, function(err: any)
					ds.toast.show {
						title = locale.s("shell.sidebar.session.couldnt_delete", { title = title }),
						body = if type(err) == "string" and err ~= "" then err else nil,
						tone = "warning",
					}
				end)
			end,
		},
		-- (As its own body line, but following the session.)
		ui.text { says, w_full = true, wrap = true, text_size = "$font.size.md", line_height = 20, color = "$text.secondary" },
	}
end

-- Sleep until a time of your own: a day and a time from a minute ahead.
local function sleep_dialog(): Node
	return ds.Dialog {
		open = sleep_custom_open,
		title = ui.t("shell.activity.sleep.custom_title") :: any,
		-- Under it, the moment chosen, in words: what Sleep will do.
		body = function(): string
			local at = sleep_custom_at:get()
			if not at then
				return ""
			end
			return locale.s("shell.activity.sleep.until_when", {
				day = locale.weekday(at),
				date = locale.month_day(at),
				time = locale.clock(at),
			})
		end,
		size = "sm",
		confirm = {
			label = ui.t("shell.activity.sleep.title") :: any,
			-- (A time that has passed, or none chosen, puts nothing to sleep.)
			on_click = function()
				local id, at = sleep_custom:peek(), sleep_custom_at:peek()
				sleep_custom:set(nil)
				sleep_custom_open:set(false)
				if id and at and at > os.time() then
					activity.sleep(id, at)
				end
			end,
		},
		-- (Built as it opens: its month of days is nothing to keep.)
		build = function(): Node
			return ds.DatePicker {
				id = "sleep-custom",
				value = sleep_custom_at,
				min = function(): number
					return os.time() + 60
				end,
				label = ui.t("shell.activity.sleep.custom_title") :: any,
			}
		end,
	}
end

local function rename_dialog(): Node
	local function done()
		local id = renaming:peek()
		local title = string.match(new_title:peek(), "^%s*(.-)%s*$") or ""
		renaming:set(nil)
		rename_open:set(false)
		if id and title ~= "" then
			sessions.rename(id, string.sub(title, 1, 200))
		end
	end
	return ds.Dialog {
		open = rename_open,
		title = ui.t("shell.sidebar.session.rename_title") :: any,
		size = "sm",
		confirm = { label = ui.t("common.rename") :: any, on_click = done },
		ds.TextField { value = new_title, label = ui.t("shell.sidebar.session.name"), placeholder = function(): string
			return locale.s("shell.sidebar.session.name_placeholder")
		end, select_on_focus = true, w_full = true },
	}
end

M.Sidebar = ui.component(function(props: Props): Node
	probe.builds += 1
	probe.built_at = ui.now()
	-- (Made again, a reload: no row is rested on yet.)
	if not props.island then
		M.card:set(nil)
	end
	local head = if props.island
		then nil
		else ui.row {
			shrink = 0,
			items = "center",
			project_filter(),
			ds.IconButton {
				icon = "lucide-folder-plus",
				label = locale.t("ds.sidebar.new_project"),
				tooltip = locale.s("ds.sidebar.new_project"),
				tone = "quiet",
				on_click = add_a_project,
			},
		}
	local bar = ds.Sidebar {
		compact = shared.compact,
		inbox = shared.inbox,
		active_first = shared.active_first,
		on_pin = M.pin,
		on_settle = M.settle,
		on_rename_lost = function(id: string)
			local c = chat(id)
			if c then
				new_title:set(c.title)
				renaming:set(id)
				rename_open:set(true)
			end
		end,
		on_archive_all = function(asked: { string })
			local ids: { string } = {}
			for _, c in sessions.now:peek().chats do
				if table.find(asked, c.id) then
					table.insert(ids, c.id)
				end
			end
			if table.find(ids, route.current:peek().session or "") then
				route.new_chat()
			end
			sessions.archive_all(ids, function(n: number)
				ds.toast.show { title = locale.s("shell.sidebar.notice.archive_failed", { n = n }), tone = "danger" }
			end)
		end,
		unseen = function(): boolean
			for _, c in sessions.now:get().chats do
				if c.status == "done" then
					return true
				end
			end
			return false
		end,
		on_mark_read = function()
			for _, c in sessions.now:peek().chats do
				if c.status == "done" then
					sessions.seen(c.id)
				end
			end
		end,
		drafts = M.drafts,
		on_draft = route.open_draft,
		on_discard_draft = M.discard_draft,
		search_label = function(): string
			local only = only_project()
			for _, p in sessions.now:get().projects do
				if p.id == only then
					return locale.s("ds.sidebar.search_in", { name = p.name })
				end
			end
			return locale.s("ds.sidebar.search")
		end,
		head = head,
		peek = if props.island then nil else function(): boolean
			return M.peek_mounted:get()
		end,
		can_place = can_place,
		dim = if props.island then nil else function(): string?
			local c = route.chat:get()
			if c.name ~= "home" then
				return nil
			end
			return if c.project ~= "none" and route.project:get() then "loose" else "project"
		end,
		organize = shared.organize :: any,
		organize_order = shared.organize_order,
		sort = shared.sort :: any,
		details = shared.details,
		places = shared.places :: any,
		width = shared.width,
		default_width = fit.SIDEBAR_DEFAULT,
		shown = if props.island then nil else view.sidebar_shown,
		before = if props.island then nil else 5,
		-- (Its edge double-clicked: the chat and the panel split again, as
		-- Zeron's `reset_panel_widths`.)
		on_reset = function()
			view.seam_reset:set(view.seam_reset:peek() + 1)
		end,
		collapsed = props.collapsed,
		limit = props.limit,
		direct = props.direct,
		loading = function(): boolean
			return sessions.state:get() ~= "ready"
		end,
		island = props.island,
		selected = shared.selected,
		projects = projects,
		here = function(): string?
			local _, here = devices()
			return here
		end :: any,
		top = props.top,
		card = not props.island,
		card_x = rail.W + 5,
		options_x = if props.island then nil else top_bar.options_x,
		card_bg = function(): string
			local frame: any = boundary.load("@app/screens/shell/frame")
			return if frame and frame.framed:get() then "#00000000" else "$zeron.inset.bg"
		end,
		no_new_row = not props.island,
		-- (A device's "+", inside a project's group: on that device too.)
		on_new = function(project: string?, device: string?)
			route.new_chat(if device then project else checkout_here(project), if device then device_named(device, project) else nil)
		end,
		on_archive = M.archive,
		fan_view = function(s: any, cur: () -> any): Node
			return Pills.Fan {
				name = `sidebar-fan-{s.id}`,
				chat = s.id,
				size = 16,
				bare = true,
				tiles = function(): { any }
					return cur().fan or {}
				end,
				place = { side = "right", align = "start", offset = 8 },
			}
		end,
		archived = shelf,
		activity = activity.on,
		on_unarchive = function(id: string)
			sessions.unarchive(id)
		end,
		pages = rail.sidebar_pages,
		on_page = function(page: any)
			route.plugin_page(page.plugin, page.id)
		end,
		on_search = function()
			view.palette:set(true)
		end,
		menu = menu,
		project_menu = project_menu,
		narrowed = function(): boolean
			return narrow:get() ~= nil
		end,
		notice = if props.island then nil else filter_notice(),
		tip = if props.island then nil else foot_notes(),
		-- (The rail has no list to scroll, and its own folds.)
		offset = if props.island then nil else shared.offset,
		folded = if props.island then nil else shared.folded,
		expanded = if props.island then nil else shared.expanded,
		order =if props.island then nil else M.order,
		jump_hints = if props.island
			then nil
			else function(): boolean
				return M.jump_hints:get()
			end,
		renaming_session = if props.island then nil else renaming_row,
		on_card = if props.island
			then nil
			else function(c: Card?)
				M.card:set(c)
			end,
		on_rename = function(id: string, title: string)
			sessions.rename(id, string.sub(title, 1, 200))
		end,
	}
	if props.island then
		return bar
	end
	local function gap_now(): number
		return if fit.current:get().sidebar then 5 else 0
	end
	return ui.div {
		id = "shell-sidebar",
		hook = "shell.sidebar",
		h_full = true,
		shrink = 0,
		relative = true,
		x = function(): number
			return M.peek_x(gap_now())
		end,
		on_hover = function(inside: boolean)
			M.peek_over(inside)
		end,
		on_press = function()
			M.peek_press(true)
		end,
		on_release = function()
			M.peek_press(false)
		end,
		on_key = function(): boolean?
			M.peek_key()
			return nil
		end,
		-- (Gone only with the column's last 5 points, so the card's left edge stands still as it folds.)
		pl = function(): number
			return ui.raw(math.min(5, view.sidebar_shown:get()), "the window's gap before its card")
		end,
		ui.show(function(): boolean
			return M.peek_mounted:get()
		end, ui.div {
			absolute = true,
			left = 0,
			top = 0,
			bottom = 0,
			w = function(): number
				return M.peek_w() + gap_now() + 12
			end,
			ui.div {
				absolute = true,
				left = function(): number
					return gap_now() - 1
				end,
				top = 42,
				bottom = 8,
				w = M.peek_w,
				Glass.Panel { kind = "menu", id = "sidebar-peek-panel", w_full = true, h_full = true, pointer_events = "none" },
			},
		}),
		-- (That gap's top moves the window, as the bar beside it does.)
		ui.div { absolute = true, left = 0, top = 0, w = ui.raw(5, "the window's gap"), h = ui.raw(42, "the top bar"), window_drag = true },
		bar,
		rename_dialog(),
		delete_dialog(),
		sleep_dialog(),
		project_dialogs(),
	}
end)

-- A project's name and icon edited (Edit project…), from its header's menu
-- or Settings → Projects: the window's one editor, `start` and `also` as its
-- `open`'s.
function M.edit_project(id: string, start: { [string]: any }?, also: { string }?)
	project_editor().open(id, start, also)
	project_editor_used:set(true)
end

-- `Sidebar { … }` builds it; `sidebar.pin(id)`, `sidebar.archive(id)` and
-- `sidebar.rename(id)` act on a session as its menu does.
function M.rename(id: string)
	local just = only_project()
	local listed = false
	for _, c in sessions.now:peek().chats do
		if c.id == id then
			listed = true
			if just and not in_filter(c.project, just) then
				ds.toast.show { title = locale.s("shell.sidebar.notice.clear_filter"), tone = "danger" }
				return
			end
		end
	end
	if listed then
		if view.collapsed:peek() and not M.peek_mounted:peek() then
			fit.toggle_sidebar()
		end
		renaming_row:set(id)
		return
	end
	local c = chat(id)
	if c then
		new_title:set(c.title)
		renaming:set(id)
		rename_open:set(true)
	end
end
local function draft_key(): any
	return require("@app/screens/chat/shared")
end
local draft_frozen = ui.state("shell_sidebar_draft_frozen", nil :: string?)
-- (A draft not shown keeps where it was left for: the route says only the shown one's.)
local draft_left = ui.state("shell_sidebar_draft_left", {} :: { [string]: { target: string, project: string? } })
local function draft_project(): string?
	local c = route.chat:get()
	local p: any = if c.name ~= "home" or c.project ~= "none" then route.project:get() else nil
	return if p then p.id else nil
end
local function draft_target(): string
	local c = route.chat:get()
	local p = if c.name ~= "home" or c.project ~= "none" then route.project:get() else nil
	local folder = if p then p.name else "~"
	local _, here = devices()
	return if here then `{folder} @ {here}` else folder
end
M.drafts = function(): { any }
	local at_home = route.chat:get().name == "home"
	local active = route.draft:get()
	local just = only_project()
	local left = draft_left:get()
	local keys = draft_key().new_drafts()
	if at_home and not table.find(keys, active) then
		table.insert(keys, active)
		table.sort(keys, draft_key().new_before)
	end
	local out: { any } = {}
	for _, key in keys do
		local open = at_home and key == active
		local prompt = if open then draft_frozen:get() else draft_key().draft_of(key).text
		local was = if open then nil else left[key]
		local project = if was then was.project else draft_project()
		local filtered = just ~= nil and not (project and in_filter(project, just))
		if prompt ~= nil and not filtered then
			table.insert(out, {
				id = key,
				target = if was then was.target else draft_target(),
				prompt = (string.gsub(prompt, "%s+", " ")),
				open = open,
			})
		end
	end
	return out
end
do
	local was_home = route.chat:peek().name == "home"
	local was_key = route.draft:peek()
	local was: { target: string, project: string? }? = nil
	ui.effect(function()
		local at_home = route.chat:get().name == "home"
		local key = route.draft:get()
		local now = if at_home then { target = draft_target(), project = draft_project() } else nil
		if at_home == was_home and key == was_key then
			was = now
			return
		end
		if was_home and was then
			local all = table.clone(draft_left:peek())
			all[was_key] = was
			draft_left:set(all)
		end
		was_home, was_key, was = at_home, key, now
		if at_home then
			local d = draft_key().draft_peek(key)
			draft_frozen:set(if d.text ~= "" or #d.cargo > 0 then d.text else nil)
		else
			draft_frozen:set(nil)
		end
	end)
end
function M.discard_draft(key: string)
	draft_key().discard_draft_of(key)
	if route.draft:peek() == key then
		draft_frozen:set(nil)
	end
end

-- What a walk with the keys holds on (screens/shell/init `step`): the order, less a split pair's moves.
function M.walk_key(): string
	local parts: { string } = {}
	for _, c in sessions.now:peek().chats do
		table.insert(parts, `{c.id}:{c.updated or ""}`)
	end
	local p = shared.places:peek()
	table.insert(parts, table.concat(p.pinned, ","))
	for _, sec in p.sections do
		table.insert(parts, `{sec.id}={table.concat(sec.sessions, ",")}`)
	end
	for key in shared.folded:peek() do
		table.insert(parts, key)
	end
	table.sort(parts)
	table.insert(parts, `{shared.sort:peek()}|{table.concat(ds_session_row.levels(shared.organize:peek()), ">")}|{tostring(shared.inbox:peek())}|{tostring(shared.project:peek())}`)
	return table.concat(parts, ";")
end
function M.project_glyph(project: string?, size: number): Node?
	local now = sessions.now:peek()
	for _, p in now.projects do
		if p.id == project then
			local first = oldest_checkout(p, now.projects)
			local l = route.look(first.id) or { hue = route.hue(first.id) }
			if (first :: any).icon == nil and not l.src and not l.emoji then
				return ds_session_row.Monogram {
					name = function(): string
						return first.name
					end,
					seed = first.path or first.id,
					size = size,
				}
			end
			return ds.Tile { label = first.name, hue = l.hue, src = l.src, emoji = l.emoji, size = size }
		end
	end
	return ds_session_row.Monogram {
		name = function(): string
			return "Home"
		end,
		seed = "home",
		size = size,
	}
end
-- Working and Settled fold each launch: crates/ui's `sidebar_collapsed_groups` is in memory.
local shelves_folded = ui.state("shell_sidebar_shelves_folded", false)
if not shelves_folded:peek() then
	shelves_folded:set(true)
	local f = table.clone(shared.folded:peek())
	f["group:inbox:working"], f["group:inbox:settled"] = true, true
	shared.folded:set(f)
end

local OPEN_MS, CLOSE_MS, SLIDE_MS = 120, 300, 120
local peek_open = ui.state("shell_sidebar_peek", false)
local peek_t = ui.state("shell_sidebar_peek_t", 0)
local peek_edge = ui.state("shell_sidebar_peek_edge", false)
local peek_inside = ui.state("shell_sidebar_peek_inside", false)
local peek_handoff = ui.state("shell_sidebar_peek_handoff", false)
local peek_pressed = ui.state("shell_sidebar_peek_pressed", false)
local peek_keys = ui.state("shell_sidebar_peek_keys", false)
local peek_touched = false
local peek_left = ui.state("shell_sidebar_peek_left", 0)
local menu_held = require("@app/ds/menu").held
local split_dragging = ui.shared("split.dragging", false)
local peek_suppressed = ui.state("shell_sidebar_peek_suppressed", false)
local peek_pending: boolean? = nil
local peek_cancel: (() -> ())? = nil
local peek_going: any = nil
M.peek_mounted = ui.computed(function(): boolean
	return peek_open:get() or peek_t:get() > 0 or peek_handoff:get()
end)
function M.peek_w(): number
	local limit = fit.sidebar_room:get()
	return math.min(shared.width:get(), limit) + 2
end
function M.peek_x(gap: number): number
	if not M.peek_mounted:get() then
		return 0
	end
	if peek_handoff:get() then
		return peek_left:get()
	end
	local w = M.peek_w()
	return (8 + w) * peek_t:get() - w - gap + 1
end
function M.peek_press(down: boolean)
	if peek_open:peek() and down then
		peek_touched = true
		peek_keys:set(false)
	end
	if peek_pressed:peek() ~= (down and peek_open:peek()) then
		peek_pressed:set(down and peek_open:peek())
	end
end
function M.peek_key()
	if peek_open:peek() then
		peek_touched = true
		if not peek_keys:peek() then
			peek_keys:set(true)
		end
	end
end
-- (A press anywhere: zgx tells a script of no focus leaving a subtree, so the keys' hold ends here.)
function M.peek_press_outside()
	if peek_keys:peek() then
		peek_keys:set(false)
	end
end
function M.peek_over(inside: boolean)
	if peek_inside:peek() ~= inside then
		peek_inside:set(inside)
	end
end
local function peek_set(on: boolean)
	if peek_open:peek() == on then
		return
	end
	peek_open:set(on)
	if peek_going then
		peek_going:cancel()
	end
	peek_going = anime.animate(peek_t, motion.params({ value = if on then 1 else 0, duration = SLIDE_MS, ease = "outCubic" }))
end
-- (Nil on v2, whose view has no peek.)
local hover_setting = (view :: any).sidebar_hover
local function peek_enabled(): boolean
	local c = route.current:get()
	return hover_setting ~= nil
		and hover_setting:get()
		and view.collapsed:get()
		and c.name ~= "settings"
		and ui.window.active:get() ~= false
		and not view.palette:get()
		and not delete_open:get()
		and not rename_open:get()
		and not project_remove_open:get()
end
local function peek_held(): boolean
	return menu_held:get() > 0 or renaming_row:get() ~= nil or peek_keys:get() or peek_pressed:get()
end
local function peek_wanted(): boolean
	if not peek_enabled() or peek_suppressed:get() then
		return false
	end
	local inside, edge = peek_inside:get(), peek_edge:get()
	if peek_open:get() then
		return inside or edge or peek_held() or split_dragging:get()
	end
	return not split_dragging:get() and (edge or (peek_t:get() > 0 and inside))
end
ui.effect(function()
	if not peek_edge:get() and not peek_inside:get() and peek_suppressed:peek() then
		peek_suppressed:set(false)
	end
	if peek_handoff:get() then
		return
	end
	local wanted = peek_wanted()
	local open = peek_open:peek()
	if not peek_enabled() then
		if peek_cancel then
			peek_cancel()
			peek_cancel, peek_pending = nil, nil
		end
		peek_set(false)
		return
	end
	if wanted == open then
		if peek_cancel then
			peek_cancel()
			peek_cancel, peek_pending = nil, nil
		end
		return
	end
	if wanted and peek_t:peek() > 0 then
		if peek_cancel then
			peek_cancel()
			peek_cancel, peek_pending = nil, nil
		end
		peek_set(true)
		return
	end
	if peek_pending == wanted then
		return
	end
	if peek_cancel then
		peek_cancel()
	end
	peek_pending = wanted
	peek_cancel = ui.after(if wanted then OPEN_MS else CLOSE_MS, function()
		peek_cancel, peek_pending = nil, nil
		if peek_wanted() == wanted then
			peek_set(wanted)
		end
	end)
end)
local peek_was = view.collapsed:peek()
ui.effect(function()
	local shut = view.collapsed:get()
	if shut == peek_was then
		return
	end
	peek_was = shut
	peek_suppressed:set(not (not peek_edge:peek() and not peek_inside:peek()))
	if not shut and peek_t:peek() > 0 then
		local gap = if fit.current:peek().sidebar then 5 else 0
		local w = M.peek_w()
		peek_left:set((8 + w) * peek_t:peek() - w - gap + 1)
		peek_handoff:set(true)
		anime.animate(peek_left, motion.params({
			value = 0,
			duration = motion.sidebar.duration,
			onComplete = function()
				peek_open:set(false)
				peek_t:set(0)
				peek_handoff:set(false)
			end,
		}))
	end
end)
function M.peek_dismiss(): boolean
	if not peek_open:peek() or menu_held:peek() > 0 or peek_handoff:peek() then
		return false
	end
	peek_suppressed:set(true)
	peek_keys:set(false)
	peek_set(false)
	if peek_touched then
		peek_touched = false
		draft_key().focus:add(1)
	end
	return true
end
function M.PeekEdge(): Node
	return ui.div {
		id = "sidebar-peek-edge",
		absolute = true,
		left = rail.W,
		top = 42,
		bottom = 0,
		w = 14,
		hidden = function(): boolean
			return not hover_setting or not hover_setting:get() or not view.collapsed:get() or route.current:get().name == "settings"
		end,
		on_hover = function(inside: boolean)
			if peek_edge:peek() ~= inside then
				peek_edge:set(inside)
			end
		end,
	}
end

-- Its width, kept: the settings window's list shares it (screens/shell/
-- settings), so the two sidebars are one width, dragged in either.
M.width = shared.width

return setmetatable(M, {
	__call = function(_, props: Props): Node
		return M.Sidebar(props)
	end,
})
