local Invocations = require("@app/screens/chat/invocations")
local ZeronCommands = require("@app/screens/chat/zeron_commands")
local prefs = require("@app/screens/settings/prefs")
local files = require("@app/screens/files/api")

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

	local lists = ui.computed(function(): any
		-- (No project chosen: nowhere to search, and nothing that could come.)
		local nowhere = b.target() == nil
		local given: { [string]: any } = { ["@"] = if nowhere then {} else results:get() }
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
			if item.zeron then
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
