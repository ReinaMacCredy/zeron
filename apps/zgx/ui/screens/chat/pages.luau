-- What a transcript's agent pages reach of their chat: picks on its draft (cargo kind "pick"), what was sent.

local shared = require("@app/screens/chat/shared")
local sessions = require("@account/sessions")
local references = require("@ds/references")
local HtmlPicks = require("@ds/html_picks")
local store = require("@app/screens/chat/store")
local queued = require("@app/screens/chat/queued")
local Held = require("@app/screens/chat/held")

local M = {}

-- Queued messages whose picks are chips in the draft they are edited in, each with what followed its picks' block.
M.editing = ui.shared("chat.picks.editing", {} :: { [string]: string })

-- Messages steered into a running turn (no queue row, no prompt shown yet), by chat: their picks are sent from then, not from when the transcript has them.
local STEERED = ui.shared("chat.picks.steered", {} :: { [string]: { any } })
local steers = 0

local function picks_in(text: string): { any }
	local picks = {}
	local _, cards = HtmlPicks.split(text)
	for _, c in cards do
		for _, p in c.picks do
			table.insert(picks, { kind = "pick", card = c.card, group = p.group or "", code = p.code, label = p.label })
		end
	end
	return picks
end

local function carrying(messages: { any }, picks: { any }): number
	local n = 0
	for _, m in messages do
		local has = false
		for _, a in (if m.role == "user" then m.attachments else nil) or {} do
			for _, p in picks do
				has = has or (a.kind == "pick" and a.card == p.card and (a.group or "") == p.group and a.code == p.code)
			end
		end
		n += if has then 1 else 0
	end
	return n
end

-- (A minute on it counts as lost: its picks open again rather than stay locked for good.)
local function landed(messages: { any }, e: any): boolean
	return carrying(messages, e.picks) > e.had or os.time() - e.at > 60
end

-- What `M.unsteer` takes, should the message not go.
function M.steer(chat: string, text: string): string?
	local picks = picks_in(text)
	if #picks == 0 then
		return nil
	end
	local messages = store.messages:peek()
	local list = {}
	for _, e in STEERED:peek()[chat] or {} do
		if not landed(messages, e) then
			table.insert(list, e)
		end
	end
	steers += 1
	local id = `steered:{steers}`
	table.insert(list, { id = id, picks = picks, had = carrying(messages, picks), at = os.time() })
	STEERED:set_key({ chat }, list)
	return id
end

function M.unsteer(chat: string, id: string?)
	local list = {}
	for _, e in STEERED:peek()[chat] or {} do
		if e.id ~= id then
			table.insert(list, e)
		end
	end
	STEERED:set_key({ chat }, list)
end

-- A message waiting in the chat's queue has sent its picks, until they are chips again in an edit of it (`except`: the one being taken to edit) or it leaves the queue.
function M.with_queue(chat: string?, messages: { any }, except: string?): { any }
	local s = store.session:get()
	if not chat or not s or s.id ~= chat then
		return messages
	end
	local engine = s.queue or {}
	local seen: { [string]: boolean } = {}
	for _, m in messages do
		seen[m.id] = true
	end
	local out: { any }? = nil
	local function add(r: any)
		if seen[r.id] or M.editing:get()[r.id] or r.id == except then
			return
		end
		seen[r.id] = true
		local picks = picks_in(r.text or "")
		if #picks > 0 then
			out = out or table.clone(messages)
			table.insert(out :: { any }, { id = r.id, role = "user", attachments = picks })
		end
	end
	for _, r in engine do
		add(r)
	end
	for _, r in queued.rows(chat, engine) do
		if r.sending then
			add(r)
		end
	end
	for _, h in Held.of(chat) do
		add({ id = Held.PREFIX .. h.id, text = h.req.text })
	end
	for _, e in STEERED:get()[chat] or {} do
		if not landed(messages, e) then
			out = out or table.clone(messages)
			table.insert(out :: { any }, { id = e.id, role = "user", attachments = e.picks })
		end
	end
	return out or messages
end

