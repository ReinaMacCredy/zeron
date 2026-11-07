local Invocations = require("@app/screens/chat/invocations")
local ZeronCommands = require("@app/screens/chat/zeron_commands")
local prefs = require("@app/screens/settings/prefs")
local files = require("@app/screens/files/api")
local sessions = require("@account/sessions")

local M = {}

local SEARCH_DEBOUNCE, SEARCH_RETRY, PREFETCH_DELAY = 80, 250, 400

export type Bind = {
	-- (Names what it keeps: a reload reads nothing again, two composers keep apart.)
	key: string?,
	agent: () -> string?,
	target: () -> Invocations.Target?,
	in_chat: () -> boolean,
	attachments: (() -> { any })?,
}

local function search_words(why: string?): string
	local said = string.lower(why or "")
	if why == "offline" then
		return ui.t("chat.invocations.unreachable"):get()
	elseif string.find(said, "unknown method", 1, true) or string.find(said, "update zeron", 1, true) then
		return ui.t("chat.completions.search_outdated"):get()
	end
	return ui.t("chat.completions.search_failed"):get()
end

-- At most this many threads at once (Zeron's THREAD_MENTION_LIMIT).
M.THREAD_LIMIT = 6

-- Threads whose title holds the query (any case) or whose id starts with it,
-- on `scope.device`, apart from `scope.exclude`; the scope's project first,
-- then the rest, each in the list's order; as `@` rows.
function M.thread_items(chats: { any }, query: string, scope: { device: string?, project: string?, exclude: string? }, projects: { [string]: string }): { any }
	local q = string.lower(string.match(query or "", "^%s*(.-)%s*$") or "")
	local own, other = {}, {}
	for _, c in chats do
		if c.id ~= scope.exclude and (scope.device == nil or c.device == scope.device) then
			local title = string.match(string.gsub(c.title or "", "%s+", " "), "^%s*(.-)%s*$") or ""
			if title == "" then
				title = ui.t("chat.completions.new_session"):get()
			end
			if q == "" or string.find(string.lower(title), q, 1, true) or string.sub(string.lower(c.id), 1, #q) == q then
				table.insert(if c.project == scope.project then own else other, { c = c, title = title })
			end
		end
	end
	for _, x in other do
		table.insert(own, x)
	end
	local out: { any } = {}
	for i, x in own do
		if i > M.THREAD_LIMIT then
			break
		end
		local c = x.c
		table.insert(out, {
			value = `thread:{c.id}`,
			key = `thread-{c.id}`,
			label = x.title,
			detail = if c.project then projects[c.project] else nil,
			consume = true,
			thread = { id = c.id, mark = c.mark or c.agent, icon = c.agent_icon, status = c.status },
		})
	end
	return out
end

function M.bind(b: Bind): { [string]: any }
	local kept = b.key or "chat"
	local results = ui.state(`{kept}_mention_results`, nil :: { any }?)
	local search_failed = ui.state(`{kept}_mention_failed`, nil :: string?)
	local searched = ui.state(`{kept}_mention_searched`, nil :: string?)
	local read_open = ui.state(`{kept}_invocations_open`, nil :: string?)
	local warmed = ui.state(`{kept}_invocations_warmed`, nil :: string?)
	-- (Told by ds/composer_mentions: the caret's token, which a draft's end alone wouldn't say.)
	local token = ui.state(`{kept}_mention_token`, nil :: any)

	local function invocation_key(): string?
		return Invocations.key(b.agent(), b.target())
	end

	-- The threads `@` can mention (the shipping app's `thread_matches`): this
	-- device's (where the prompt runs and reads), its project's first, never
	-- the chat being written in, the first few matching what's typed.
	local function threads(): { any }?
		local t = token:get()
		if not t or t.trigger ~= "@" then
			return nil
		end
		local place = b.target()
		local now = sessions.now:get()
		local here_chat = if place then place.chat else nil
		local device = if place then place.device else nil
		local project = if place then place.space else nil
		if here_chat then
			for _, c in now.chats do
				if c.id == here_chat then
					device = device or c.device
					project = project or c.project
				end
			end
		end
		device = device or now.here
		local names: { [string]: string } = {}
		for _, p in now.projects or {} do
			names[p.id] = p.name
		end
		return M.thread_items(now.chats, t.query, { device = device, project = project, exclude = here_chat }, names)
	end

	local lists = ui.computed(function(): any
		-- (No project chosen: nowhere to search, and nothing that could come.)
		local nowhere = b.target() == nil
		local given: { [string]: any } = { ["@"] = if nowhere then {} else results:get() }
		given.threads = threads()
		if b.attachments then
			given.attachments = b.attachments()
		end
		local agent = b.agent()
		local catalog = Invocations.catalog(invocation_key())
		local commands = Invocations.items(catalog, "command")
		local skills = Invocations.items(catalog, "skill")
		given["$"] = if nowhere then {} else skills
		-- No agent to ask, or nowhere to ask it about: Zeron's own alone. Its list still coming: nil.
		if agent == nil or nowhere then
			commands = {}
		elseif commands == nil then
			return given
		end
		local slash = table.clone(commands)
		local taken: { [string]: boolean } = {}
		for _, c in slash do
			taken[c.value] = true
		end
		if not prefs.completion(agent).separate then
			for _, skill in skills or {} do
				table.insert(slash, skill)
			end
		end
		for _, row in ZeronCommands.rows(taken, b.in_chat()) do
			table.insert(slash, row)
		end
		given["/"] = slash
		return given
	end)

	ui.effect(function()
		local t = token:get()
		local key = if t and t.trigger ~= "@" then invocation_key() else nil
		if key == read_open:peek() then
			return
		end
		read_open:set(key)
		local agent, target = b.agent(), b.target()
		if key and agent and target then
			warmed:set(key)
			Invocations.load(agent, target)
		end
	end)
	-- (The timer goes with this effect's run: clicking through chats replaces it unread.)
	ui.effect(function()
		local key = invocation_key()
		if not key or key == warmed:peek() or token:get() ~= nil then
			return
		end
		ui.after(PREFETCH_DELAY, function()
			warmed:set(key)
			local agent, target = b.agent(), b.target()
			if agent and target and Invocations.key(agent, target) == key then
				Invocations.load(agent, target)
			end
		end)
	end)

	local search_round = 0
	ui.effect(function()
		local t = token:get()
		if not t or t.trigger ~= "@" then
			if searched:peek() ~= nil then
				search_round += 1
				searched:set(nil)
				results:set(nil)
				search_failed:set(nil)
			end
			return
		end
		local place = b.target()
		if not place then
			return
		end
		local key = `{place.chat or place.space or ""}:{t.query}`
		if key == searched:peek() then
			return
		end
		local where: any = if place.chat then { chat = place.chat } else { space = place.space, device = place.device }
		local query = t.query
		-- (The run before's timer went as this one began. Written once it
		-- searches, so a run that stopped it first doesn't count as searched.)
		ui.after(SEARCH_DEBOUNCE, function()
			searched:set(key)
			search_failed:set(nil)
			search_round += 1
			local round = search_round
			local function attempt(may_retry: boolean)
				files.search(where, query):after(function(found: any)
					if round ~= search_round then
						return
					end
					local items = {}
					for _, f in found or {} do
						table.insert(items, {
							value = f.path,
							label = string.match(f.path, "([^/]+)/?$") or f.path,
							detail = string.match(f.path, "^(.*)/[^/]+/?$"),
							file = not f.dir,
							dir = f.dir,
							searched = true,
							query = query,
							link = require("@app/screens/chat/shared").file_link(f.path, f.dir),
						})
					end
					results:set(items)
				end, function(err: any)
					if round ~= search_round then
						return
					end
					local why = if type(err) == "table" then err.message else err
					if why == "offline" and may_retry then
						ui.after(SEARCH_RETRY, function()
							if round == search_round then
								attempt(false)
							end
						end)
						return
					end
					search_failed:set(search_words(why))
					results:set({})
				end)
			end
			attempt(true)
		end)
	end)

	return {
		mentions = lists,
		on_mention = function(item: any, trigger: string, token_text: string)
			if item.thread then
				require("@app/screens/chat/shared").mention_chat(item.thread.id)
			elseif item.zeron then
				local place = b.target()
				ZeronCommands.run(item.zeron, if place then place.chat else nil)
			elseif trigger == "@" then
				Invocations.remember(item, trigger, token_text)
			else
				Invocations.remember(item, trigger)
			end
		end,
		mention_enabled = function(trigger: string): boolean
			if trigger == "$" then
				return prefs.completion(b.agent()).dollar
			end
			return true
		end,
		mention_empty = setmetatable({}, {
			__index = function(_, trigger: string): string?
				if trigger == "/" and not prefs.completion(b.agent()).separate then
					return ui.t("chat.composer.commands_or_skills"):get()
				end
				return nil
			end,
		}) :: any,
		mention_failed = function(trigger: string): string?
			if trigger == "@" then
				return search_failed:get()
			end
			local catalog = Invocations.catalog(invocation_key())
			if trigger == "$" then
				return Invocations.failed(catalog, "skill")
			end
			return Invocations.failed(catalog, "command")
				or (if prefs.completion(b.agent()).separate then nil else Invocations.failed(catalog, "skill"))
		end,
		mention_token = function(t: any)
			token:set(t)
		end,
		command_of = function(draft: string): string?
			return ZeronCommands.of_text(draft, (lists:peek() or {})["/"] or {})
		end,
	}
end

return M
