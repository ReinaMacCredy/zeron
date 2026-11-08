-- The deck's Queue and Plan (ds/composer_deck, its rows ds/composer_dock's)
-- with screens/chat/asks over a made-up session: what
-- tests/chat_queue.test.luau and the plan's tests in chat_tasks.test.luau drive.
-- The queue's tab is open while it has rows (the tests read them), as a person
-- who opened it; the plan's only when a test opens it.
-- The session is `test_session`; the engine answers only when told. Here, beside
-- app.luau, as a mounted script loads modules from its own directory down.

ui.theme.strict("warn")
local ds = require("@ds")
local Deck = require("@ds/composer_deck")
local Asks = require("@app/screens/chat/asks")
local store = require("@app/screens/chat/store")
ds.setup()

local session = ui.state("test_session", { id = "s1", running = true, steers = true, stopped = false, queue = {} } :: any)
local calls = ui.state("calls", "")
local said: { string } = {}
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end
local last_notice = ui.state("test_notice", "")
local no_steer_all = ui.state("test_no_steer_all", false)
local saving = ui.state("test_saving", nil :: string?)
local empty = ui.state("test_empty", false)
local narrow = ui.state("test_narrow", false)

local function patch(fields: { [string]: any })
	local s = table.clone(session:peek())
	for k, v in fields do
		s[k] = v
	end
	session:set(s)
end
local function rows(): { any }
	return table.clone(session:peek().queue or {})
end
local function without(id: string): { any }
	local out = {}
	for _, r in rows() do
		if r.id ~= id then
			table.insert(out, r)
		end
	end
	return out
end

type Held = { ok: (any) -> (), err: (any) -> (), act: () -> () }
local held: { Held } = {}
local function later(act: () -> ()): any
	local p: any = {}
	function p.after(_self: any, ok: (any) -> (), err: (any) -> ())
		table.insert(held, { ok = ok, err = err, act = act })
	end
	return p
end
local function answer_all(fn: (Held) -> ())
	local all = held
	held = {}
	for _, h in all do
		fn(h)
	end
end
ui.command {
	id = "engine_answers",
	title = "The engine does what it was asked",
	run = function()
		answer_all(function(h)
			h.act()
			h.ok(nil)
		end)
	end,
}
ui.command {
	id = "engine_refuses",
	title = "The engine refuses what it was asked",
	run = function()
		answer_all(function(h)
			h.err({ message = "failed" })
		end)
	end,
}
ui.command {
	id = "engine_gone",
	title = "The engine says the row had left",
	run = function()
		answer_all(function(h)
			h.err({ message = "gone" })
		end)
	end,
}

local stub = store :: any
local old_store = ui.state("test_old_store", false)
stub.queue = function(id: string, op: any): any
	note(`queue:{id}:{op.op}:{op.item}:{op.to or ""}`)
	if old_store:peek() then
		return nil
	end
	return later(function()
		if op.op == "move" then
			local q = without(op.item)
			for _, r in rows() do
				if r.id == op.item then
					table.insert(q, math.clamp(op.to, 1, #q + 1), r)
				end
			end
			patch { queue = q }
		elseif op.op == "send" and session:peek().steers == false and session:peek().running then
			local q = without(op.item)
			for _, r in rows() do
				if r.id == op.item then
					table.insert(q, 1, r)
				end
			end
			patch { queue = q }
		else
			patch { queue = without(op.item) }
		end
	end)
end
stub.steer_all = function(id: string, items: { string }, _message: string): any
	note(`steer_all:{id}:{table.concat(items, "+")}`)
	if no_steer_all:peek() then
		error("no host action named chat.steer_all")
	end
	return later(function()
		local q = {}
		for _, r in rows() do
			if not table.find(items, r.id) then
				table.insert(q, r)
			end
		end
		patch { queue = q }
	end)
end

local asks = Asks.bind {
	id = function(): string?
		local s = session:get()
		return if s then s.id else nil
	end,
	session = function(): any
		return session:get()
	end,
	chosen = ui.state("test_chosen", {} :: { [string]: boolean }),
	asked = ui.state("test_asked", nil :: any),
	notice = function(n: any)
		last_notice:set(if n.key then ui.t(n.key):get() else n.text)
	end,
	saving = function(): string?
		return saving:get()
	end,
}

local added: string? = nil
ui.command {
	id = "add",
	title = "A message is sent while a turn runs",
	run = function()
		added = asks.queue_add("Then run the tests", nil)
	end,
}
ui.command {
	id = "add_picture",
	title = "A picture is sent while a turn runs, its path all it has",
	run = function()
		added = asks.queue_add("Look", { { kind = "image", name = "flint.png", path = "assets/favicons/flint.png" } })
	end,
}
ui.command {
	id = "add_taken",
	title = "The engine queued it",
	run = function()
		patch { queue = (function()
			local q = rows()
			table.insert(q, { id = "eq1", text = "Then run the tests" })
			return q
		end)() }
		asks.queue_added(added, "eq1")
	end,
}
ui.command {
	id = "add_ran",
	title = "The engine ran it: the turn had ended",
	run = function()
		asks.queue_added(added, nil)
	end,
}
ui.command {
	id = "add_failed",
	title = "It did not reach the engine",
	run = function()
		asks.queue_failed(added)
	end,
}

ui.command {
	id = "send_newest",
	title = "The primary modifier and Enter, the pill empty",
	run = function()
		local list = asks.queue:peek()
		asks.on_queue_send(list[#list].id)
	end,
}

local tray = ui.state("test_tray", nil :: Deck.Tray?)
ui.effect(function()
	if #asks.queue:get() > 0 and tray:peek() == nil then
		tray:set("queue")
	end
end)

return ui.column {
	w_full = true,
	h_full = true,
	p = "$space.4",
	justify = "end",
	Deck.Dock {
		queue = asks.queue,
		can_steer = asks.can_steer,
		on_queue_send = asks.on_queue_send,
		on_queue_steer_all = asks.on_queue_steer_all,
		on_queue_remove = asks.on_queue_remove,
		on_queue_reorder = asks.on_queue_reorder,
		on_queue_edit = function(id: string)
			note(`edit:{id}`)
		end,
		todos = asks.todos,
		on_dismiss_todos = asks.on_dismiss_todos,
		live = function(): boolean
			return session:get().running == true
		end,
		composer_empty = function(): boolean
			return empty:get()
		end,
		pictures = nil,
		narrow = function(): boolean
			return narrow:get()
		end,
		open = tray,
	},
}
