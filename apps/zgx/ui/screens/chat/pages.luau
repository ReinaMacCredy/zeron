-- What a transcript's agent pages reach of their chat: picks on its draft (cargo kind "pick"), what was sent, its folder.

local shared = require("@app/screens/chat/shared")
local sessions = require("@account/sessions")
local files = require("@app/screens/files/api")
local route = require("@app/screens/shell/route")
local boundary = require("@app/screens/shell/boundary")
local FileLinks = require("@app/screens/chat/file_links")

local M = {}

M.lit = ui.shared("chat.picks.lit", nil :: string?)

-- (Up to a later card of the same title: what was sent there is that card's.)
function M.sent_of(list: { any }, title: string, message: string): { list: { any }, locked: { [string]: boolean } }
	local out, locked = {}, {}
	local after = false
	for _, m in list do
		if m.id == message then
			after = true
		elseif after and m.role == "assistant" then
			local again = false
			for _, b in m.blocks or {} do
				if b.kind == "html" and b.title == title then
					again = true
				end
			end
			if again then
				break
			end
		elseif after and m.role == "user" then
			for _, a in m.attachments or {} do
				if a.kind == "pick" and a.card == title then
					table.insert(out, { group = a.group or "", code = a.code, label = a.label })
					if (a.group or "") ~= "" then
						locked[a.group] = true
					end
				end
			end
		end
	end
	return { list = out, locked = locked }
end

function M.place(key: string, card: string, title: string, p: any, sent: { list: { any }, locked: { [string]: boolean } })
	local group = p.group or ""
	if group ~= "" and sent.locked[group] then
		return
	end
	if group == "" then
		for _, s in sent.list do
			if (s.group or "") == "" and s.label == p.label then
				return
			end
		end
	end
	local list = table.clone(shared.peek_cargo_of(key))
	local at: number? = nil
	for i, it in list do
		if it.kind == "pick" and it.card == card and (it.group or "") == group and (group ~= "" or it.code == p.code) then
			at = i
		end
	end
	local locked = {}
	for g in sent.locked do
		table.insert(locked, g)
	end
	table.sort(locked)
	local name = group
	for _, g in p.groups or {} do
		if g.code == group then
			name = g.name
		end
	end
	local id = `pick-{card}-{if group ~= "" then group else p.code}`
	if at then
		local it = list[at]
		if group ~= "" and it.code ~= p.code then
			local swapped = table.clone(it)
			swapped.code, swapped.label, swapped.name = p.code, p.label, p.label
			if #(p.groups or {}) > 0 then
				swapped.groups = p.groups
			end
			list[at] = swapped
		else
			table.remove(list, at)
			id = ""
		end
	else
		table.insert(list, {
			id = id,
			kind = "pick",
			name = p.label,
			card = card,
			card_title = title,
			group = group,
			group_name = name,
			code = p.code,
			label = p.label,
			groups = p.groups or {},
			locked = locked,
		})
	end
	shared.set_cargo_of(key, list)
	if id ~= "" then
		M.lit:set(id)
		ui.after(700, function()
			if M.lit:peek() == id then
				M.lit:set(nil)
			end
		end)
	end
end

function M.pages(key: () -> string?, messages: State<{ any }>, session: () -> any): any
	local function here(): boolean
		local s = session()
		return s == nil or s.device == nil or s.device == sessions.now:peek().here
	end
	local function target(): any
		local s = session()
		return { chat = if s then s.id else nil, device = if s then s.device else nil, root = if s then s.cwd else nil }
	end
	return {
		chosen = function(card: string): { any }
			local k = key()
			if not k then
				return {}
			end
			local out = {}
			for _, it in shared.cargo_of(k) do
				if it.kind == "pick" and it.card == card then
					table.insert(out, it)
				end
			end
			return out
		end,
		sent = function(title: string, message: string): any
			return M.sent_of(messages:get(), title, message)
		end,
		pick = function(card: string, title: string, message: string, p: any)
			local k = key()
			if k then
				M.place(k, card, title, p, M.sent_of(messages:peek(), title, message))
			end
		end,
		remote = function(): boolean
			return not here()
		end,
		device_name = function(): string?
			local s = session()
			if not s or not s.device then
				return nil
			end
			for _, d in sessions.now:peek().devices or {} do
				if d.id == s.device then
					return d.name
				end
			end
			return nil
		end,
		resolve = FileLinks.resolve,
		root = function(): string?
			local s = session()
			return if s then s.cwd else nil
		end,
		reveal = function(path: string)
			files.reveal(target(), path):after(function() end, function() end)
		end,
		open_external = function(path: string)
			files.open_external(target(), path):after(function() end, function() end)
		end,
		open_in_zeron = function(path: string)
			local s = session()
			local panel: any = boundary.load("@app/screens/panel")
			if s and panel and panel.open_file then
				(panel.open_file :: any)(s.id, path, route.project:peek())
			end
		end,
	}
end

return M
