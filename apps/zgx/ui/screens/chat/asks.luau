-- What a chat's composer shows of its session beyond its draft, and what's
-- done with it, for one chat by its id: what the agent asks (a question,
-- an approval), the queue, the todos and the background tasks (the tray).
-- The window's composer (the chat shown) and a split pane's (its own chat,
-- focused or not) are made from the same, so a chat shows the same in
-- either, and a pane out of focus is as live as the one in it.
--
--   local a = Asks.bind { id = fn, session = fn, chosen = state, asked = state, notice = fn }
--   ds.Composer { question = a.question, approval = a.approval, chosen = a.chosen,
--     on_pick = a.on_pick, on_answer = a.on_answer, …, queue = a.queue, todos = a.todos,
--     tasks = a.tasks }
--
-- `session` is the chat's record as the host publishes it (the window's
-- chat's, or a lane's); `chosen` the options picked in its question, which
-- start afresh as another question is asked (`asked`, the request they're
-- for, and its chat).

local store = require("@app/screens/chat/store")
local queued = require("@app/screens/chat/queued")
local Dock = require("@ds/composer_dock")
local Carried = require("@app/screens/chat/carried")
local references = require("@ds/references")
local T = require("@ds/transcript")
local HtmlPicks = require("@ds/html_picks")
local Pages = require("@app/screens/chat/pages")

local M = {}


-- A queued message edited in a chat's composer, by whichever composer shows
-- that chat: `text` its draft, `editing` which message and the draft it set
-- aside (put back as the edit ends), `focus` gives it the keys, `say` a
-- notice. `cargo`/`set_cargo`: the draft's chips, where the message's
-- files show while it's edited (the draft's own set aside meanwhile).
export type Edits = {
	id: () -> string?,
	text: State<string>,
	editing: State<{ item: string, chat: string?, aside: string, aside_cargo: { any }? }?>,
	focus: () -> (),
	say: (tone: string, key: string) -> (),
	cargo: (() -> { any })?,
	set_cargo: (({ any }) -> ())?,
	-- The draft's chip map (ds/references): the message's mentions shown as
	-- chips while it's edited, and links again as it's saved.
	refs: (() -> string)?,
}

local chips = 0
-- A queued message's files (the host's `queued_attachment`s) as chips.
local function chips_of(attachments: { any }?): { any }
	local out = {}
	for _, a in attachments or {} do
		chips += 1
		table.insert(out, {
			id = `queued-file-{chips}`,
			kind = a.kind,
			name = a.name,
			src = a.src,
			path = a.path,
			on_host = true,
		})
	end
	return out
end

