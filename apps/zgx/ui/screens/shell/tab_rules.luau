-- crates/ui shell/tabs.rs `sync_open_tabs`, `auto_open_tab_ids`.

local sessions = require("@account/sessions")
local splits = require("@app/screens/shell/split")
local route = require("@app/screens/shell/route")

local seeded = ui.persist("shell_tabs_seeded", false)
local since = os.time()
-- (Kept across a reload of the scripts: a run going on then isn't a new one.)
local last = ui.state("shell_tabs_statuses", {} :: { [string]: string })

local function asking(status: string?): boolean
	return status == "input" or status == "approval"
end

ui.effect(function()
	local now = sessions.now:get()
	-- (As Zeron's waits to have heard both: a chat's state unheard isn't idle, and
	-- what it says when it comes would read as a run starting.)
	if now.state ~= "ready" or now.live == false then
		return
	end
	if not seeded:get() then
		seeded:set(true)
		local active: { any } = {}
		for _, c in now.chats do
			if c.status then
				table.insert(active, c)
			end
		end
		table.sort(active, function(a: any, b: any): boolean
			return a.updated > b.updated
		end)
		local ids: { string } = {}
		for _, c in active do
			table.insert(ids, c.id)
		end
		splits.add_tabs(ids)
	end
	local was = last:peek()
	local is: { [string]: string } = {}
	local opened: { string } = {}
	for _, c in now.chats do
		local status = c.status
		is[c.id] = status or ""
		local before = was[c.id]
		-- (First seen: one made since this launch starts from rest, an older one is only being listed.)
		if before == nil and c.created > since then
			before = ""
		end
		-- (Gone quiet, not finished: its return is no new run.)
		if c.stale and before ~= nil then
			is[c.id] = before
		elseif before ~= nil and status ~= before then
			local new_run = status == "working" and before ~= "working" and not asking(before)
			if new_run or (asking(status) and not asking(before)) then
				table.insert(opened, c.id)
			end
		end
	end
	last:set(is)
	if #opened > 0 then
		splits.add_tabs(opened)
	end
	local alive = table.clone(is)
	for _, list in now.children or {} do
		for _, child in list do
			alive[child.id] = ""
		end
	end
	local focused = splits.focused:peek()
	-- (The chat showing may be one just sent, which the chats haven't heard of yet, or a
	-- launch's, which it gives a moment to arrive: gone once it was seen.)
	local function gone(id: string): boolean
		return alive[id] == nil and (id ~= focused or was[id] ~= nil)
	end
	if splits.reconcile(gone) then
		route.forget_sessions(gone)
	end
end)

return { seeded = seeded }
