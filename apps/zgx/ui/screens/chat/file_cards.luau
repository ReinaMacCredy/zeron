local ds = require("@ds")
local FileCard = require("@ds/file_card")
local FileLinks = require("@app/screens/chat/file_links")
local sessions = require("@account/sessions")
local files = require("@app/screens/files/api")

local known = ui.state("file_cards_known", {} :: { [string]: FileCard.File | false })
local asking: { [string]: { string } } = {}
-- (A path not listed is asked again after a while: an agent may write it after naming it.)
local missed: { [string]: number } = {}
local STALE_MS = 5000

local function base_name(path: string): string
	return string.match(path, "([^/]+)$") or path
end

local function settle(scope: string, paths: { string }, listed: { [string]: FileCard.File }?, whole: boolean)
	local now = known:peek()
	ui.batch(function()
		for _, path in paths do
			local key = `{scope}\0{path}`
			local f = if listed then listed[path] else nil
			if f then
				known:set_key(key, f)
			else
				missed[key] = ui.now()
				if (listed and whole) or now[key] == nil then
					known:set_key(key, false)
				end
			end
		end
	end)
end

local function ask(where: files.Target, scope: string, path: string)
	local dir = string.match(path, "^(.*)/[^/]+$") or ""
	local key = `{scope}\0{dir}`
	local waiting = asking[key]
	if waiting then
		table.insert(waiting, path)
		return
	end
	waiting = { path }
	asking[key] = waiting
	files.list(where, dir):after(function(listing: any)
		asking[key] = nil
		local listed: { [string]: FileCard.File } = {}
		local entries = if type(listing) == "table" then listing.entries else nil
		for _, e in entries or {} do
			if not e.dir then
				listed[e.path] = { path = e.path, name = e.name or base_name(e.path), size = e.size }
			end
		end
		settle(scope, waiting :: { string }, listed, type(listing) == "table" and listing.truncated ~= true)
	end, function()
		asking[key] = nil
		settle(scope, waiting :: { string }, nil, false)
	end)
end

local function probe(where: files.Target, scope: string, path: string)
	local key = `{scope}\0{path}`
	if asking[key] then
		return
	end
	asking[key] = { path }
	local function settled(f: FileCard.File | false)
		asking[key] = nil
		if not f then
			missed[key] = ui.now()
		end
		known:set_key(key, f)
	end
	files.read(where, path):after(function(r: any)
		local file = type(r) == "table" and r.readonly ~= "notRegularFile"
		settled(if file then { path = path, name = base_name(path), size = r.size, outside = true } else false)
	end, function()
		settled(false)
	end)
end

return function(session: () -> any?): FileCard.Files
	local function place(target: string): (files.Target?, string, string, boolean)
		local s = session()
		local cwd = if s then s.cwd else nil
		local r = if s and cwd then FileLinks.resolve(target, cwd) else nil
		if not (s and cwd and r) then
			return nil, "", "", false
		end
		return { chat = s.id, device = s.device, root = cwd }, r.path, `{s.id}\0{cwd}`, r.outside == true
	end
	local function look(where: files.Target, scope: string, path: string, outside: boolean)
		if outside then
			probe(where, scope, path)
		else
			ask(where, scope, path)
		end
	end
	return {
		find = function(target: string): FileCard.File | false | nil
			local where, path, scope, outside = place(target)
			if not where then
				return false
			end
			local key = `{scope}\0{path}`
			local f = known:get()[key]
			if f == nil or (f == false and ui.now() - (missed[key] or 0) > STALE_MS) then
				look(where, scope, path, outside)
			end
			return f
		end,
		check = function(target: string)
			local where, path, scope, outside = place(target)
			if where then
				look(where, scope, path, outside)
			end
		end,
		read = function(target: string): any
			local where, path = place(target)
			if not where then
				return {
					after = function(_self: any, _ok: any, err: (any) -> ())
						err("not in the chat's folder")
					end,
				}
			end
			return files.read(where, path)
		end,
		reveal = function(target: string)
			local where, path, _, outside = place(target)
			if not where or outside then
				return
			end
			files.reveal(where, path):after(function() end, function()
				ds.toast.show { title = ui.t("files.workspace.not_revealed", { name = base_name(path) }), tone = "danger" }
			end)
		end,
		here = function(): boolean
			local s = session()
			return s ~= nil and (s.device == nil or s.device == sessions.now:peek().here)
		end,
	}
end
