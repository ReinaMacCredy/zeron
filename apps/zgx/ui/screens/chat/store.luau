-- A chat as the host keeps it (src/chat.rs): the session open in this
-- window, its transcript and what streams into it, the models to choose
-- from, and what can be asked of it. Screens read the states and call the
-- functions; a fixture replaces the functions (and sets the states) as the
-- engine would, so the chat page runs without one.
--
--   store.session:get()   the open session (Session below), nil for none
--   store.messages:get()  its transcript, ds.Transcript's messages, oldest
--                         first, each keyed by `id`
--   store.draft:get()     the reply being written ({ id, text }): it grows
--                         here, a small state, not in the whole list
--   store.catalog:get()   { state, harnesses, models } (ds.ModelPicker's)
--
--   store.open(id?)       watch that session (nil: none), the last one let go
--   store.send(req)       a message to a session: run now, or queued while
--                         a turn runs (a promise). A new chat is made with
--                         sessions.create first, then sent its first
--                         message with `fresh`.
--   store.rewind_send(req)  store.send's req with `entry`: the chat rewound
--                         to before that prompt, then the words sent there
--   store.interrupt(id)   stop its turn
--   store.approve(id, request, scope)  allow what it asked, "once" or "always"
--   store.deny(id, request)
--   store.answer(id, request, answer)  a question's answer ({ values, text })
--   store.configure(id, patch)  { model?, effort?, fast?, options?, access?, plan? }
--   store.queue(id, op)   { op = "remove" | "send" | "move", item, to? }
--   store.queue_edit(id, item), store.queue_finish(id, item, action, text?)
--                         a queued message edited in the composer
--   store.load_catalog()  the models, looked up again
--
-- What can go wrong, a promise's `message`: "offline", "signed_out",
-- "missing" (no such chat), "failed".

local Transcript = require("@ds/transcript")
local ds = require("@ds")
local shared = require("@app/screens/chat/shared")
local prefs = require("@app/screens/settings/prefs")
local route = require("@app/screens/shell/route")

local M = {}

export type Message = Transcript.Message
export type Draft = Transcript.Draft
-- What the agent asks, waiting on an answer: an approval, or a question.
export type Ask = {
	request: string,
	kind: "approval" | "question",
	approval: any?,
	question: any?,
}
export type Session = {
	id: string,
	-- "loading" while its transcript comes; "ready"; "missing" (gone, or
	-- never there); "offline" (nothing to ask).
	state: string,
	title: string?,
	project: string?,
	device: string?,
	-- Its folder on that device.
	cwd: string?,
	harness: string?,
	-- The agent its config records (nil until that's known): whose `/` list
	-- its composer reads when the catalog doesn't name its model.
	agent: string?,
	model: string?,
	-- Its model's name: its catalog's, else its id (chat.rs `session_value`).
	model_label: string?,
	effort: string?,
	fast: boolean?,
	-- The choices made of the model's options (option id to choice id).
	options: { [string]: string }?,
	access: string?,
	plan: boolean?,
	-- A turn is running.
	running: boolean?,
	-- How much of the model's context is used, 0 to 1; and what it holds,
	-- as the agent reports it (tokens in it, the window's size).
	context: number?,
	context_tokens: number?,
	context_window: number?,
	ask: Ask?,
	queue: { any }?,
	todos: { any }?,
	tasks: { any }?,
	-- A line about it (a failed send, a run that stopped), or nil; `refused` replaces `text`.
	notice: any?,
}
export type Catalog = { state: string, complete: boolean?, harnesses: { ds.Harness }, models: { ds.Model } }
export type Request = {
	-- The session it goes to; nil for a new chat.
	session: string?,
	text: string,
	-- Files and images it carries, by path.
	attachments: { { kind: string, path: string, name: string } }?,
	-- The Appshots among them (screens/chat/appshots): what the host writes
	-- of each after the words.
	appshots: { any }?,
	-- A new chat's: where and how it runs.
	project: string?,
	device: string?,
	checkout: string?,
	branch: string?,
	harness: string?,
	model: string?,
	effort: string?,
	fast: boolean?,
	options: { [string]: string }?,
	access: string?,
	plan: boolean?,
	-- A new chat's agent profile (its id): its instructions and tools go
	-- with the config.
	profile: string?,
	-- A chat just made (sessions.create): its first message, run in `cwd`,
	-- in a new worktree of it when `worktree`.
	fresh: boolean?,
	cwd: string?,
	worktree: boolean?,
	-- Its id (store.new_id), when the page shows it before it's sent.
	message_id: string?,
}

M.session = ui.shared("chat.session", nil :: Session?)
M.messages = ui.shared("chat.messages", {} :: { Message })
M.draft = ui.shared("chat.draft", { id = "", text = "" } :: Draft)
M.stats = ui.shared("chat.stats", nil :: any)
M.token_rate = ui.shared("chat.token_rate", nil :: number?)
M.turn_tokens = ui.shared("chat.turn_tokens", nil :: string?)
M.catalog = ui.shared("chat.catalog", { state = "looking", harnesses = {}, models = {} } :: Catalog)

function M.model_label(harness: string?, model: string): string?
	for _, m in M.catalog:get().models do
		if harness == nil or m.harness == harness then
			-- (The host's catalog says `<harness>/<id>`; a child says its bare id.)
			local bare = string.match(m.value, "^[^/]+/(.+)$")
			if m.value == model or bare == model then
				return m.label
			end
		end
	end
	return nil
end
-- A call nothing waits on: its failure lands in the session's notice (the
-- host sets it), so the script needn't hear it.
local function quietly(p: any)
	p:after(function() end, function() end)
end

-- The chat the host is watching for this window ("" for none). Shared, so
-- it outlives a reload as the host's watch does: a page made again asks
-- the host nothing, and only a move to another chat (or letting go, as
-- "Try again" does before asking again) reaches it.
local watching = ui.shared("chat.watching", "")
function M.open(id: string?)
	local key = id or ""
	if watching:peek() == key then
		return
	end
	watching:set(key)
	ui.invoke("chat.open", id)
end

-- A new chat on Cloud, opened before Cloud has made it (`req`: the send it
-- will get, its `session` the id it's made with): known to the host as a chat
-- on Cloud, its first message showing, sending. `created` once Cloud has made
-- it (the host watches it then); `create_failed` if Cloud didn't.
function M.creating(req: Request)
	ui.invoke("chat.creating", req)
end
function M.created(id: string)
	ui.invoke("chat.created", id)
end
function M.create_failed(id: string)
	ui.invoke("chat.create_failed", id)
end

-- A message's id, made here so its prompt shows the moment it's sent
-- (`T.send`) and stays the same row when the engine's copy arrives.
function M.new_id(): string
	local hex = "0123456789abcdef"
	local out = {}
	for i = 1, 32 do
		local n = math.random(1, 16)
		if i == 13 then
			n = 5 -- version 4
		elseif i == 17 then
			n = 9 + (n % 4) -- variant 10xx
		end
		table.insert(out, string.sub(hex, n, n))
	end
	local s = table.concat(out)
	return `{string.sub(s, 1, 8)}-{string.sub(s, 9, 12)}-{string.sub(s, 13, 16)}-{string.sub(s, 17, 20)}-{string.sub(s, 21, 32)}`
end

function M.send(req: Request): any
	return ui.call("chat.send", req)
end

function M.rewind_send(req: { [string]: any }): any
	return ui.call("chat.rewind_send", req)
end

-- A project's branch, as git where it lives has it: whether it's the
-- folder's current one, and the worktree it's checked out in.
export type Ref = { name: string, current: boolean, worktree: string? }

-- A project's branches (the folder at `path`, on `device` when that isn't
-- this one): a promise of its refs, the current first.
function M.refs(path: string, device: string?): any
	return ui.call("chat.refs", { path = path, device = device })
end

-- The clipboard's picture written out as a PNG to send (src/pasted.rs): a
-- promise of its path, nil when the clipboard holds none.
function M.paste_image(): any
	return ui.call("chat.paste_image", nil)
end

-- A send in this device's outbox (still uploading, or stopped) taken out for
-- good, with its prompt shown as on its way: a promise of whether it was
-- still there. An engine from before it can refuses it "unsupported".
function M.remove_held(id: string, item: string, message: string?): any
	return ui.call("chat.remove_held", { id = id, item = item, message = message })
end

-- A prompt that wasn't delivered, sent again.
function M.retry(id: string, message: string)
	quietly(ui.call("chat.retry", { id = id, message = message }))
end

function M.interrupt(id: string)
	quietly(ui.call("chat.interrupt", id))
end

function M.close_page(id: string, title: string)
	quietly(ui.call("chat.close_page", { id = id, title = title }))
end

-- The same, a promise: Stop says why it couldn't. (The turn alone: an agent's
-- sub-agents and background shells go on.)
function M.stop(id: string): any
	return ui.call("chat.interrupt", id)
end

function M.approve(id: string, request: string, scope: string)
	quietly(ui.call("chat.respond", { id = id, request = request, decision = scope }))
end

function M.deny(id: string, request: string)
	quietly(ui.call("chat.respond", { id = id, request = request, decision = "deny" }))
end

-- Back a page in the question being answered: a promise of what the page
-- shown again held ({ chosen = its option values, text = its typed words }),
-- nil on the first.
function M.question_back(id: string, request: string): any
	return ui.call("chat.question_back", { id = id, request = request })
end

function M.answer(id: string, request: string, answer: { values: { string }, text: string }): any
	return ui.call("chat.respond", {
		id = id,
		request = request,
		decision = "answer",
		values = answer.values,
		text = answer.text,
	})
end

-- A transcript secret card's answer: the secret, or nil to decline.
function M.answer_computer(id: string, request: string, question: string, answer: string): any
	return ui.call("chat.answer_computer", { id = id, request = request, question = question, answer = answer })
end

function M.answer_secret(id: string, request: string, secret: string?): any
	return ui.call("chat.answer_secret", { id = id, request = request, secret = secret })
end

function M.configure(id: string, patch: { [string]: any })
	local args = table.clone(patch)
	args.id = id
	quietly(ui.call("chat.configure", args))
end

-- A queued message taken to be edited here (its text comes back, its row
-- says it's being edited); then saved over ("commit") or left ("cancel").
-- Promises: a message being edited elsewhere rejects with "locked", one
-- changed meanwhile with "conflict".
function M.queue_edit(id: string, item: string): any
	return ui.call("chat.queue_edit", { id = id, item = item })
end

-- (`attachments`: on a commit, the files it carries now, as the composer's
-- chips pack them; nil keeps the ones it had.)
function M.queue_finish(id: string, item: string, action: string, text: string?, attachments: { any }?): any
	return ui.call("chat.queue_finish", { id = id, item = item, action = action, text = text, attachments = attachments })
end

-- (`failed`: told when it didn't go, a send or steer the person is waiting on.)
function M.queue(id: string, op: { [string]: any }, failed: (() -> ())?): any
	local args = table.clone(op)
	args.id = id
	local call = ui.call("chat.queue", args)
	if failed then
		call:after(function() end, failed)
	else
		quietly(call)
	end
	return call
end

function M.steer_all(id: string, items: { string }, message_id: string): any
	return ui.call("chat.steer_all", { id = id, items = items, message_id = message_id })
end

-- Words put into the composer's draft (a plugin's `zeron.compose.insert`,
-- a shortcut): at its end, on a line of their own unless `inline`, and
-- the composer given the keys.
function M.insert(text: string, inline: boolean?)
	local now = shared.text:peek()
	local sep = if now == "" or inline or string.sub(now, -1) == "\n" then "" else "\n"
	shared.text:set(now .. sep .. text)
	shared.focus:add(1)
end

function M.load_catalog()
	ui.invoke("chat.catalog")
end

-- A background process of `session` stopped on its host: a promise of
-- whether it's stopping (false: it had ended already).
function M.stop_task(session: string, task: string): any
	return ui.call("chat.stop_task", { session = session, task = task })
end

-- `op`: "switch", "wait", "hold" (Cancel), "arm", "cancel".
function M.limit(id: string, op: string): any
	local ok, p = pcall(ui.call, "chat.limit", { id = id, op = op })
	return if ok then p else nil
end

function M.cancel_delegate(session: string, task: string): any
	local ok, p = pcall(ui.call, "chat.cancel_delegate", { session = session, task = task })
	return if ok then p else nil
end

function M.waiting_answer(args: any): any
	local ok, p = pcall(ui.call, "chat.waiting_answer", args)
	return if ok then p else nil
end

function M.waiting_back(id: string, request: string): any
	local ok, p = pcall(ui.call, "chat.waiting_back", { id = id, request = request })
	return if ok then p else nil
end

function M.roster_watch(docs: { string })
	pcall(ui.invoke, "chat.roster_watch", { docs = docs })
end

function M.shell_watch(shells: { any })
	pcall(ui.invoke, "chat.shell_watch", { shells = shells })
end

-- The host holds one set a window, and a closing card's cleanup runs after the next card has said what it wants.
local function wants(send: ({ any }) -> ()): () -> (({ [string]: any }) -> ())
	local by: { [any]: { [string]: any } } = {}
	local sent = ""
	local function flush()
		local all: { [string]: any } = {}
		for _, items in by do
			for key, item in items do
				all[key] = item
			end
		end
		local keys: { string } = {}
		for key in all do
			table.insert(keys, key)
		end
		table.sort(keys)
		local joined = table.concat(keys, "\0")
		if joined ~= sent then
			sent = joined
			local out: { any } = {}
			for _, key in keys do
				table.insert(out, all[key])
			end
			send(out)
		end
	end
	return function()
		local me = {}
		ui.on_cleanup(function()
			by[me] = nil
			flush()
		end)
		return function(items: { [string]: any })
			by[me] = items
			flush()
		end
	end
end

M.want_docs = wants(function(docs: { any })
	M.roster_watch(docs)
end)
M.want_shells = wants(function(shells: { any })
	M.shell_watch(shells)
end)

-- The device the composer runs on (nil: this one): `M.catalog` is its
-- agents and models from now on (src/chat.rs `catalog_on`).
function M.catalog_on(device: string?)
	ui.invoke("chat.catalog_on", { device = device })
end

M.dismiss_cards = nil :: (() -> boolean)?
-- The composer's `chat_new_defaults`, set as it loads (a module's persist is its own).
M.new_defaults = nil :: any

function M.new_chat_model(want: string?): any
	local wanted = want or (if M.new_defaults then M.new_defaults:get().model else nil)
	local models = M.catalog:get().models or {}
	for _, m in models do
		if m.value == wanted then
			return m
		end
	end
	return models[1]
end

-- A bare Escape no one else took (a menu, a dialog, a mention list, a
-- question): with Settings' "Escape stops the agent" on, it stops the
-- chat shown's running turn, as Zeron's does (settings.rs
-- `escape_stops_active_agent`). True if it did.
-- The same for a chat beside the one shown (a side chat with the keyboard).
function M.escape_chat(id: string, running: boolean): boolean
	if shared.let_go() then
		return true
	end
	if not prefs.escape_stops:peek() or not running then
		return false
	end
	M.interrupt(id)
	return true
end

function M.escape(): boolean
	if M.dismiss_cards and M.dismiss_cards() then
		return true
	end
	-- Files shown carried over the chat: Escape lets go of them first.
	if shared.let_go() then
		return true
	end
	if not prefs.escape_stops:peek() then
		return false
	end
	local now = route.current:peek()
	local s = M.session:peek()
	if now.name ~= "session" or not s or s.id ~= now.session or not s.running then
		return false
	end
	M.interrupt(s.id)
	return true
end

function M.set_work_tree(id: string, path: string?): any
	return ui.call("chat.set_work_tree", { id = id, path = path })
end

return M