-- (Up to a later card of the same title: what was sent there is that card's.)
function M.sent_of(list: { any }, title: string, message: string): { list: { any }, locked: { [string]: boolean }, closed: boolean }
	local out, locked = {}, {}
	local after, closed = false, false
	for _, m in list do
		if m.id == message then
			after = true
			for _, b in m.blocks or {} do
				if b.kind == "html" and b.title == title and b.closed == true then
					closed = true
				end
			end
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
	return { list = out, locked = locked, closed = closed }
end

local function swap_token(key: string, shown: string?, words: string?): boolean
	if not shown then
		return false
	end
	local text = shared.draft_peek(key).text
	local at, to = shared.find_chip(text, shown)
	if not at or not to then
		return false
	end
	if not words and string.sub(text, to + 1, to + 1) == " " then
		to += 1
	end
	shared.set_draft_of(key, "text", string.sub(text, 1, at - 1) .. (words or "") .. string.sub(text, to + 1))
	return true
end

local function token(key: string, p: any): string
	return references.add(shared.refs_key(key), { kind = "pick", name = p.label, code = if (p.group or "") ~= "" then p.code else "·", link = p.code })
end

local function staged(key: string, card: string, title: string, p: any, name: string, locked: { string }): any
	local group = p.group or ""
	return {
		id = `pick-{card}-{if group ~= "" then group else p.code}`,
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
		shown = token(key, p),
	}
end

-- (The code as the prompt's words have it, where its chip stood.)
local function chip_in(text: string, code: string, shown: string): string
	local at = 1
	while true do
		local s, e = string.find(text, code, at, true)
		if not s or not e then
			return if text == "" then shown else `{text} {shown}`
		end
		local before, after = string.sub(text, s - 1, s - 1), string.sub(text, e + 1, e + 1)
		if (before == "" or string.find(before, "^[ \t\r\n\f\v]")) and (after == "" or string.find(after, "^[ \t\r\n\f\v]")) then
			return string.sub(text, 1, s - 1) .. shown .. string.sub(text, e + 1)
		end
		at = e + 1
	end
end

-- A queued message's picks (`HtmlPicks.split`) for the draft it is edited in: its words with each code a chip again, and the chips. Nil when a page of its isn't shown here or a pick has no group: it is edited as text.
function M.edited(chat: string, words: string, cards: { any }, except: string): { text: string, cargo: { any } }?
	local messages = M.with_queue(chat, store.messages:peek(), except)
	local text, cargo = words, {}
	for _, c in cards do
		local page: any = nil
		for _, e in M.shown:peek()[chat] or {} do
			if e.title == c.card then
				page = e
			end
		end
		if not page then
			return nil
		end
		local groups, names = {}, {}
		for _, g in page.groups do
			table.insert(groups, { code = g.code, name = g.name })
			names[g.code] = g.name
		end
		local locked = {}
		for g in M.sent_of(messages, page.title, page.message).locked do
			table.insert(locked, g)
		end
		table.sort(locked)
		for _, p in c.picks do
			if not p.group or not names[p.group] then
				return nil
			end
			local it = staged(chat, page.card, page.title, { group = p.group, code = p.code, label = p.label, groups = groups }, names[p.group], locked)
			table.insert(cargo, it)
			text = chip_in(text, p.code, it.shown)
		end
	end
	return { text = text, cargo = cargo }
end

-- (The composer's field takes a pick's words a frame or so later: the draft's next change, that soon, is the pick's; any other is the person's typing.)
local placing: { [string]: boolean } = {}

function M.take_placed(key: string): boolean
	local was = placing[key] == true
	placing[key] = nil
	return was
end

function M.place(key: string, card: string, title: string, p: any, sent: { list: { any }, locked: { [string]: boolean }, closed: boolean })
	local group = p.group or ""
	if sent.closed or (group ~= "" and sent.locked[group]) then
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
	placing[key] = true
	ui.after(200, function()
		placing[key] = nil
	end)
	ui.batch(function()
		if at then
			local it = list[at]
			if group ~= "" and it.code ~= p.code then
				local swapped = table.clone(it)
				swapped.code, swapped.label, swapped.name = p.code, p.label, p.label
				if #(p.groups or {}) > 0 then
					swapped.groups = p.groups
				end
				swapped.shown = token(key, p)
				list[at] = swapped
				shared.set_cargo_of(key, list)
				if not swap_token(key, it.shown, swapped.shown) then
					shared.put_words(key, swapped.shown)
				end
			else
				table.remove(list, at)
				shared.set_cargo_of(key, list)
				swap_token(key, it.shown, nil)
			end
		else
			local it = staged(key, card, title, p, name, locked)
			table.insert(list, it)
			shared.set_cargo_of(key, list)
			shared.put_words(key, it.shown)
		end
	end)
end

M.shown = ui.shared("chat.picks.pages", {} :: { [string]: { any } })

function M.ready(chat: string, card: string, title: string, message: string, groups: { any })
	local all = table.clone(M.shown:peek())
	local list: { any } = {}
	for _, e in all[chat] or {} do
		if e.card ~= card then
			table.insert(list, e)
		end
	end
	if #groups > 0 then
		table.insert(list, { card = card, title = title, message = message, groups = groups })
	end
	all[chat] = list
	M.shown:set(all)
end

local function left_of(chat: string, messages: { any }, picked: { any }, all: boolean?): { { page: any, left: { any } } }
	messages = M.with_queue(chat, messages)
	local here: { [string]: boolean } = {}
	for _, m in messages do
		here[m.id] = true
	end
	local out = {}
	for _, e in M.shown:get()[chat] or {} do
		if here[e.message] then
			local sent = M.sent_of(messages, e.title, e.message)
			local chosen: { [string]: boolean } = {}
			for _, it in picked do
				if it.kind == "pick" and it.card == e.card then
					chosen[it.group or ""] = true
				end
			end
			local left = {}
			for _, g in e.groups do
				if not chosen[g.code] and not sent.locked[g.code] then
					table.insert(left, g)
				end
			end
			if not sent.closed and (#left > 0 or all) then
				table.insert(out, { page = e, left = left })
			end
		end
	end
	return out
end

M.tray = ui.state("chat_picks_tray", false)
M.focus = ui.state("chat_picks_focus", nil :: string?)

function M.waiting(chat: string?, messages: { any }): (any?, number)
	if not chat then
		return nil, 0
	end
	local picked = shared.cargo_of(chat)
	local drafted: { [string]: boolean } = {}
	for _, it in picked do
		if it.kind == "pick" then
			drafted[it.card] = true
		end
	end
	local focus = M.focus:get()
	local best: any = nil
	for _, it in left_of(chat, messages, picked, true) do
		if #it.left > 0 or drafted[it.page.card] then
			if it.page.card == focus then
				return it.page, #it.left
			end
			if not best or drafted[it.page.card] or not drafted[best.page.card] then
				best = it
			end
		end
	end
	if best then
		return best.page, #best.left
	end
	return nil, 0
end

-- (A page this message picks on says what it left in its own block.)
function M.open(chat: string?, messages: { any }, carried: { any }): { { title: string, left: { any } } }
	if not chat then
		return {}
	end
	local out = {}
	for _, it in left_of(chat, messages, {}) do
		local touched = false
		for _, c in carried do
			if c.kind == "pick" and c.card == it.page.card then
				touched = true
			end
		end
		for i = #out, 1, -1 do
			if out[i].title == it.page.title then
				table.remove(out, i)
			end
		end
		if not touched then
			table.insert(out, { title = it.page.title, left = it.left })
		end
	end
	return out
end

function M.pages(key: () -> string?, messages: State<{ any }>, session: () -> any): any
	local function here(): boolean
		local s = session()
		return s == nil or s.device == nil or s.device == sessions.now:peek().here
	end
	local function tray_shows(card: string): boolean
		local k = key()
		local e = if k and M.tray:get() then M.waiting(k, messages:get()) else nil
		return e ~= nil and e.card == card
	end
	-- (Its press is outside the tray and closes it: the click after goes by what the tray showed then.)
	local pressed: { card: string, open: boolean }? = nil
	return {
		tray = function(card: string): { n: number, total: number, open: boolean }?
			local k = key()
			local page: any = nil
			for _, e in (if k then M.shown:get()[k] else nil) or {} do
				if e.card == card then
					page = e
				end
			end
			if not k or not page then
				return nil
			end
			local sent = M.sent_of(M.with_queue(k, messages:get()), page.title, page.message)
			local chosen: { [string]: boolean } = {}
			for _, it in shared.cargo_of(k) do
				if it.kind == "pick" and it.card == card then
					chosen[it.group or ""] = true
				end
			end
			local n, locked = 0, 0
			for _, g in page.groups do
				n += if chosen[g.code] or sent.locked[g.code] then 1 else 0
				locked += if sent.locked[g.code] then 1 else 0
			end
			if sent.closed or locked >= #page.groups then
				return nil
			end
			return { n = n, total = #page.groups, open = tray_shows(card) }
		end,
		hold_tray = function(card: string)
			pressed = { card = card, open = tray_shows(card) }
		end,
		toggle_tray = function(card: string)
			local open = if pressed and pressed.card == card then pressed.open else tray_shows(card)
			pressed = nil
			if open then
				M.tray:set(false)
			else
				M.focus:set(card)
				M.tray:set(true)
			end
		end,
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
			return M.sent_of(M.with_queue(key(), messages:get()), title, message)
		end,
		pick = function(card: string, title: string, message: string, p: any)
			local k = key()
			if k then
				M.place(k, card, title, p, M.sent_of(M.with_queue(k, messages:peek()), title, message))
			end
		end,
		ready = function(card: string, title: string, message: string, groups: { any })
			local k = key()
			if k then
				M.ready(k, card, title, message, groups)
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
	}
end

return M