function M.queue_edits(e: Edits): { [string]: any }
	-- ("commit" saves what's in the draft, "cancel" leaves it as it was.)
	local function finish(action: string)
		local now = e.editing:peek()
		local id = if now then now.chat or e.id() else nil
		if not now or not id then
			return
		end
		local words = if e.refs then references.encode(e.refs(), e.text:peek()) else e.text:peek()
		local tail = Pages.editing:peek()[now.item]
		if tail and e.cargo then
			local chosen = {}
			for _, it in e.cargo() do
				if it.kind == "pick" then
					table.insert(chosen, it)
				end
			end
			words = HtmlPicks.with_open(HtmlPicks.with_picks(words, HtmlPicks.cards(chosen)), Pages.open(id, store.messages:peek(), chosen))
			if tail ~= "" then
				words = if words == "" then tail else `{words}\n\n{tail}`
			end
		end
		Pages.editing:set_key({ now.item }, nil)
		-- The files it carries now: its chips, as edited.
		local files = if e.cargo then Carried.pack(e.cargo(), "", "").attachments else nil
		ui.batch(function()
			e.editing:set(nil)
			e.text:set(now.aside)
			if e.set_cargo and now.aside_cargo then
				e.set_cargo(now.aside_cargo)
			end
		end)
		store.queue_finish(id, now.item, action, if action == "commit" then words else nil, if action == "commit" then files else nil):after(function() end, function(err: any)
			local why = err and err.message
			e.say("danger", if why == "conflict" then "chat.composer.edit_conflict" else "chat.composer.edit_not_saved")
		end)
	end
	return {
		finish = finish,
		on_queue_edit = function(item: string)
			local id = e.id()
			if not id or e.editing:peek() then
				return
			end
			store.queue_edit(id, item):after(function(got: any)
				ui.batch(function()
					e.editing:set { item = item, chat = id, aside = e.text:peek(), aside_cargo = if e.cargo then e.cargo() else nil }
					local text = if got then got.text else ""
					local files = chips_of(if got then got.attachments else nil)
					local head, cards, tail = HtmlPicks.split(text)
					local picked = if #cards > 0 and e.set_cargo and e.refs then Pages.edited(id, head, cards, item) else nil
					if picked then
						text = picked.text
						table.move(picked.cargo, 1, #picked.cargo, #files + 1, files)
						Pages.editing:set_key({ item }, tail)
					end
					e.text:set(if e.refs then references.decode(e.refs(), text) else text)
					if e.set_cargo then
						e.set_cargo(files)
					end
				end)
				e.focus()
			end, function(err: any)
				e.say("warning", if err and err.message == "locked" then "chat.composer.edit_locked" else "chat.composer.edit_unavailable")
			end)
		end,
		on_queue_save = function()
			finish("commit")
		end,
		on_queue_cancel = function()
			finish("cancel")
		end,
	}
end

export type Bind = {
	id: () -> string?,
	session: () -> any,
	chosen: State<{ [string]: boolean }>,
	asked: State<any>,
	notice: ((any) -> ())?,
	saving: (() -> string?)?,
	-- A notice, when a queued message's send or steer doesn't go.
	say: ((tone: string, key: string) -> ())?,
	-- Its transcript's name (ds/transcript's: a side chat's); none, the
	-- chat page's. (A function: the one it names as it steers, the window's
	-- composer's following the pane at its foot.)
	name: (string | () -> string?)?,
}

local DISMISSED = ui.shared("chat.todos.dismissed", {} :: { [string]: string })

local function signature(items: { any }): string
	local parts: { string } = {}
	for _, t in items do
		table.insert(parts, `{t.state}\0{t.text}`)
	end
	return table.concat(parts, "\0")
end

local function reason(err: any): string
	if type(err) == "table" and err.message then
		return tostring(err.message)
	end
	return tostring(err)
end

function M.bind(b: Bind): { [string]: any }
	local ask = ui.computed(function(): store.Ask?
		local s = b.session()
		return if s then s.ask else nil
	end)
	local question = ui.computed(function(): any
		local a = ask:get()
		if not a or a.kind ~= "question" then
			return nil
		end
		if a.question.inline then
			local q = table.clone(a.question)
			q.request = a.request
			return q
		end
		return a.question
	end)
	local approval = ui.computed(function(): any
		local a = ask:get()
		return if a and a.kind == "approval" then a.approval else nil
	end)
	-- (Fresh for another question of the same chat: moving to another
	-- chat brings its own picks with its draft, screens/chat/shared.)
	ui.effect(function()
		local a = ask:get()
		local now = { id = b.id(), request = if a then a.request else nil }
		local was = b.asked:peek()
		if type(was) ~= "table" or was.id ~= now.id or was.request ~= now.request then
			b.asked:set(now)
			if type(was) == "table" and was.id == now.id then
				b.chosen:set({})
			end
		end
	end)
	local function list(key: string): any
		return ui.computed(function(): { any }
			local s = b.session()
			return if s and s[key] then s[key] else {}
		end)
	end
	local engine_queue = list("queue")
	local queue = ui.computed(function(): { any }
		local s = b.session()
		local rows = queued.rows(b.id(), engine_queue:get())
		local saving = if b.saving then b.saving() else nil
		local compacting = s ~= nil and s.compacting == true
		if not saving and not compacting then
			return rows
		end
		local out: { any } = {}
		for i, r in rows do
			local row = table.clone(r)
			row.saving = if r.id == saving then true else nil
			row.after_compaction = if compacting then true else nil
			out[i] = row
		end
		return out
	end)
	-- (Only on a list from this session: one not ready has said nothing of its queue.)
	ui.effect(function()
		local s = b.session()
		local id = b.id()
		if s and s.id == id and s.queue then
			queued.prune(id, s.queue)
		end
	end)
	local function say(key: string, err: any?)
		if not b.notice then
			-- (Upstream's composer says a key alone: no room for `{err}`.)
			if b.say and (err == nil or key == "chat.composer.queue_send_failed") then
				b.say("danger", if err == nil then key else "chat.composer.didnt_send")
			end
			return
		end
		if err == nil then
			b.notice { tone = "danger", key = key, dismissable = true }
		else
			b.notice { tone = "danger", text = ui.t(key, { err = reason(err) }):get(), dismissable = true }
		end
	end
	-- An old host's session has neither: it reads as it always did.
	local function can_steer(): boolean?
		local s = b.session()
		if not s then
			return false
		end
		if s.steers == nil and s.stopped == nil then
			return s.running == true
		end
		return s.steers
	end
	-- `from`, the chat it was queued in, as a row's answer can come after a switch.
	local function send_row(item: string, row: any, from: string?)
		local id = from or b.id()
		if not id then
			return
		end
		local s = b.session()
		local here = s ~= nil and s.id == id
		if not here then
			s = nil
		end
		local carries = row.cargo ~= nil and #row.cargo > 0
		local next_turn = s ~= nil and s.running == true and not carries and s.steers == false
		local function deliver()
			local p = store.queue(id, { op = "send", item = item }) :: any
			if not p then
				queued.unhide(id, { row.id, item })
				return
			end
			if next_turn then
				-- (Only moved up: a hide from before it was listed lifts.)
				queued.unhide(id, { row.id, item })
				queued.front(id, item)
			else
				queued.hide(id, { item })
			end
			p:after(function()
				if next_turn then
					ui.after(queued.LISTED_MS, function()
						queued.unfront(id, item)
					end)
				end
			end, function(err: any)
				if next_turn then
					queued.unfront(id, item)
				else
					queued.unhide(id, { row.id, item })
				end
				if reason(err) ~= "gone" then
					say("chat.composer.queue_send_failed", err)
				end
			end)
		end
		-- Sent now, its prompt glides to the top and holds the runway, as a sent one does (the host's copy has the queued row's id).
		if next_turn then
			deliver()
		else
			T.steer(deliver, item, if type(b.name) == "function" then b.name() else b.name)
		end
	end
	local then_send: { [string]: any } = {}
	-- Each act is on the chat and the question it was asked about, as they
	-- are as it's taken.
	local function on_ask(f: (id: string, request: string) -> ())
		return function()
			local id, a = b.id(), ask:peek()
			if id and a then
				f(id, a.request)
			end
		end
	end
	return {
		ask = ask,
		question = question,
		approval = approval,
		chosen = b.chosen,
		on_pick = function(value: string)
			local q = question:peek()
			local now = table.clone(b.chosen:peek())
			if q and q.multi then
				now[value] = not now[value] or nil
			else
				now = { [value] = true }
			end
			b.chosen:set(now)
		end,
		on_answer = function(answer: any)
			local id, a = b.id(), ask:peek()
			if id and a then
				local picks = b.chosen:peek()
				b.chosen:set({})
				store.answer(id, a.request, answer):after(function() end, function(err: any)
					b.chosen:set(picks)
					answer.keep()
					say("ds.pills.answer_failed", err)
				end)
			end
		end,
		on_close_question = on_ask(store.deny),
		-- Back a page: it shows again with what was picked on it.
		on_back = function(set_text: (string) -> ())
			local id, a = b.id(), ask:peek()
			if not (id and a) then
				return
			end
			store.question_back(id, a.request):after(function(page: any)
				local now: { [string]: boolean } = {}
				for _, v in if page then page.chosen or page else {} do
					now[v] = true
				end
				b.chosen:set(now)
				if page and page.text and page.text ~= "" then
					set_text(page.text)
				end
			end, function() end)
		end,
		-- (The engine allows once, each time it's asked.)
		can_always = false,
		on_approve = function(scope: string)
			local id, a = b.id(), ask:peek()
			if id and a then
				store.approve(id, a.request, scope)
			end
		end,
		on_deny = function(change: any?)
			local id, a = b.id(), ask:peek()
			if not (id and a) then
				return
			end
			if not change then
				store.deny(id, a.request)
				return
			end
			if not string.find(change.text, "%S") then
				if b.notice then
					b.notice { tone = "warning", key = "ds.composer.changes_need_words", dismissable = true }
				elseif b.say then
					b.say("warning", "ds.composer.changes_need_words")
				end
				change.focus()
				return
			end
			store.request_changes(id, a.request, change.text):after(function() end, function(err: any)
				change.keep()
				say("ds.pills.answer_failed", err)
			end)
		end,
		queue = queue,
		engine_queue = engine_queue,
		can_steer = can_steer,
		on_queue_send = function(item: string)
			local id = b.id()
			if not id then
				return
			end
			local row: any = nil
			for _, r in queue:peek() do
				if r.id == item then
					row = r
				end
			end
			if not row then
				return
			end
			if row.sending then
				then_send[item] = row
				queued.hide(id, { item })
				return
			end
			if row.locked or row.review or row.editing or row.removing then
				return
			end
			local s = b.session()
			local carries = row.cargo ~= nil and #row.cargo > 0
			if not carries and can_steer() == nil and s and s.running == true then
				return
			end
			send_row(item, row)
		end,
		-- Plain rows only: one with files would interrupt, and Steer never does.
		on_queue_steer_all = function()
			local id = b.id()
			local steer_all = (store :: any).steer_all
			if not id or not steer_all then
				return
			end
			local ids: { string } = {}
			for _, r in queue:peek() do
				if Dock.steerable(r) then
					table.insert(ids, r.id)
				end
			end
			if #ids < 2 then
				return
			end
			local ok, p = pcall(steer_all, id, ids, store.new_id())
			if not ok or not p then
				return
			end
			queued.hide(id, ids)
			p:after(function() end, function(err: any)
				queued.unhide(id, ids)
				say("chat.composer.queue_send_failed", err)
			end)
		end,
		on_queue_remove = function(item: string)
			local id = b.id()
			if not id then
				return
			end
			local p = store.queue(id, { op = "remove", item = item }) :: any
			if not p then
				return
			end
			queued.removing(id, item, true)
			p:after(function()
				queued.hide(id, { item })
				queued.removing(id, item, false)
			end, function(err: any)
				queued.removing(id, item, false)
				say(if reason(err) == "gone" then "chat.composer.queue_gone" else "chat.composer.queue_remove_failed")
			end)
		end,
		on_queue_reorder = function(from: number, to: number)
			local id = b.id()
			local rows = queue:peek()
			local row = rows[from]
			if not id or not row or row.sending or row.review or row.locked or row.removing then
				return
			end
			local engine_to = queued.move(id, rows, engine_queue:peek(), from, to)
			local p = store.queue(id, { op = "move", item = row.id, to = engine_to }) :: any
			if not p then
				queued.unmove(id)
				return
			end
			p:after(function()
				ui.after(queued.LISTED_MS, function()
					queued.unmove(id)
				end)
			end, function(err: any)
				queued.unmove(id)
				say("chat.composer.queue_reorder_failed", err)
			end)
		end,
		-- What it hands back goes with the answer: that answer is for the chat
		-- it was queued in, whichever shows by then.
		queue_add = function(text: string, cargo: { any }?): any
			local id = b.id()
			if not id then
				return nil
			end
			local shown: { any }? = nil
			if cargo then
				shown = {}
				for _, it in cargo do
					local row = table.clone(it)
					row.src = it.src or (if it.kind == "image" then it.path else nil)
					table.insert(shown :: { any }, row)
				end
			end
			return { session = id, row = queued.add(id, engine_queue:peek(), text, shown) }
		end,
		queue_added = function(q: any, got: any)
			local waiting = then_send[q.row]
			then_send[q.row] = nil
			if type(got) == "string" then
				queued.added(q.session, q.row, got)
				if waiting then
					send_row(got, waiting, q.session)
				end
			else
				queued.forget(q.session, q.row)
			end
		end,
		queue_failed = function(q: any)
			then_send[q.row] = nil
			queued.forget(q.session, q.row)
		end,
		todos = ui.computed(function(): { any }
			local s = b.session()
			local items = if s and s.todos then s.todos else {}
			local id = b.id()
			if id and DISMISSED:get()[id] == signature(items) then
				return {}
			end
			return items
		end),
		on_dismiss_todos = function()
			local id = b.id()
			local s = b.session()
			if id and s and s.todos then
				DISMISSED:set_key(id, signature(s.todos))
			end
		end,
		tasks = list("tasks"),
	}
end

return M
