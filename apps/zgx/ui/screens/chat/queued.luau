local M = {}

type Add = {
	id: string,
	text: string,
	cargo: { any }?,
	before: { [string]: boolean },
	real: string?,
	answered: number?,
}
type Overlay = {
	adds: { Add },
	gone: { [string]: boolean },
	removing: { [string]: boolean },
	front: string?,
	order: { string }?,
	keys: { [string]: string },
}

local OVERLAY = ui.shared("chat.queue.overlay", {} :: { [string]: Overlay })
local made = ui.shared("chat.queue.made", 0)

-- A row sent on at once (the turn had just ended) is taken and never listed.
M.LISTED_MS = 1500

local function of(all: { [string]: Overlay }, session: string): Overlay
	return all[session] or { adds = {}, gone = {}, removing = {}, keys = {} }
end

local function change(session: string, fn: (Overlay) -> ())
	local all = table.clone(OVERLAY:peek())
	local was = of(all, session)
	local o: Overlay = {
		adds = table.clone(was.adds),
		gone = table.clone(was.gone),
		removing = table.clone(was.removing),
		front = was.front,
		order = was.order,
		keys = table.clone(was.keys),
	}
	fn(o)
	all[session] = o
	OVERLAY:set(all)
end

local function matches(o: Overlay, engine: { any }): { [number]: string }
	local claimed: { [string]: boolean } = {}
	local out: { [number]: string } = {}
	for i, a in o.adds do
		for _, r in engine do
			if a.real and r.id == a.real then
				out[i] = r.id
				break
			end
		end
	end
	for _, id in out do
		claimed[id] = true
	end
	for i, a in o.adds do
		if not out[i] and not a.real then
			for _, r in engine do
				if not claimed[r.id] and not a.before[r.id] and r.text == a.text then
					out[i] = r.id
					claimed[r.id] = true
					break
				end
			end
		end
	end
	return out
end

local function in_order(order: { string }, engine: { any }): boolean
	local listed: { [string]: boolean } = {}
	for _, r in engine do
		listed[r.id] = true
	end
	local wanted: { string } = {}
	for _, id in order do
		if listed[id] then
			table.insert(wanted, id)
		end
	end
	local at = 1
	local named: { [string]: boolean } = {}
	for _, id in order do
		named[id] = true
	end
	for _, r in engine do
		if named[r.id] then
			if wanted[at] ~= r.id then
				return false
			end
			at += 1
		end
	end
	return true
end

function M.rows(session: string?, engine: { any }): { any }
	if not session then
		return engine
	end
	local o = OVERLAY:get()[session]
	if not o then
		return engine
	end
	local matched = matches(o, engine)
	local keys = table.clone(o.keys)
	for i, id in matched do
		keys[id] = o.adds[i].id
	end
	local hidden = table.clone(o.gone)
	for i, id in matched do
		if o.gone[o.adds[i].id] then
			hidden[id] = true
		end
	end
	local out: { any } = {}
	for _, r in engine do
		if not hidden[r.id] then
			local key = keys[r.id]
			local removing = o.removing[r.id]
			if key or removing then
				local row = table.clone(r)
				row.key = key
				row.removing = removing
				table.insert(out, row)
			else
				table.insert(out, r)
			end
		end
	end
	if o.order then
		local by_id: { [string]: any } = {}
		for _, r in out do
			by_id[r.id] = r
		end
		local placed: { any } = {}
		for _, id in o.order do
			if by_id[id] then
				table.insert(placed, by_id[id])
				by_id[id] = nil
			end
		end
		for _, r in out do
			if by_id[r.id] then
				table.insert(placed, r)
			end
		end
		out = placed
	end
	if o.front then
		for i, r in out do
			if r.id == o.front and i > 1 then
				table.remove(out, i)
				table.insert(out, 1, r)
				break
			end
		end
	end
	local now = ui.now()
	for i, a in o.adds do
		local let_go = a.answered ~= nil and now - a.answered > M.LISTED_MS
		if not matched[i] and not let_go and not o.gone[a.id] then
			table.insert(out, { id = a.id, key = a.id, text = a.text, cargo = a.cargo, sending = true })
		end
	end
	return out
end

function M.add(session: string, engine: { any }, text: string, cargo: { any }?): string
	local n = made:peek() + 1
	made:set(n)
	local id = `queued:{n}`
	local before: { [string]: boolean } = {}
	for _, r in engine do
		before[r.id] = true
	end
	change(session, function(o)
		table.insert(o.adds, { id = id, text = text, cargo = cargo, before = before })
	end)
	return id
