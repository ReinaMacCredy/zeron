-- Bots from src/bots.rs `bots.list`; each host action under `pcall`, as an old host has none.

export type Avatar = { glyph: string?, frame: string?, color: string?, image: string? }
export type Bot = {
	id: string,
	name: string,
	chat: string,
	avatar: Avatar,
	pinned: boolean,
	archived: boolean,
	unread: boolean,
	template: string?,
	harness: string?,
	model: string?,
	access: string?,
	instructions: string?,
	created: number?,
}

local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")

local M = {}

M.list = ui.shared("bots.list", { state = "idle", bots = {} } :: { state: string, bots: { Bot } })

-- (The host's call, which a test swaps for an old host's.)
M.raw_call = ui.call
local function call(name: string, args: any): any
	local ok, p = pcall(M.raw_call, name, args)
	return if ok then p else nil
end

M.host = {
	create = function(args: any): any
		return call("bots.create", args)
	end,
	update = function(args: any): any
		return call("bots.update", args)
	end,
	delete = function(args: any): any
		return call("bots.delete", args)
	end,
	image = function(args: any): any
		return call("bots.image", args)
	end,
}

local function all(): { Bot }
	return (M.list:get().bots or {}) :: { Bot }
end

function M.pinned(): { Bot }
	local out: { Bot } = {}
	for _, b in all() do
		if b.pinned and not b.archived then
			table.insert(out, b)
		end
	end
	return out
end

function M.unpinned(): { Bot }
	local out: { Bot } = {}
	for _, b in all() do
		if not b.pinned and not b.archived then
			table.insert(out, b)
		end
	end
	return out
end

function M.active(): { Bot }
	local out: { Bot } = {}
	for _, b in all() do
		if not b.archived then
			table.insert(out, b)
		end
	end
	return out
end

function M.archived(): { Bot }
	local out: { Bot } = {}
	for _, b in all() do
		if b.archived then
			table.insert(out, b)
		end
	end
	return out
end

function M.find(id: string?): Bot?
	for _, b in all() do
		if b.id == id then
			return b
		end
	end
	return nil
end

function M.of_chat(chat: string?): Bot?
	if not chat then
		return nil
	end
	for _, b in all() do
		if b.chat == chat then
			return b
		end
	end
	return nil
end

local function root_of(chat: string): string
	local up: { [string]: string } = {}
	for parent, kids in sessions.now:get().children or {} do
		for _, k in kids do
			up[k.id] = parent
		end
	end
	local at = chat
	for _ = 1, 16 do
		local parent = up[at]
		if not parent then
			break
		end
		at = parent
	end
	return at
end

M.open = ui.computed(function(): Bot?
	local c = route.current:get()
	return if c.name == "session" then M.of_chat(root_of(c.session)) else nil
end)

M.on_page = ui.computed(function(): boolean
	local c = route.current:get() :: any
	return c.name == "plugin" and c.plugin == "zeron" and c.page == "bots"
end)

M.mode = ui.computed(function(): boolean
	return M.on_page:get() or M.open:get() ~= nil
end)

function M.go(bot: Bot)
	if bot.unread then
		M.host.update { id = bot.id, unread = false }
	end
	route.open_session(bot.chat)
end

-- A Bot just made, by its chat (its row may not have arrived yet).
function M.go_chat(chat: string)
	route.open_session(chat)
end

-- The image at `path` as an avatar's: a promise of its data URL.
function M.image(path: string): any
	return M.host.image { path = path }
end

function M.gallery()
	route.plugin_page("zeron", "bots")
end

M.sort = ui.persist("bots_sessions_sort", "status")
M.compact = ui.persist("bots_sessions_compact", false)

function M.create(args: { name: string, avatar: Avatar?, template: string?, instructions: string?, picks: any?, project: string?, device: string? }): any
	return M.host.create(args)
end

function M.rename(id: string, name: string): any
	return M.host.update { id = id, name = name }
end

function M.set_avatar(id: string, avatar: Avatar): any
	return M.host.update { id = id, avatar = avatar }
end

function M.pin(id: string, pinned: boolean): any
	return M.host.update { id = id, pinned = pinned }
end

function M.mark_unread(id: string, unread: boolean): any
	return M.host.update { id = id, unread = unread }
end

function M.archive(id: string, archived: boolean): any
	return M.host.update { id = id, archived = archived }
end

function M.delete(id: string): any
	return M.host.delete { id = id }
end

return M
