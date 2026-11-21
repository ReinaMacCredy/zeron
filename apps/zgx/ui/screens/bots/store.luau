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

local M = {}

-- Every Bot, as the engine orders them: the pinned ones first, in the order
-- they were pinned, then the rest, newest first.
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
}

local function all(): { Bot }
	return (M.list:get().bots or {}) :: { Bot }
end

-- The Bots in use (not archived): those pinned, and the rest.
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

-- The Bot whose chat this is, if it is one's.
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

-- A promise of `{ id, chat }`, or nil where the host can't.
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

-- The Bot and its chat with it; whoever calls asks first.
function M.delete(id: string): any
	return M.host.delete { id = id }
end

return M