end

function M.added(session: string, row: string, id: string?)
	change(session, function(o)
		for i, a in o.adds do
			if a.id == row then
				local now = table.clone(a)
				now.real = id
				now.answered = ui.now()
				o.adds[i] = now
			end
		end
	end)
	ui.after(M.LISTED_MS + 100, function()
		local o = OVERLAY:peek()[session]
		if not o then
			return
		end
		for _, a in o.adds do
			if a.id == row then
				M.forget(session, row)
			end
		end
	end)
end

function M.forget(session: string, row: string)
	change(session, function(o)
		o.gone[row] = nil
		for i, a in o.adds do
			if a.id == row then
				table.remove(o.adds, i)
				break
			end
		end
	end)
end

function M.hide(session: string, ids: { string })
	change(session, function(o)
		for _, id in ids do
			o.gone[id] = true
		end
	end)
end

function M.unhide(session: string, ids: { string })
	change(session, function(o)
		for _, id in ids do
			o.gone[id] = nil
		end
	end)
end

function M.removing(session: string, id: string, on: boolean)
	change(session, function(o)
		o.removing[id] = if on then true else nil
	end)
end

function M.front(session: string, id: string)
	change(session, function(o)
		o.front = id
	end)
end

function M.unfront(session: string, id: string)
	change(session, function(o)
		if o.front == id then
			o.front = nil
		end
	end)
end

-- `to` is a place among the rows shown, which differ from the engine's by those hidden and on their way.
function M.move(session: string, shown: { any }, engine: { any }, from: number, to: number): number
	local after = table.clone(shown)
	local row = table.remove(after, from)
	table.insert(after, to, row)
	local listed: { [string]: number } = {}
	local at = 0
	for _, r in engine do
		if r.id ~= row.id then
			at += 1
			listed[r.id] = at
		end
	end
	local place = 1
	for i = to - 1, 1, -1 do
		local before = listed[after[i].id]
		if before then
			place = before + 1
			break
		end
	end
	local order: { string } = {}
	for _, r in after do
		if not r.sending then
			table.insert(order, r.id)
		end
	end
	change(session, function(o)
		o.order = order
	end)
	return place
end

function M.unmove(session: string)
	change(session, function(o)
		o.order = nil
	end)
end

function M.prune(session: string?, engine: { any })
	if not session then
		return
	end
	local o = OVERLAY:peek()[session]
	if not o then
		return
	end
	local listed: { [string]: boolean } = {}
	for _, r in engine do
		listed[r.id] = true
	end
	local adding: { [string]: boolean } = {}
	for _, a in o.adds do
		adding[a.id] = true
	end
	local matched = matches(o, engine)
	local stale = false
	for i in matched do
		stale = stale or o.adds[i] ~= nil
	end
	for id in o.gone do
		stale = stale or not (listed[id] or adding[id])
	end
	for id in o.removing do
		stale = stale or not listed[id]
	end
	for id in o.keys do
		stale = stale or not listed[id]
	end
	local function front_done(front: string?): boolean
		return front ~= nil and (not listed[front] or (engine[1] ~= nil and engine[1].id == front))
	end
	local order_done = o.order ~= nil and in_order(o.order, engine)
	stale = stale or front_done(o.front) or order_done
	if not stale then
		return
	end
	change(session, function(n)
		local adds: { Add } = {}
		for i, a in n.adds do
			if matched[i] then
				n.keys[matched[i]] = a.id
				if n.gone[a.id] then
					n.gone[a.id] = nil
					n.gone[matched[i]] = true
				end
			else
				table.insert(adds, a)
			end
		end
		n.adds = adds
		local still: { [string]: boolean } = {}
		for _, a in n.adds do
			still[a.id] = true
		end
		for id in table.clone(n.gone) do
			if not (listed[id] or still[id]) then
				n.gone[id] = nil
			end
		end
		for id in table.clone(n.removing) do
			if not listed[id] then
				n.removing[id] = nil
			end
		end
		for id in table.clone(n.keys) do
			if not listed[id] then
				n.keys[id] = nil
			end
		end
		if front_done(n.front) then
			n.front = nil
		end
		if order_done then
			n.order = nil
		end
	end)
end

return M
