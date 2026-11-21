-- What the chat page's parts share: the one composer's draft, what it
-- carries and the passages quoted into it, each chat's own (as Zeron's
-- composer keeps them, crates/ui composer.rs `drafts` and `attachments`:
-- per chat, a new chat's under its own key, kept for the run, not saved
-- across launches); how tall the docked composer is (the transcript
-- leaves that room); and a bump that hands the composer the keys.
--
-- The composer binds `text`, `cargo` and `quotes`: the chat shown's. Moving
-- to another chat (`show`) puts them away under the chat left and takes out
-- the next one's (empty if none), in place: the composer stays one node.

local Quote = require("@ds/quote")
local references = require("@ds/references")
local AttachmentMentions = require("@ds/attachment_mentions")
local Annotation = require("@ds/annotation")
local sessions = require("@account/sessions")
local agents = require("@account/agents")
local cloud = require("@account/cloud")
local locale = require("@ds/locale")
local toast = require("@ds/toast")
local ChatLinks = require("@app/screens/chat/chat_links")
local composer_surface = require("@ds/composer_surface")
local route = require("@app/screens/shell/route")

local Draft = require("@ds/draft_text")
local M = {}

-- The draft, what it carries, the options picked in a question being
-- answered, and where the transcript is scrolled to. Shared state, not a
-- module's own: a template replacing any chat module (this one included)
-- and every hot reload find them where they were.
M.text = ui.shared("chat.composer.text", "")
M.cargo = ui.shared("chat.composer.cargo", {} :: { any })
M.quotes = ui.shared("chat.composer.quotes", {} :: { [string]: string })
M.chosen = ui.shared("chat.composer.chosen", {} :: { [string]: boolean })
-- Whether two of a chat's picks are the same: tables by their entries, none
-- and off alike.
function M.same_pick(a: any, b: any): boolean
	if type(a) == "table" or type(b) == "table" then
		local x: { [string]: any } = if type(a) == "table" then a else {}
		local y: { [string]: any } = if type(b) == "table" then b else {}
		for k, v in x do
			if y[k] ~= v then
				return false
			end
		end
		for k, v in y do
			if x[k] ~= v then
				return false
			end
		end
		return true
	end
	return (a or false) == (b or false)
end

-- What a chat runs, as its chip shows it: its model, or with none of its own
-- the row its agent marks default, else its first (gpui's `default_model`);
-- its level, kept to what the model offers, else the one it starts on.
function M.resolve_model(s: any, catalog: any): { model: string?, effort: string? }
	local value: string? = s.model
	local found: any = nil
	for _, m in catalog.models do
		if value and m.value == value then
			found = m
			break
		elseif not value and m.harness == s.harness and (found == nil or (m.default and not found.default)) then
			found = m
		end
	end
	local effort: string? = s.effort
	if found and found.efforts and #found.efforts > 0 then
		local has = false
		for _, l in found.efforts do
			has = has or l.value == effort
		end
		if not has then
			effort = found.effort or found.efforts[1].value
		end
	end
	return { model = value or (if found then found.value else nil), effort = effort }
end
-- (Past the end, as ds/transcript's PAST_END: never `math.huge`, which a
-- measured list takes as its top.)
M.offset = ui.shared("chat.transcript.offset", 1e9)
-- The session the transcript last showed: a reloaded page showing it
-- again keeps its place; another one opens at its latest.
M.shown = ui.shared("chat.transcript.session", nil :: string?)
-- Where in it the reader was (ds/transcript's `keep`), for a page made
-- again; nil at the end.
M.anchor = ui.shared("chat.transcript.anchor", nil :: any)
-- The two pills over a composer, a centred pair: its transcript's "Jump to
-- latest" (`"jump"`) and the composer's folded trays' pill (`"waiting"`),
-- each's width while it shows, nil while it doesn't. Every pane has its
-- own pair, by its key: the chat page's transcript `"page"`, a lane's (a
-- split's pane, a panel tab) `lane.<slot>`, its composer the same; the
-- window's composer `"window"`. Each says only its own width; what stands
-- beside it reads the other's: the window's composer the transcript at its
-- foot (`at_foot`), a split's focused pane the window's composer, any
-- other pane its own composer. (One width for the whole window had every
-- pane's pills reading the one the chat page last said.)
local pill_widths: { [string]: State<number?> } = {}
function M.pill_width_of(kind: "jump" | "waiting", key: string): State<number?>
	local k = `{kind}|{key}`
	local s = pill_widths[k]
	if not s then
		s = ui.shared(`chat.pills.{k}`, nil :: number?)
		pill_widths[k] = s
	end
	return s :: State<number?>
end
-- The pill whose width it says, as one said by one pill alone: nil from a
-- pill made again after another took its key (a chat page made anew, its
-- old transcript going after) clears only its own.
local pill_owner: { [string]: any } = {}
function M.pill_teller(kind: "jump" | "waiting", key: string): (w: number?) -> ()
	local me = {}
	local s = M.pill_width_of(kind, key)
	local k = `{kind}|{key}`
	return function(w: number?)
		if w == nil and pill_owner[k] ~= me then
			return
		end
		pill_owner[k] = if w == nil then nil else me
		if s:peek() ~= w then
			s:set(w)
		end
	end
end
-- The transcript at the window's composer's foot (its pair's key): the
-- chat page's, or in a split the focused pane's (screens/shell/page).
M.at_foot = ui.shared("chat.pills.at_foot", "page")
-- (The chat page's pair, by its old names.)
M.jump_width = M.pill_width_of("jump", "page")
M.pill_width = M.pill_width_of("waiting", "window")
-- Each chat's place as it was left (its `anchor`; none, its end), by
-- session id: switched back to, a chat opens where it was read.
M.places = ui.shared("chat.transcript.places", {} :: { [string]: any })
-- The same across launches: each chat's place as last read, the chat shown
-- with its place as it is read, each with when (os.time), the most recent
-- `KEPT_PLACES` of them. A launch starts from them, so the chat it opens,
-- and any other gone back to, opens where it was read.
local KEPT_PLACES = 200
local saved_places = ui.persist("chat_places", {} :: { [string]: any })
do
	local saved = saved_places:peek()
	if next(M.places:peek()) == nil and type(saved) == "table" and next(saved) ~= nil then
		local all: { [string]: any } = {}
		for id, kept in saved do
			if type(kept) == "table" and type(kept.place) == "table" then
				all[id] = kept.place
			end
		end
		M.places:set(all)
	end
end
ui.effect(function()
	local places = M.places:get()
	local shown = M.shown:get()
	local reading = M.anchor:get()
	local before = saved_places:peek()
	local now = os.time()
	local all: { [string]: any } = {}
	local count = 0
	for id, place in places do
		if id ~= shown then
			local was = if type(before) == "table" then before[id] else nil
			-- (When it was read: kept while its place is as it was.)
			local at = if type(was) == "table" and was.at and was.place and was.place.id == place.id and was.place.dy == place.dy then was.at else now
			all[id] = { place = place, at = at }
			count += 1
		end
	end
	if shown and reading ~= nil then
		all[shown] = { place = reading, at = now }
		count += 1
	end
	-- The most recent kept.
	if count > KEPT_PLACES then
		local order: { { id: string, at: number } } = {}
		for id, kept in all do
			table.insert(order, { id = id, at = tonumber(kept.at) or 0 })
		end
		table.sort(order, function(a: { id: string, at: number }, b: { id: string, at: number }): boolean
			return a.at > b.at
		end)
		for i = KEPT_PLACES + 1, #order do
			all[order[i].id] = nil
		end
	end
	saved_places:set(all)
end)
-- Every other chat's draft, by key (a session's id; "" a new chat's), and
-- the key of the one shown.
local kept = ui.shared("chat.composer.drafts", {} :: { [string]: any })
local here = ui.shared("chat.composer.here", "")

local function cargo_in(key: string): { any }
	if here:peek() == key then
		return M.cargo:peek()
	end
	local d = kept:peek()[key]
	return if d and type(d.cargo) == "table" then d.cargo else {}
end

local function change_kept(key: string, change: (draft: any) -> ())
	local all = table.clone(kept:peek())
	local d = table.clone(all[key] or { text = "", cargo = {}, quotes = {} })
	change(d)
	all[key] = d
	kept:set(all)
end

local function set_cargo_in(key: string, list: { any })
	if here:peek() == key then
		M.cargo:set(list)
		return
	end
	change_kept(key, function(d: any)
		d.cargo = list
	end)
end

local function push_cargo(key: string, items: { any })
	if #items == 0 then
		return
	end
	local list = table.clone(cargo_in(key))
	for _, item in items do
		table.insert(list, item)
	end
	set_cargo_in(key, list)
end
-- Bumped, the composer takes the keys.
M.focus = ui.state("chat_focus", 0)
-- The docked composer's height, as laid out.
M.dock_h = ui.state("chat_dock_h", 52)
-- The chips made so far, for each its own id: kept with the cargo (which
-- a reload keeps), so a chip made after a reload never takes one's id.
local made = ui.shared("chat.composer.chips_made", 0)
local function chip_id(kind: string): string
	local n = made:peek() + 1
	made:set(n)
	return `{kind}{n}`
end

M.refused = ui.shared("chat.composer.refused", nil :: string?)
local numbered = ui.shared("chat.composer.numbered", {} :: { [string]: number })

local MAX_ATTACHMENT = 24 * 1024 * 1024
local PASTE_MIN_CHARS = 5000
local PASTE_HEAD_BYTES = 4 * 1024
local PASTE_LABEL_GRAPHEMES = 20

-- (`ui.call` raises on a host too old to have the action.)
function M.ask(action: string, arg: any): any?
	local ok, answer = pcall(ui.call, action, arg)
	return if ok then answer else nil
end

-- The new-chat page's draft (its words, what it carries, the passages
-- quoted into it), kept across launches as its settings are
-- (screens/chat/composer's `chat_new_defaults`); a chat's isn't.
local saved_new = ui.persist("chat_new_draft", nil :: any)
do
	-- A launch (nothing out yet, nothing put away for a new chat): the new
	-- page's draft as it was left, out as the new chat's (put away under
	-- "" if a chat shows first, as any move does).
	local all = kept:peek()
	local was = saved_new:peek()
	if type(was) == "table" and all[""] == nil and here:peek() == "" and M.text:peek() == "" and #M.cargo:peek() == 0 then
		ui.batch(function()
			M.text:set(if type(was.text) == "string" then was.text else "")
			M.cargo:set(if type(was.cargo) == "table" then was.cargo else {})
			M.quotes:set(if type(was.quotes) == "table" then was.quotes else {})
			-- (Its chips' ids taken: one made from here never repeats one.)
			local top = made:peek()
			for _, c in M.cargo:peek() do
				local n = tonumber(string.match(tostring(c.id or ""), "(%d+)$"))
				if n and n > top then
					top = n
				end
			end
			if top ~= made:peek() then
				made:set(top)
			end
		end)
	end
end
-- Saved as it changes, at most every SAVE_EVERY ms: each save reads the
-- whole draft (into the saved state, then its file), and a long one read
-- at every keystroke cost a copy of it each.
local SAVE_EVERY = 250
local saved_at = -math.huge
local function save_new()
	saved_at = ui.now()
	local now: any = nil
	if here:peek() == "" then
		local text, cargo, quotes = M.text:peek(), M.cargo:peek(), M.quotes:peek()
		if text ~= "" or #cargo > 0 then
			now = { text = text, cargo = cargo, quotes = quotes }
		end
	else
		local put = kept:peek()[""]
		if put then
			now = { text = put.text, cargo = put.cargo, quotes = put.quotes }
		end
	end
	if now ~= nil or saved_new:peek() ~= nil then
		saved_new:set(now)
	end
end
ui.effect(function()
	-- What it's saved from, followed (the draft's edits, not the draft).
	if here:get() == "" then
		M.text:len()
		M.cargo:get()
		M.quotes:get()
	else
		kept:len()
	end
	-- (A run's timer goes as the next run begins: steady typing saves every
	-- SAVE_EVERY, and the last keystroke's once it has waited that long.)
	local wait = SAVE_EVERY - (ui.now() - saved_at)
	if wait <= 0 then
		save_new()
	else
		ui.after(wait, save_new)
	end
end)
-- The window closing (the app quitting, or the window closed): Zeron's
-- window sets `app.closing` (src/main.rs) before it writes what the scripts
-- saved, and a save still waiting its turn goes now. (A timer never fires
-- once the window has closed, and the last keystrokes were lost with it.)
local closing = ui.shared("app.closing", false)
ui.effect(function()
	if closing:get() == true then
		save_new()
	end
end)

-- The other new chats' drafts (`new:<n>`, made by route's `new_chat` beside one written in), kept across launches as "" is.
local function is_new(key: string): boolean
	return key == "" or string.sub(key, 1, 4) == "new:"
end
local function words_in(d: any): boolean
	return type(d) == "table" and ((type(d.text) == "string" and d.text ~= "") or (type(d.cargo) == "table" and #d.cargo > 0))
end
local saved_more = ui.persist("chat_new_drafts", nil :: any)
local minted = ui.persist("chat_new_minted", 0)
do
	local more = saved_more:peek()
	if type(more) == "table" then
		local all = table.clone(kept:peek())
		local top = made:peek()
		for key, d in more do
			if all[key] == nil and here:peek() ~= key and words_in(d) then
				all[key] = d
				for _, c in (if type(d.cargo) == "table" then d.cargo else {}) :: { any } do
					local n = tonumber(string.match(tostring(c.id or ""), "(%d+)$"))
					if n and n > top then
						top = n
					end
				end
			end
		end
		ui.batch(function()
			kept:set(all)
			if top ~= made:peek() then
				made:set(top)
			end
		end)
	end
end
ui.effect(function()
	local out: any = nil
	local h = here:get()
	if h ~= "" and is_new(h) then
		local d = { text = M.text:get(), cargo = M.cargo:get(), quotes = M.quotes:get() }
		if words_in(d) then
			out = { [h] = d }
		end
	end
	for key, d in kept:get() do
		if key ~= "" and key ~= h and is_new(key) and words_in(d) then
			out = out or {}
			out[key] = { text = d.text, cargo = d.cargo, quotes = d.quotes }
		end
	end
	if out ~= nil or saved_more:peek() ~= nil then
		saved_more:set(out)
	end
end)

-- Whether a file's name is a picture the window can draw. (zgx decodes
-- no GIF or HEIC yet: those are file chips, so a thumbnail never fails.)
-- A project's file or folder as Zeron mentions it in a prompt, and back
-- (screens/chat/file_links).
local FileLinks = require("@app/screens/chat/file_links")
M.file_link = FileLinks.link
M.file_of = FileLinks.path_of

local function is_image(name: string): boolean
	local ext = string.match(string.lower(name), "%.(%a+)$")
	return ext == "png" or ext == "jpg" or ext == "jpeg" or ext == "webp"
end

-- The chat shown is now `key` (a session's id, or "" for a new chat): the
-- draft shown is put away under the chat left (nothing kept for an empty
-- one), and `key`'s own is taken out.
-- The chat whose draft is out ("" for a new chat's).
function M.showing(): string
	return here:peek()
end

-- The chat the draft in `text` is (a new chat's ""), followed: it changes
-- with `text` as a move puts one draft away and takes out the next.
function M.owner(): string
	return here:get()
end

-- `showing`, read as it changes.
M.showing_now = M.owner

function M.show(key: string)
	local was = here:peek()
	if was == key then
		return
	end
	ui.batch(function()
		local all = table.clone(kept:peek())
		local text, cargo, quotes, chosen, refused = M.text:peek(), M.cargo:peek(), M.quotes:peek(), M.chosen:peek(), M.refused:peek()
		if text ~= "" or #cargo > 0 or next(chosen) ~= nil or refused ~= nil then
			all[was] = { text = text, cargo = cargo, quotes = quotes, chosen = chosen, refused = refused }
		else
			all[was] = nil
		end
		local now = all[key]
		all[key] = nil
		kept:set(all)
		here:set(key)
		M.text:set(if now then now.text else "")
		M.cargo:set(if now then now.cargo else {})
		M.quotes:set(if now and now.quotes then now.quotes else {})
		M.chosen:set(if now and now.chosen then now.chosen else {})
		M.refused:set(if now then now.refused else nil)
	end)
end

-- (By the list `take` hands out, so `restore` needs no key from its caller.)
local taken: { [any]: { key: string, raw: string, quotes: { [string]: string } } } = setmetatable({}, { __mode = "k" }) :: any

function M.refs_key(of: string?): string
	return `chat:{of or here:peek()}`
end

-- What was typed and carried, taken for sending (the composer cleared).
function M.take(of: string?): (string, { any })
	local key = of or here:peek()
	local mine = here:peek() == key
	local stored = if mine then nil else kept:peek()[key]
	local raw = if mine then M.text:peek() else M.draft_peek(key).text
	local quotes = if mine then M.quotes:peek() elseif stored and stored.quotes then stored.quotes else {}
	local text = references.encode(M.refs_key(key), Quote.expand(raw, Quote.kept(raw, quotes)))
	-- A pasted text whose chip was deleted stays behind, as gpui's does; one
	-- whose chip's map is gone (a reload) is found by its words.
	local cargo = {}
	local carried: { [string]: boolean } = {}
	local lost = {}
	for _, item in cargo_in(key) do
		if item.inline and item.shown then
			table.insert(lost, item)
		end
	end
	-- (Longest first: a twin's words hold its first's.)
	table.sort(lost, function(a: any, b: any): boolean
		return #a.shown > #b.shown
	end)
	for _, item in lost do
		local link = `[{item.name}](zeron-attachment:{item.number})`
		if not string.find(text, link, 1, true) then
			local at, to = M.find_chip(text, item.shown)
			if at and to then
				text = string.sub(text, 1, at - 1) .. link .. string.sub(text, to + 1)
			end
		end
	end
	for _, item in cargo_in(key) do
		local link = `[{item.name}](zeron-attachment:{item.number})`
		if not item.inline or string.find(text, link, 1, true) then
			table.insert(cargo, item)
			if item.number then
				carried[tostring(item.number)] = true
			end
		end
	end
	-- (A paste still being written goes as its name, gpui's `demote_unattached_mentions`.)
	local function demote(name: string, n: string): string?
		return if carried[n] then nil else name
	end
	text = string.gsub(text, "%[([^%]]*)%]%(zeron%-attachment:(%d+)%)", demote)
	text = string.gsub(text, "%[([^%]]*)%]%(zeron%-image:(%d+)%)", demote)
	taken[cargo] = { key = key, raw = raw, quotes = quotes }
	ui.batch(function()
		if mine then
			M.text:set("")
			M.cargo:set({})
			M.quotes:set({})
			M.refused:set(nil)
		else
			change_kept(key, function(d: any)
				d.text, d.cargo, d.quotes, d.refused = "", {}, {}, nil
			end)
		end
		local numbers = table.clone(numbered:peek())
		numbers[key] = nil
		numbered:set(numbers)
	end)
	-- A new chat's draft, sent: saved empty at once, not on the throttle's
	-- next turn, which a quit in between would never reach (the message
	-- coming back as the draft at the next launch).
	if here:peek() == "" then
		save_new()
	end
	return text, cargo
end

-- (`text` only for a list a reload lost track of: `raw` has no folded comments.)
function M.restore(text: string, cargo: { any })
	local from = taken[cargo]
	local key = if from then from.key else here:peek()
	local words = if from then from.raw else text
	local merged = table.clone(cargo)
	local have: { [string]: boolean } = {}
	local top = numbered:peek()[key] or 0
	for _, item in cargo do
		have[item.id] = true
		top = math.max(top, item.number or 0)
	end
	for _, item in cargo_in(key) do
		if not have[item.id] then
			have[item.id] = true
			top = math.max(top, item.number or 0)
			table.insert(merged, item)
		end
	end
	merged = M.renumber(merged)
	local function joined(since: string): string
		return if since == "" or words == "" then words .. since else `{words}\n\n{since}`
	end
	local function quotes_with(since: { [string]: string }?): { [string]: string }
		local all = table.clone(if from then from.quotes else {})
		for line, passage in since or {} do
			all[line] = passage
		end
		return all
	end
	ui.batch(function()
		if top > 0 then
			local numbers = table.clone(numbered:peek())
			numbers[key] = top
			numbered:set(numbers)
		end
		if here:peek() == key then
			M.cargo:set(merged)
			M.quotes:set(quotes_with(M.quotes:peek()))
			M.text:set(joined(M.text:peek()))
		else
			change_kept(key, function(d: any)
				d.cargo = merged
				d.quotes = quotes_with(d.quotes)
				d.text = joined(if type(d.text) == "string" then d.text else "")
			end)
		end
	end)
end

-- Text selected in a transcript or a rendered file, staged on chat `key`'s
-- draft as Zeron's quotes (crates/ui state.rs `add_annotation`): an
-- annotation carries a comment (empty until one is written) and, drawn where
-- it was selected, an anchor for its badge; a quote with no comment is a
-- plain selection. Numbered among the chat's annotations, in order.
local function cargo_get(key: string): { any }
	if here:get() == key then
		return M.cargo:get()
	end
	local d = kept:get()[key]
	return if d and type(d.cargo) == "table" then d.cargo else {}
end

-- For screens/chat/pages, which stages picks of its own.
function M.cargo_of(key: string): { any }
	return cargo_get(key)
end

function M.peek_cargo_of(key: string): { any }
	return cargo_in(key)
end

function M.set_cargo_of(key: string, list: { any })
	set_cargo_in(key, list)
end

function M.add_annotation(key: string, q: { text: string, source: string?, anchor: any?, comment: string? }): string
	local id = chip_id("quote")
	push_cargo(key, { { id = id, kind = "quote", name = Quote.excerpt(q.text), text = q.text, comment = q.comment or "", source = q.source, anchor = q.anchor } })
	return id
end

-- gpui's `set_terminal_quote`: what's selected in the panel's terminal rides the shown chat's next message as a quote of its own, replaced as the selection changes, gone as it clears.
M.TERMINAL_QUOTE = "terminal-selection"
function M.set_terminal_quote(text: string?): boolean
	local wanted = if text and string.find(text, "%S") then text else nil
	local list = M.cargo:peek()
	local current: string? = nil
	for _, item in list do
		if item.id == M.TERMINAL_QUOTE then
			current = item.text
		end
	end
	if current == wanted then
		return false
	end
	local out: { any } = {}
	for _, item in list do
		if item.id ~= M.TERMINAL_QUOTE then
			table.insert(out, item)
		end
	end
	if wanted then
		table.insert(out, { id = M.TERMINAL_QUOTE, kind = "quote", terminal = true, name = Quote.excerpt(wanted), text = wanted })
	end
	M.cargo:set(out)
	return true
end

local function change_quote(key: string, id: string, change: (item: any) -> ())
	local list = table.clone(cargo_in(key))
	for i, item in list do
		if item.id == id then
			local next_item = table.clone(item)
			change(next_item)
			list[i] = next_item
		end
	end
	set_cargo_in(key, list)
end

function M.set_comment_of(key: string, id: string, comment: string)
	change_quote(key, id, function(item: any)
		item.comment = string.match(comment, "^%s*(.-)%s*$") or comment
	end)
end

-- Chat `key`'s annotations as Zeron numbers them (every one, whatever drew
-- it), each `{ n, id, text, comment, source, anchor }`: followed.
function M.annotations_of(key: string): { any }
	local out: { any } = {}
	for _, item in cargo_get(key) do
		if item.kind == "quote" and item.comment ~= nil then
			table.insert(out, { n = #out + 1, id = item.id, text = item.text, comment = item.comment, source = item.source, anchor = item.anchor })
		end
	end
	return out
end

-- A chip's pencil (ds/composer_cargo): the annotation `id` of the chat shown,
-- for the surface that drew it to open its comment again.
M.editing = ui.shared("chat.annotation.edit", nil :: any)

-- What a transcript or a file's preview needs to annotate for chat `key()`
-- (ds/annotation_layer's `Notes`): only what `source` drew (nil: the
-- conversation). `extra.side`: Ask in side chat; `extra.focus`: the
-- composer that takes the keys after a comment (the chat page's by default);
-- `extra.mention`: Mention (by default into chat `key()`'s draft, at the
-- chat page's caret while it shows that chat).
function M.notes(key: () -> string?, source: (string | () -> string?)?, extra: { side: ((text: string) -> ())?, focus: (() -> ())?, mention: ((q: { text: string, who: string? }) -> ())? }?): { [string]: any }
	local x = extra or {}
	local function from(): string?
		return if type(source) == "function" then source() else source
	end
	return {
		list = function(): { any }
			local k = key()
			if not k then
				return {}
			end
			local here = from()
			local out: { any } = {}
			for _, a in M.annotations_of(k) do
				if a.source == here then
					table.insert(out, a)
				end
			end
			return out
		end,
		add = function(text: string, anchor: any): string?
			local k = key()
			return if k then M.add_annotation(k, { text = text, source = from(), anchor = anchor }) else nil
		end,
		comment = function(id: string, text: string, focus: boolean)
			local k = key()
			if k then
				M.set_comment_of(k, id, text)
				if focus then
					if x.focus then
						x.focus()
					else
						M.focus:add(1)
					end
				end
			end
		end,
		remove = function(id: string)
			local k = key()
			if k then
				M.remove_cargo_of(k, id)
			end
		end,
		side = x.side,
		mention = x.mention or function(q: { text: string, who: string? })
			local k = key()
			if k then
				M.mention_in(k, { text = q.text, who = q.who or from() or "" }, if here:peek() == k then composer_surface.field else nil)
			end
		end,
		keys = function()
			local k = key()
			local field = composer_surface.field
			local el = if k and here:peek() == k and field then field:peek() else nil
			if el then
				el:focus()
			elseif x.focus then
				x.focus()
			else
				M.focus:add(1)
			end
		end,
		edit = function(): string?
			local e = M.editing:get()
			return if e then e.id else nil
		end,
		edited = function()
			M.editing:set(nil)
		end,
	}
end

-- The draft shown, lifted out whole to be put back later (a plugin's
-- `zeron.take_draft`): its words as typed, its chips and the passages
-- quoted in it, the composer cleared. Nil when there's nothing to lift.
function M.lift(): { text: string, cargo: { any }, quotes: { [string]: string } }?
	local text, cargo, quotes = M.text:peek(), M.cargo:peek(), M.quotes:peek()
	if text == "" and #cargo == 0 then
		return nil
	end
	ui.batch(function()
		M.text:set("")
		M.cargo:set({})
		M.quotes:set({})
	end)
	return { text = text, cargo = cargo, quotes = quotes }
end

-- A lifted draft put back on the draft shown (`zeron.put_draft`). Onto an
-- empty composer it's as it was; onto words already there, its words follow
-- them after a blank line and its chips follow theirs, its files numbered
-- after those there.
function M.lay(d: { text: string, cargo: { any }, quotes: { [string]: string } })
	local here_text, here_cargo = M.text:peek(), M.cargo:peek()
	ui.batch(function()
		if here_text == "" and #here_cargo == 0 then
			M.text:set(d.text)
			M.cargo:set(d.cargo)
			M.quotes:set(d.quotes)
			return
		end
		local quotes = table.clone(M.quotes:peek())
		for line, passage in d.quotes do
			quotes[line] = passage
		end
		M.quotes:set(quotes)
		if d.text ~= "" then
			M.text:set(if here_text == "" then d.text else `{here_text}\n\n{d.text}`)
		end
		for _, item in d.cargo do
			local moved = table.clone(item)
			moved.mention = nil
			M.push_cargo(nil, moved)
		end
	end)
	M.focus:add(1)
end

-- A passage of the transcript, quoted into the draft (its line marks it)
-- or carried as a chip.
function M.mention(q: Quote.Quote)
	local quotes = Quote.kept(M.text:peek(), M.quotes:peek())
	local line = Quote.line_in(quotes, q)
	quotes[line] = q.text
	ui.batch(function()
		M.quotes:set(quotes)
		M.text:set(Quote.add(M.text:peek(), line))
	end)
	M.focus:add(1)
end

-- A passage quoted into chat `key`'s draft at `field`'s caret (the end
-- without one), its line marking it, then the keys there, the caret after it.
-- `focus`: who takes the keys with no field (the chat page's composer).
function M.mention_in(key: string, q: Quote.Quote, field: any?, focus: (() -> ())?)
	local mine = here:peek() == key
	local text = if mine then M.text:peek() else M.draft_peek(key).text
	local held = if mine then M.quotes:peek() else ((kept:peek()[key] or {}) :: any).quotes or {}
	local line = Quote.line_in(Quote.kept(text, held), q)
	local el = if field then field:peek() else nil
	local at: number? = nil
	if el then
		local a, b = el:selection_bytes()
		if a and b and math.max(a, b) <= #text then
			at = math.max(a, b)
		end
	end
	local out, caret = Quote.insert(text, line, at)
	ui.batch(function()
		if mine then
			local quotes = table.clone(M.quotes:peek())
			quotes[line] = q.text
			M.quotes:set(quotes)
			M.text:set(out)
		else
			change_kept(key, function(d: any)
				local quotes = table.clone(d.quotes or {})
				quotes[line] = q.text
				d.quotes, d.text = quotes, out
			end)
		end
	end)
	if not el then
		if focus then
			focus()
		else
			M.focus:add(1)
		end
		return
	end
	-- (Once the field shows the new words.)
	ui.after(0, function()
		local now = field:peek()
		if now then
			now:focus()
			now:set_selection_bytes(caret, caret)
		end
	end)
end

function M.attach_quote(q: Quote.Quote, key: string?)
	-- The same passage twice (attached again, or its menu and its bar both
	-- taken) is one chip.
	for _, item in M.peek_cargo(key) do
		if item.kind == "annotation" and item.quote and Quote.same({ text = item.quote, who = item.name, from = item.from }, q) then
			if key == nil then
				M.focus:add(1)
			end
			return
		end
	end
	M.push_cargo(key, Quote.chip(q, chip_id("quote")))
	if key == nil then
		M.focus:add(1)
	end
end

-- An element (or a region) picked on a page in the panel's browser
-- (screens/panel/browser): a chip, numbered after the draft's other picks
-- (its "Annotation N", the number the page's mark wears). `extra`: the tab
-- it was picked in and the page's marker. The chip's id and its number.
-- The number the next pick gets: one past the draft's highest.
function M.next_annotation(): number
	local index = 0
	for _, item in M.cargo:peek() do
		if item.kind == "annotation" and item.index then
			index = math.max(index, item.index)
		end
	end
	return index + 1
end

function M.attach_annotation(pick: { [string]: any }, extra: { tab: string?, marker: number? }?): (string, number)
	local index = M.next_annotation()
	local with_index = table.clone(pick)
	with_index.index = index
	local id = chip_id("pick")
	M.cargo:push(Annotation.chip(with_index, id, extra))
	return id, index
end

-- The words typed for a pick (its note field): kept with its chip, sent
-- after its token.
function M.note_cargo(id: string, note: string)
	local list = table.clone(M.cargo:peek())
	for i, item in list do
		if item.id == id then
			local next_item = table.clone(item)
			next_item.note = if note ~= "" then note else nil
			list[i] = next_item
		end
	end
	M.cargo:set(list)
end

-- Chat `key`'s chips, wherever its draft is, for a chat with a composer of
-- its own (a side chat's tab). (`key` nil: the chat shown's.)
function M.peek_cargo(key: string?): { any }
	return if key == nil then M.cargo:peek() else cargo_in(key)
end

function M.push_cargo(key: string?, item: any)
	-- A file or a picture: its number in the draft, the next after any
	-- there (an `@` mention of it names it by that: ds/attachment_mentions).
	if item.path and item.mention == nil then
		local top = 0
		for _, other in M.peek_cargo(key) do
			if type(other.mention) == "number" and other.mention > top then
				top = other.mention
			end
		end
		item.mention = top + 1
	end
	if key == nil then
		M.cargo:push(item)
	else
		push_cargo(key, { item })
	end
end

-- Chat `key`'s chips, taken for sending (none left on its draft).
function M.take_cargo_of(key: string): { any }
	local list = cargo_in(key)
	if #list > 0 then
		set_cargo_in(key, {})
	end
	return list
end

-- Chips of a message that didn't go: back on chat `key`'s draft, before any
-- added since.
function M.restore_cargo_of(key: string, list: { any })
	local all = table.clone(list)
	for _, item in cargo_in(key) do
		table.insert(all, item)
	end
	set_cargo_in(key, all)
end

local function next_number(key: string): number
	local top = numbered:peek()[key] or 0
	for _, item in cargo_in(key) do
		top = math.max(top, item.number or 0)
	end
	-- (A queued message being edited mentions its own, which no chip here numbers.)
	for _, m in AttachmentMentions.mentions(references.encode(M.refs_key(key), M.draft_peek(key).text)) do
		top = math.max(top, m.index)
	end
	return top + 1
end

local function note_number(key: string, number: number)
	local numbers = table.clone(numbered:peek())
	numbers[key] = number
	numbered:set(numbers)
end

function M.refuse_in(key: string, why: string?)
	if here:peek() == key then
		M.refused:set(why)
	else
		change_kept(key, function(d: any)
			d.refused = why
		end)
	end
end
local refuse_in = M.refuse_in

-- gpui's `linked_elsewhere_message`: a file over the limit goes by its path,
-- which only this computer can read.
function M.linked_elsewhere(cargo: { any }): string?
	for _, item in cargo do
		if item.linked then
			return locale.s("chat.composer.linked_elsewhere", { name = item.name })
		end
	end
	return nil
end

function M.size_label(bytes: number): string
	local mb = bytes / (1024 * 1024)
	return if mb >= 1024 then string.format("%.1f GB", mb / 1024) else string.format("%.0f MB", mb)
end

local function stage_files(key: string, base: number, files: { any }, stats: { [string]: any }?)
	local items: { any } = {}
	local refused: { string } = {}
	for _, f in files do
		local name = f.name or f.path or "file"
		local stat = if stats and f.path then stats[f.path] else nil
		if stat and not stat.readable then
			table.insert(refused, locale.s("chat.composer.file_unreadable", { name = name }))
		elseif stat and not stat.is_file then
			table.insert(refused, locale.s("chat.composer.not_a_file", { name = name }))
		else
			local number = base + #items
			if stat and stat.size > MAX_ATTACHMENT then
				table.insert(items, {
					id = chip_id("file"),
					kind = "file",
					name = name,
					label = `{name} · {M.size_label(stat.size)}`,
					path = f.path,
					on_host = true,
					linked = true,
					bytes = stat.size,
					number = number,
				})
			else
				local picture = is_image(name)
				local ext = if picture then string.match(string.lower(name), "%.(%a+)$") else nil
				local escaped = string.gsub(name, "([%[%]\\])", "\\%1")
				local link = if picture then `[Image {number}](zeron-image:{number})` else `[{escaped}](zeron-attachment:{number})`
				table.insert(items, {
					id = chip_id("file"),
					kind = if picture then "image" else "file",
					name = if picture then `Image {number}.{ext}` else name,
					src = if picture then f.path else nil,
					path = f.path,
					number = number,
					shown = references.add(M.refs_key(key), { kind = if picture then "image" else "file", name = if picture then `Image {number}` else name, link = link }),
				})
			end
		end
	end
	local refs = M.refs_key(key)
	for _, item in items do
		if item.kind == "image" and item.shown then
			local asked = M.ask("chat.thumb", item.path)
			if asked then
				asked:after(function(svg: any)
					if type(svg) == "string" then
						references.set_thumb(refs, item.shown, "code:" .. svg)
					end
				end, function() end)
			end
		end
	end
	ui.batch(function()
		local words: { string } = {}
		for _, item in items do
			if item.shown then
				table.insert(words, item.shown)
			end
		end
		if #words > 0 then
			M.put_words(key, table.concat(words, " "), true)
		end
		push_cargo(key, items)
		if #refused > 0 then
			refuse_in(key, table.concat(refused, " "))
		end
	end)
end

function M.attach_files(files: { any }, of: string?)
	local key = of or here:peek()
	refuse_in(key, nil)
	-- (Taken now: a second attach can come before the host answers the first.)
	local base = next_number(key)
	note_number(key, base + #files - 1)
	local paths: { string } = {}
	for _, f in files do
		if f.path then
			table.insert(paths, f.path)
		end
	end
	local asked = if #paths > 0 then M.ask("chat.stat_files", { paths = paths }) else nil
	if not asked then
		stage_files(key, base, files, nil)
		return
	end
	asked:after(function(list: any)
		local stats: { [string]: any } = {}
		for _, stat in list or {} do
			stats[stat.path] = stat
		end
		stage_files(key, base, files, stats)
	end, function()
		stage_files(key, base, files, nil)
	end)
end

-- (`$` too: this composer writes a skill so. Four spaces or a tab is code.)
local function leads_with_command(draft: string): boolean
	local indent, name = string.match(draft, "^([ \t\r\n]*)[/$](%S+)")
	if not indent or not name or not string.find(name, "^[%w%-_:.\128-\255]+$") then
		return false
	end
	local last = string.match(indent, "[^\r\n]*$") or ""
	return #last < 4 and not string.find(last, "\t", 1, true)
end

local function pastes_as_file(text: string, draft: string): boolean
	local bytes = #text
	if bytes < PASTE_MIN_CHARS then
		return false
	end
	-- (A character is 1 to 4 bytes: only a length that could fall either way is counted.)
	if bytes < PASTE_MIN_CHARS * 4 and (utf8.len(text) or bytes) < PASTE_MIN_CHARS then
		return false
	end
	if leads_with_command(draft) then
		return false
	end
	-- (A plain find: a pattern over 24 MiB outruns a script call's budget.)
	if bytes > MAX_ATTACHMENT then
		return true
	end
	local at = 1
	while true do
		local from = string.find(text, "](zeron-", at, true)
		if not from then
			return true
		end
		local scheme = string.match(text, "^(%a+):%d+%)", from + 8)
		if scheme == "attachment" or scheme == "image" then
			return false
		end
		at = from + 8
	end
end

-- (zgx has no grapheme call for scripts: marks, joiners, skin tones and tags.)
local EXTENDING = {
	{ 0x0300, 0x036F },
	{ 0x0483, 0x0489 },
	{ 0x0591, 0x05BD },
	{ 0x0610, 0x061A },
	{ 0x064B, 0x065F },
	{ 0x0E31, 0x0E31 },
	{ 0x0E34, 0x0E3A },
	{ 0x0E47, 0x0E4E },
	{ 0x1AB0, 0x1AFF },
	{ 0x1DC0, 0x1DFF },
	{ 0x200D, 0x200D },
	{ 0x20D0, 0x20FF },
	{ 0xFE00, 0xFE0F },
	{ 0xFE20, 0xFE2F },
	{ 0x1F3FB, 0x1F3FF },
	{ 0xE0020, 0xE007F },
	{ 0xE0100, 0xE01EF },
}
local function extends(cp: number): boolean
	for _, range in EXTENDING do
		if cp >= range[1] and cp <= range[2] then
			return true
		end
	end
	return false
end

function M.paste_label(text: string): string?
	local head = string.sub(text, 1, PASTE_HEAD_BYTES)
	local valid, bad = utf8.len(head)
	if not valid then
		head = string.sub(head, 1, (bad :: number) - 1)
	end
	head = string.gsub(head, "\226\128[\168\169]", "\n")
	for raw in string.gmatch(head, "[^\r\n]+") do
		local line = string.gsub(raw, "%c", " ")
		line = string.match(line, "^%s*(.-)%s*$") or ""
		if line ~= "" then
			local count, joined, flags = 0, false, 0
			for pos, cp in utf8.codes(line) do
				local flag = cp >= 0x1F1E6 and cp <= 0x1F1FF
				if not (joined or extends(cp) or (flag and flags % 2 == 1)) then
					count += 1
					if count > PASTE_LABEL_GRAPHEMES then
						return (string.match(string.sub(line, 1, pos - 1), "^(.-)%s*$") or "") .. "…"
					end
				end
				joined = cp == 0x200D
				flags = if flag then flags + 1 else 0
			end
			return line
		end
	end
	return nil
end

M.find_chip = references.find_chip

-- A long paste, as gpui's composer takes it: a file of its own, carried
-- with the message, and a chip at the caret standing for it (its first
-- line), sent as `[Pasted text N.txt](zeron-attachment:N)`. `insert` puts
-- the chip at the caret; without it the chip ends the draft.
function M.paste_text(text: string, of: string?, insert: ((shown: string) -> boolean)?): boolean
	local key = of or here:peek()
	if not pastes_as_file(text, M.draft_peek(key).text) then
		return false
	end
	if #text > MAX_ATTACHMENT then
		refuse_in(key, locale.s("chat.composer.paste_too_large"))
		return true
	end
	local number = next_number(key)
	local name = `Pasted text {number}.txt`
	local asked = M.ask("chat.paste_text", { text = text, name = name })
	if not asked then
		return false
	end
	refuse_in(key, nil)
	note_number(key, number)
	local inserted = false
	local function put(words: string)
		inserted = insert ~= nil and insert(words)
		if not inserted then
			M.set_draft_of(key, "text", M.draft_peek(key).text .. words)
		end
	end
	local label = M.paste_label(text)
	local shown: string? = nil
	local written: string? = nil
	local failed = false
	local bytes = #text
	-- (Landing after the draft was sent or the chip deleted: nothing to carry.)
	local function landed(path: string)
		if M.find_chip(M.draft_peek(key).text, shown :: string) then
			push_cargo(key, { { id = chip_id("paste"), kind = "file", name = name, label = label, path = path, pasted = true, inline = true, shown = shown, bytes = bytes, number = number } })
		end
	end
	-- (Not written: the words themselves where the chip stands, if it still does.)
	local function unwritten()
		failed = true
		if not shown then
			return
		end
		local draft = M.draft_peek(key).text
		local at, to = M.find_chip(draft, shown)
		if at and to then
			M.set_draft_of(key, "text", string.sub(draft, 1, at - 1) .. text .. string.sub(draft, to + 1))
		end
	end
	asked:after(function(path: any)
		if type(path) ~= "string" or path == "" then
			unwritten()
		elseif shown then
			landed(path)
		else
			written = path
		end
	end, unwritten)
	if failed then
		put(text)
	else
		shown = references.add(M.refs_key(key), { kind = "pasted", name = label or name, link = `[{name}](zeron-attachment:{number})` })
		put(shown :: string)
		if written then
			landed(written)
		end
	end
	if not of and not inserted then
		M.focus:add(1)
	end
	return true
end

function M.open_pasted(id: string, insert: ((text: string) -> boolean)?, of: string?)
	local key = of or here:peek()
	local item: any = nil
	for _, it in cargo_in(key) do
		if it.id == id then
			item = it
		end
	end
	if not item or not item.path then
		return
	end
	local function unreadable()
		refuse_in(key, locale.s("chat.composer.file_unreadable", { name = item.name }))
	end
	local asked = M.ask("chat.read_text", { path = item.path })
	if not asked then
		unreadable()
		return
	end
	asked:after(function(read: any)
		if type(read) ~= "table" or type(read.text) ~= "string" then
			unreadable()
			return
		end
		-- (Pressed twice: the first read took it.)
		local still = false
		for _, it in cargo_in(key) do
			still = still or it.id == id
		end
		if not still then
			return
		end
		ui.batch(function()
			M.remove_cargo_of(key, id)
			local draft = M.draft_peek(key).text
			local at, to = nil, nil
			if item.shown then
				at, to = M.find_chip(draft, item.shown)
			end
			if at and to then
				M.set_draft_of(key, "text", string.sub(draft, 1, at - 1) .. read.text .. string.sub(draft, to + 1))
			elseif not ((of or here:peek() == key) and insert and insert(read.text)) then
				M.set_draft_of(key, "text", draft .. read.text)
			end
		end)
		if not of then
			M.focus:add(1)
		end
	end, unreadable)
end

-- A composer's chips for chat `of` (the window's draft when nil): what's
-- pasted, picked and dropped. `dropped` runs before a drop is taken.
function M.cargo_props(of: string?, dropped: (() -> ())?): { [string]: any }
	return {
		on_paste_text = function(text: string, insert: ((shown: string) -> boolean)?): boolean
			return M.paste_text(text, of, insert)
		end,
		on_open_pasted = function(id: string, insert: ((text: string) -> boolean)?)
			M.open_pasted(id, insert, of)
		end,
		on_attach = function()
			ui.open_files({ title = ui.t("chat.composer.attach_files"):get(), multiple = true }):after(function(files: any)
				if files and #files > 0 then
					M.attach_files(files, of)
				end
			end, function() end)
		end,
		on_files = function(files: { any })
			if dropped then
				dropped()
			end
			M.attach_files(files, of)
		end,
		-- A picture pasted (a screenshot): written out by the host, then
		-- carried as an image, as one dropped is, named as Zeron names
		-- it (attachments.rs `stage_clipboard_image`).
		on_paste_image = function(): boolean
			(require("@app/screens/chat/store") :: any).paste_image():after(function(path: string?)
				if type(path) == "string" and path ~= "" then
					M.attach_files({ { name = "image.png", path = path, clipboard = true } }, of)
				end
			end, function() end)
			return true
		end,
	}
end

-- Files of the project (the files pane's, dragged here or its "Add to
-- chat"): chips, carried with the message as files from outside are. Each
-- is already on the session's computer (its project is), so it goes by its
-- path there, not uploaded again; `paths` are the project's own (relative
-- to `root`, the project's folder there), and a chip says where in it the
-- file is. One already carried isn't added twice.
function M.add_to_chat(paths: { string }, root: string?, key: string?)
	local have: { [string]: boolean } = {}
	for _, item in M.peek_cargo(key) do
		if item.path then
			have[item.path] = true
		end
	end
	-- On this computer (the project whose folder it is lives here): a
	-- picture shows as one, its thumbnail read from where it is. On
	-- another, a file chip.
	local here = false
	local now = sessions.now:peek()
	for _, p in now.projects do
		if p.path == root and p.device == now.here then
			here = true
		end
	end
	for _, rel in paths do
		local at = if root and root ~= "" then `{root}/{rel}` else rel
		if not have[at] then
			have[at] = true
			local name = string.match(rel, "([^/]+)/?$") or rel
			local picture = here and is_image(name)
			M.push_cargo(key, {
				id = chip_id("file"),
				kind = if picture then "image" else "file",
				name = name,
				src = if picture then at else nil,
				-- (Where in the project, under its name.)
				meta = string.match(rel, "^(.*)/[^/]+$"),
				path = at,
				rel = rel,
				on_host = true,
			})
		end
	end
	if key == nil then
		M.focus:add(1)
	end
end

-- The composer's model picker, opened from elsewhere (the shell's ⌘/):
-- bumped, it opens. `open_model_picker` says whether there was a composer
-- to open it in. `composer_shown` counts the composers mounted now
-- (screens/chat/composer: a swapped one mounts before the old one goes):
-- `composer_mounted`/`composer_gone` change it. A template's composer
-- written before it counted sets true or false, read as one or none (its
-- `false` on cleanup clears the count whole: a limit of mixing the two,
-- only while such a composer and a counted one swap).
M.open_model = ui.shared("chat.composer.open_model", 0)
-- A new chat's agent asked for from elsewhere (the Skills page's Try now): an agent's id; the composer takes its first model unless it runs that agent already, then clears it.
M.want_agent = ui.shared("chat.composer.want_agent", nil :: string?)
M.composer_shown = ui.shared("chat.composer.shown", 0)
M.said = ui.shared("chat.composer.said", { n = 0, text = "" })
function M.say(text: string)
	M.said:set { n = M.said:peek().n + 1, text = text }
end
local function composers(): number
	local n: any = M.composer_shown:peek()
	return if type(n) == "number" then n elseif n then 1 else 0
end
function M.composer_mounted()
	M.composer_shown:set(composers() + 1)
end
function M.composer_gone()
	M.composer_shown:set(math.max(0, composers() - 1))
end
function M.open_model_picker(): boolean
	if composers() <= 0 then
		return false
	end
	M.open_model:set(M.open_model:peek() + 1)
	return true
end

-- ⌘− ⌘=: the composer's effort a stop down or up. Whether there was a composer to step.
M.step_effort = ui.shared("chat.composer.step_effort", { n = 0, by = 0 })
function M.step_effort_by(by: number): boolean
	if M.composer_shown:peek() <= 0 then
		return false
	end
	M.step_effort:set({ n = M.step_effort:peek().n + 1, by = by })
	return true
end

-- Files carried over the chat (from outside, or the files pane), by the
-- zone they're over: the session page, a new chat's page, the composer's
-- pill. While any is, the whole chat shows "Drop to attach"; a drop on any
-- attaches to the composer.
M.dropping = ui.shared("chat.dropping", {} :: { [string]: boolean })
-- The same over a chat with a composer of its own (a side chat's tab, in the
-- panel): each zone its own overlay and its own draft, the chat page none the
-- wiser.
M.dropping_own = ui.shared("chat.dropping_own", {} :: { [string]: boolean | string })

-- (`what`: "session" while a sidebar session is carried, "tab" while a tab
-- is, either's drop a mention; else files.)
local function over_in(map: any, zone: string, inside: boolean, what: string?)
	local now = map:peek()
	local want = if inside then (what or true) else nil
	if now[zone] == want then
		return
	end
	local next = table.clone(now)
	next[zone] = want
	map:set(next)
end

function M.over(zone: string, inside: boolean, what: string?)
	over_in(M.dropping, zone, inside, what)
end

local function any_files(map: any): boolean
	for _, what in map:get() do
		if what == true then
			return true
		end
	end
	return false
end

-- Whether files are being carried over the chat ("Drop to attach").
function M.carrying(): boolean
	return any_files(M.dropping)
end

-- Whether a sidebar session is being carried over it ("Drop to mention").
function M.carrying_session(): boolean
	for _, what in M.dropping:get() do
		if what == "session" then
			return true
		end
	end
	return false
end

function M.mentioning_over(zone: string): boolean
	local what = M.dropping:get()[zone]
	return what == "session" or what == "tab"
end

-- Files carried from outside end with a drop or with their leave. One lost
-- on the way (the platform's drag ended elsewhere, or what the files were
-- over went first) left the chat dimmed under "Drop to attach" for good,
-- Escape and all. Any sign the drag is over lets go of them: Escape, the
-- window losing focus, the pointer over the chat (none reaches a window
-- while another app's drag is over it), or no word of them for a while. A
-- sidebar session or a tab carried is the window's own drag, whose end is
-- always heard: kept. True if files were let go of.
local function let_go_in(map: any): boolean
	local next, any = {}, false
	for zone, what in map:peek() do
		if what ~= true then
			next[zone] = what
		else
			any = true
		end
	end
	if any then
		map:set(next)
	end
	return any
end

function M.let_go(): boolean
	local page = let_go_in(M.dropping)
	local own = let_go_in(M.dropping_own)
	return page or own
end

-- No word of them for this long (each move from one part of the chat to
-- another is one): let go. The pointer's word counts only once they've
-- been over the chat a moment: a move it made just before the drag came
-- can be heard after it.
local STALE_MS = 8000
local SETTLE_MS = 300
local settled = false
ui.effect(function()
	settled = false
	if any_files(M.dropping) or any_files(M.dropping_own) then
		ui.after(SETTLE_MS, function()
			settled = true
		end)
		ui.after(STALE_MS, function()
			M.let_go()
		end)
	end
end)
ui.effect(function()
	if ui.window.active:get() == false then
		M.let_go()
	end
end)

-- The pointer over the chat again: no drag from outside is under way.
function M.pointer_back()
	if settled then
		M.let_go()
	end
end

-- Another chat mentioned (a sidebar session dropped on the chat): a chip,
-- sent as Zeron's chat mention (screens/chat/chat_links). Once each.
function M.mention_chat(id: string, key: string?)
	if not ChatLinks.valid(id) then
		return
	end
	local mine = key or here:peek()
	if id == mine then
		M.refuse_in(mine, locale.s("chat.composer.self_mention"))
		return
	end
	M.refuse_in(mine, nil)
	for _, item in M.peek_cargo(key) do
		if item.kind == "chat" and item.chat == id then
			return
		end
	end
	local title, mark = nil :: string?, nil :: string?
	for _, c in sessions.now:peek().chats do
		if c.id == id then
			title, mark = c.title, c.agent
		end
	end
	if title == nil then
		for _, list in sessions.now:peek().children or {} do
			for _, c in list do
				if c.id == id then
					title, mark = c.title, c.agent
				end
			end
		end
	end
	M.push_cargo(key, { id = chip_id("chat"), kind = "chat", name = ChatLinks.title(title), chat = id, mark = mark })
end

-- A chat as it's listed now, for what says who sent a message: its title (its
-- short id, untitled) and its agent's mark. Nil for one that isn't listed.
function M.chat_of(id: string): { title: string, mark: string? }?
	local now = sessions.now:get()
	local found: any = nil
	for _, c in now.chats do
		if c.id == id then
			found = c
			break
		end
	end
	if not found then
		for _, c in now.archived or {} do
			if c.id == id then
				found = c
				break
			end
		end
	end
	if not found then
		for _, list in now.children or {} do
			for _, c in list do
				if c.id == id then
					found = c
					break
				end
			end
			if found then
				break
			end
		end
	end
	if not found then
		return nil
	end
	local title = string.match(string.gsub(found.title or "", "%s+", " "), "^%s*(.-)%s*$") or ""
	return { title = if title ~= "" then title else string.sub(id, 1, 8), mark = found.agent }
end

local function listed(id: string): any
	local now = sessions.now:get()
	for _, c in now.chats do
		if c.id == id then
			return c
		end
	end
	for _, list in now.children or {} do
		for _, c in list do
			if c.id == id then
				return c
			end
		end
	end
	for _, c in now.archived or {} do
		if c.id == id then
			return c
		end
	end
	return nil
end

-- Zeron's `resolve_chat_key`, as the MCP server reads a message's recipient.
function M.resolve_chat(key: string): { id: string, title: string, mark: string? }?
	key = string.match(key or "", "^%s*(.-)%s*$") or ""
	if key == "" then
		return nil
	end
	local function said(c: any): { id: string, title: string, mark: string? }
		local title = string.match(string.gsub(c.title or "", "%s+", " "), "^%s*(.-)%s*$") or ""
		return { id = c.id, title = if title ~= "" then title else string.sub(c.id, 1, 8), mark = c.mark or c.agent }
	end
	local exact = listed(key)
	if exact then
		return said(exact)
	end
	local now = sessions.now:get()
	local all: { any } = table.clone(now.chats)
	for _, list in now.children or {} do
		for _, c in list do
			table.insert(all, c)
		end
	end
	local by_prefix, by_title = {}, {}
	local lower = string.lower(key)
	for _, c in all do
		if string.sub(c.id, 1, #key) == key then
			table.insert(by_prefix, c)
		end
		if string.lower(string.match(c.title or "", "^%s*(.-)%s*$") or "") == lower then
			table.insert(by_title, c)
		end
	end
	if #by_prefix == 1 then
		return said(by_prefix[1])
	end
	-- (An ambiguous prefix names nothing: the server refuses it.)
	if #by_prefix == 0 and #by_title == 1 then
		return said(by_title[1])
	end
	return nil
end

-- `home()`: the chat whose panel its children open in.
function M.threads_for(session: () -> string?, home: () -> string?, live: () -> any): any
	local lanes = require("@app/screens/chat/lanes") :: any
	return {
		child = function(id: string): any
			return listed(id)
		end,
		chat = M.resolve_chat,
		limit = function(id: string): any
			local s = live()
			if not s or s.id ~= session() then
				return nil
			end
			for _, l in s.limits or {} do
				if l.chat == id then
					return l
				end
			end
			return nil
		end,
		is_open = function(id: string): boolean
			return lanes.is_open(home(), id)
		end,
		open_child = function(id: string)
			local h = home()
			if h then
				lanes.open_child(h, id)
			end
		end,
		open_chat = function(id: string)
			lanes.open_thread(id)
		end,
		limit_op = function(id: string, op: string)
			local answer = require("@app/screens/chat/store").limit(id, op)
			if answer then
				answer:after(function() end, function() end)
			end
		end,
		agent = function(): any
			local s = live()
			if not s or s.id ~= session() then
				return nil
			end
			local row = listed(s.id)
			local Threads = require("@ds/transcript_threads") :: any
			return { label = Threads.harness_name(s.harness), mark = if row then row.mark or row.agent else nil, model = s.model, harness = s.harness }
		end,
		model_label = function(harness: string?, model: string): string?
			return (require("@app/screens/chat/store") :: any).model_label(harness, model)
		end,
		plan = function(): any
			return (require("@app/screens/chat/threads") :: any).card(session())
		end,
		approve_plan = function()
			(require("@app/screens/chat/threads") :: any).approve(session())
		end,
		edit_plan = function()
			(require("@app/screens/chat/threads") :: any).edit(session())
		end,
	}
end

-- A chat chip's text as it goes.
function M.chat_link(title: string, id: string): string
	return ChatLinks.link(title, id) or ""
end

-- A drag from inside the window that drops here: the files pane's files
-- (their paths, also carried by file tabs). Other drags aren't file context.
local function workspace_paths(payload: any): { string }?
	if type(payload) == "table" and type(payload.paths) == "table" and #payload.paths > 0 then
		return payload.paths
	end
	return nil
end

-- The props that make a node a drop zone, named `zone` in `map`: files from
-- outside, and the files pane's, attach as chips on chat `key`'s draft (nil:
-- the chat shown's); a sidebar session is mentioned there. `landed` runs as
-- anything does.
local function zone_props(map: any, zone: string, key: string?, landed: (() -> ())?): { [string]: any }
	local function land()
		map:set({})
		if landed then
			landed()
		end
	end
	return {
		on_file_hover = function(inside: boolean)
			over_in(map, zone, inside)
		end,
		on_file_drop = function(files: { any })
			M.attach_files(files, key)
			land()
		end,
		drop_when = function(payload: any): boolean
			-- (Files, and a sidebar session: a mention.)
			return workspace_paths(payload) ~= nil or (type(payload) == "table" and type(payload.session) == "string")
		end,
		on_drag_over = function(payload: any)
			local chat = type(payload) == "table" and type(payload.session) == "string"
			over_in(map, zone, true, if chat and payload.session_tab ~= nil then "tab" elseif chat then "session" else nil)
		end,
		on_drag_leave = function()
			over_in(map, zone, false)
		end,
		on_hover = function(inside: boolean)
			if inside then
				M.pointer_back()
			end
		end,
		on_drop = function(payload: any)
			local paths = workspace_paths(payload)
			if paths then
				M.add_to_chat(paths, payload.root, key)
			elseif type(payload) == "table" and type(payload.session) == "string" then
				M.mention_chat(payload.session, key)
			end
			land()
		end,
	}
end

-- A drop zone of the chat page, named `zone`: what lands attaches to the
-- window's composer.
function M.drop_zone(zone: string): { [string]: any }
	return zone_props(M.dropping, zone, nil)
end

-- A drop zone of chat `key`, whose composer is its own (a side chat's tab):
-- its props, and whether files, and a sidebar session, are over it now, for
-- its own overlays. `landed` runs as anything lands (its composer takes the
-- keys).
function M.own_drop_zone(zone: string, key: string, landed: () -> ()): ({ [string]: any }, () -> boolean, () -> boolean)
	return zone_props(M.dropping_own, zone, key, landed),
		function(): boolean
			return M.dropping_own:get()[zone] == true
		end,
		function(): boolean
			return M.dropping_own:get()[zone] == "session"
		end
end

-- A review comment left on a project's file (the files pane's editor or
-- its preview): a chip on the composer, sent as the review comments block
-- after the words (screens/chat/comments). `file` is the project's path
-- (relative to its folder), `line` the line it's on. Its id comes back, to
-- change it (`update_comment`) or take it off (`remove_cargo`). On the
-- cargo it's an item of kind "comment" with `file`, `line`, `body` (and,
-- from the files pane, its `path` and the project's `root` too: given as
-- `{ path, line, body, root }`, `file` is the path within the root).
-- On a diff (a plugin's, `zeron.compose_comment`): `side` ("L" or "R"), a
-- range's last line `to`, the code it's on `snippet`; `plugin` and `key`
-- say whose it is (published back to it: screens/plugins), and `session`
-- the chat whose draft takes it (the one shown, else its draft kept).
function M.attach_comment(c: {
	file: string?,
	path: string?,
	line: number,
	body: string,
	root: string?,
	to: number?,
	side: string?,
	snippet: string?,
	plugin: string?,
	key: string?,
	session: string?,
}): string
	local target = c.session or here:peek()
	if c.plugin and c.key then
		-- (A plugin's comment asked again under its key: an edit, its chip kept.)
		local list = cargo_in(target)
		for i, item in list do
			if item.kind == "comment" and item.plugin == c.plugin and item.key == c.key then
				local next = table.clone(list)
				local one = table.clone(item)
				one.body = c.body
				one.meta = (string.gsub(c.body, "%s+", " "))
				next[i] = one
				set_cargo_in(target, next)
				return item.id
			end
		end
	end
	local id = chip_id("comment")
	local file = c.file or c.path or ""
	local root = c.root
	if not c.file and root and root ~= "" and string.sub(file, 1, #root + 1) == root .. "/" then
		file = string.sub(file, #root + 2)
	end
	local name = string.match(file, "([^/]+)$") or file
	local to = if c.to and c.to > c.line then c.to else nil
	local item = {
		id = id,
		kind = "comment",
		name = if to then `{name}:{c.line}–{to}` else `{name}:{c.line}`,
		meta = (string.gsub(c.body, "%s+", " ")),
		file = file,
		path = c.path or file,
		root = root,
		line = c.line,
		to = to,
		side = if c.side == "L" or c.side == "R" then c.side else nil,
		snippet = c.snippet,
		body = c.body,
		plugin = c.plugin,
		key = c.key,
	}
	push_cargo(target, { item })
	return id
end

-- Every comment a plugin added still carried, in the draft shown and the
-- drafts kept: by plugin, its key → the chip's id, its session and its note.
function M.plugin_comments(): { [string]: { [string]: { id: string, session: string, body: string } } }
	local out: { [string]: { [string]: { id: string, session: string, body: string } } } = {}
	local function take(cargo: { any }?, session: string)
		for _, item in cargo or {} do
			if item.kind == "comment" and type(item.plugin) == "string" and type(item.key) == "string" then
				out[item.plugin] = out[item.plugin] or {}
				out[item.plugin][item.key] = { id = item.id, session = session, body = item.body or "" }
			end
		end
	end
	take(M.cargo:get(), here:get())
	for session, draft in kept:get() do
		take(draft.cargo, session)
	end
	return out
end

-- A plugin's comment taken off by the plugin (its card's Remove), wherever
-- its draft is.
function M.remove_plugin_comment(plugin: string, key: string)
	local function take(cargo: { any }?, session: string): boolean
		for _, item in cargo or {} do
			if item.kind == "comment" and item.plugin == plugin and item.key == key then
				M.remove_cargo_of(session, item.id)
				return true
			end
		end
		return false
	end
	if take(M.cargo:peek(), here:peek()) then
		return
	end
	for session, draft in kept:peek() do
		if take(draft.cargo, session) then
			return
		end
	end
end

-- A comment carried to another line (the edits above it moved it): its
-- chip says the new line.
function M.move_comment(id: string, line: number)
	local list = M.cargo:peek()
	for i, item in list do
		if item.id == id and item.kind == "comment" then
			local next = table.clone(item)
			next.line = line
			local name = string.match(item.file or "", "([^/]+)$") or item.file or ""
			next.name = `{name}:{line}`
			M.cargo:set_key({ i }, next)
			return
		end
	end
end

-- A comment's words changed (its chip follows).
function M.update_comment(id: string, body: string)
	local list = M.cargo:peek()
	for i, item in list do
		if item.id == id and item.kind == "comment" then
			local next = table.clone(item)
			next.body = body
			next.meta = (string.gsub(body, "%s+", " "))
			M.cargo:set_key({ i }, next)
			return
		end
	end
end

-- A pasted text's chip leaves the words with it (gpui's `remove_attachment_chips`).
local function without_chip(text: string, item: any): string
	local at, to = nil, nil
	if item.shown then
		at, to = M.find_chip(text, item.shown)
	end
	return if at and to then string.sub(text, 1, at - 1) .. string.sub(text, to + 1) else text
end

function M.put_words(key: string, words: string, focus: boolean?)
	local refs = M.refs_key(key)
	local text = M.draft_peek(key).text
	local el = references.field(refs)
	local caret = references.caret(refs)
	local from: number? = nil
	if el then
		local a, b = el:selection_bytes()
		if a and b then
			from, caret = math.min(a, b), math.max(a, b)
		end
	end
	local at = math.clamp(caret or #text, 0, #text)
	for _, span in references.spans(refs, text) do
		if at > span.from - 1 and at < span.to then
			at = span.to
		end
	end
	while at < #text and (string.byte(text, at + 1) or 0) >= 0x80 and (string.byte(text, at + 1) or 0) < 0xC0 do
		at += 1
	end
	local start = if from and from ~= caret then math.clamp(from, 0, at) else at
	local tail = if at == #text then " " else ""
	local lead = if start > 0 and not string.find(string.sub(text, start, start), "%s") then " " else ""
	local put = lead .. words .. tail
	if el and el:set_selection_bytes(start, at) and el:replace_selection(put) then
		if focus then
			el:focus()
		end
		return
	end
	M.set_draft_of(key, "text", string.sub(text, 1, start) .. put .. string.sub(text, at + 1))
	references.caret(refs, start + #put)
	if focus and key == here:peek() then
		M.focus:add(1)
	end
end

function M.remove_cargo(id: string)
	local left = {}
	for _, item in M.cargo:peek() do
		if item.id ~= id then
			table.insert(left, item)
		else
			M.text:set(without_chip(M.text:peek(), item))
		end
	end
	M.cargo:set(M.renumber(left))
end

-- Chat `key`'s draft, wherever it is: out in the composer while it shows
-- that chat, else kept for it: its words, its chips and the options picked
-- in a question it's asked. What shows a chat's draft by its id (a split
-- pane's composer) reads it here, never the chat shown's; `show` moves a
-- draft between the two in one batch, so it's never read half moved.
function M.draft_of(key: string): { text: string, cargo: { any }, chosen: { [string]: boolean }, refused: string? }
	if here:get() == key then
		return { text = M.text:get(), cargo = M.cargo:get(), chosen = M.chosen:get(), refused = M.refused:get() }
	end
	local d = kept:get()[key]
	return {
		text = if d and type(d.text) == "string" then d.text else "",
		cargo = if d and type(d.cargo) == "table" then d.cargo else {},
		chosen = if d and type(d.chosen) == "table" then d.chosen else {},
		refused = if d then d.refused else nil,
	}
end

function M.draft_peek(key: string): { text: string, cargo: { any } }
	if here:peek() == key then
		return { text = M.text:peek(), cargo = M.cargo:peek() }
	end
	local d = kept:peek()[key]
	return {
		text = if d and type(d.text) == "string" then d.text else "",
		cargo = if d and type(d.cargo) == "table" then d.cargo else {},
	}
end

function M.discard_draft_of(key: string)
	if numbered:peek()[key] then
		local numbers = table.clone(numbered:peek())
		numbers[key] = nil
		numbered:set(numbers)
	end
	if here:peek() == key then
		M.text:set("")
		M.cargo:set({})
		M.quotes:set({})
		M.chosen:set({})
		return
	end
	local all = table.clone(kept:peek())
	all[key] = nil
	kept:set(all)
end

M.is_new = is_new
function M.has_words(key: string): boolean
	local d = M.draft_peek(key)
	return d.text ~= "" or #d.cargo > 0
end
function M.new_before(a: string, b: string): boolean
	return (tonumber(string.sub(a, 5)) or 0) < (tonumber(string.sub(b, 5)) or 0)
end
-- The new chats' drafts written in, "" first, then in the order they were made.
function M.new_drafts(): { string }
	local out: { string } = {}
	local h = here:get()
	if is_new(h) and (M.text:get() ~= "" or #M.cargo:get() > 0) then
		table.insert(out, h)
	end
	for key, d in kept:get() do
		if key ~= h and is_new(key) and words_in(d) then
			table.insert(out, key)
		end
	end
	table.sort(out, M.new_before)
	return out
end
function M.mint_new(): string
	local n = minted:peek() + 1
	minted:set(n)
	return `new:{n}`
end

export type Whole = { text: string, cargo: { any }, quotes: { [string]: string }, number: number? }
function M.whole_of(key: string): Whole
	local mine = here:peek() == key
	local d = if mine then nil else kept:peek()[key]
	return {
		text = if mine then M.text:peek() elseif d and type(d.text) == "string" then d.text else "",
		cargo = table.clone(cargo_in(key)),
		quotes = if mine then M.quotes:peek() elseif d and type(d.quotes) == "table" then d.quotes else {},
		number = numbered:peek()[key],
	}
end

function M.put_whole(key: string, w: Whole)
	ui.batch(function()
		if numbered:peek()[key] ~= w.number then
			local numbers = table.clone(numbered:peek())
			numbers[key] = w.number
			numbered:set(numbers)
		end
		if here:peek() == key then
			M.text:set(w.text)
			M.cargo:set(w.cargo)
			M.quotes:set(w.quotes)
			return
		end
		local all = table.clone(kept:peek())
		local d = table.clone(all[key] or {})
		d.text, d.cargo, d.quotes = w.text, w.cargo, w.quotes
		local empty = w.text == "" and #w.cargo == 0 and next(d.chosen or {}) == nil and d.refused == nil
		all[key] = if empty then nil else d
		kept:set(all)
	end)
end

local function joined(first: Whole, second: Whole): Whole
	local cargo = table.clone(first.cargo)
	for _, item in second.cargo do
		table.insert(cargo, item)
	end
	local quotes = table.clone(first.quotes)
	for line, passage in second.quotes do
		quotes[line] = passage
	end
	local top = math.max(first.number or 0, second.number or 0)
	return {
		text = if first.text == "" then second.text elseif second.text == "" then first.text else `{first.text}\n\n{second.text}`,
		cargo = M.renumber(cargo),
		quotes = quotes,
		number = if top > 0 then top else nil,
	}
end

function M.move_draft(from: string, to: string): { from: Whole, to: Whole }
	local a, b = M.whole_of(from), M.whole_of(to)
	-- (Skill and command chips are named per chat: carried as their links.)
	local words = references.decode(M.refs_key(to), references.encode(M.refs_key(from), a.text))
	ui.batch(function()
		M.put_whole(to, joined(b, { text = words, cargo = a.cargo, quotes = a.quotes, number = a.number }))
		M.put_whole(from, { text = "", cargo = {}, quotes = {} })
	end)
	return { from = a, to = b }
end

-- Follows chat `key`'s draft as `draft_of` does, wherever it is, without
-- reading its words (a long draft isn't copied for it): what follows it to
-- learn that it changed reads it only then (a split pane's mirror of it).
function M.track_draft_of(key: string)
	if here:get() == key then
		M.text:len()
		M.cargo:get()
		M.chosen:get()
	else
		kept:len()
	end
end

-- The options picked in chat `key`'s question, as `draft_of` reads them,
-- without its words.
function M.chosen_of(key: string): { [string]: boolean }
	if here:get() == key then
		return M.chosen:get()
	end
	local d = kept:get()[key]
	return if d and type(d.chosen) == "table" then d.chosen else {}
end

-- The length of chat `key`'s draft's words, in bytes, as `draft_of`
-- reads them (out in the composer, its draft not copied for it).
function M.draft_size_of(key: string): number
	if here:peek() == key then
		return Draft.size(M.text)
	end
	local d = kept:peek()[key]
	return if d and type(d.text) == "string" then #d.text else 0
end

-- Chat `key`'s draft's words as they are, read without following them
-- (`draft_of` follows them): what a plugin's `zeron.compose` adds to.
function M.peek_text_of(key: string): string
	if here:peek() == key then
		return M.text:peek()
	end
	local d = kept:peek()[key]
	return if d and type(d.text) == "string" then d.text else ""
end

-- Chat `key`'s draft changed where it's shown by its id (`field`: "text"
-- or "chosen"): written wherever it is, as `draft_of` reads it.
function M.set_draft_of(key: string, field: string, value: any)
	if here:peek() == key then
		if field == "text" then
			M.text:set(value)
		else
			M.chosen:set(value)
		end
		return
	end
	local all = table.clone(kept:peek())
	local d = table.clone(all[key] or { text = "", cargo = {}, quotes = {} })
	d[field] = value
	all[key] = d
	kept:set(all)
end

-- A chip taken off chat `key`'s draft, wherever the draft is.
function M.remove_cargo_of(key: string, id: string)
	if here:peek() == key then
		M.remove_cargo(id)
		return
	end
	local all = table.clone(kept:peek())
	local d = all[key]
	if not d then
		return
	end
	d = table.clone(d)
	local left = {}
	for _, item in d.cargo or {} do
		if item.id ~= id then
			table.insert(left, item)
		elseif type(d.text) == "string" then
			d.text = without_chip(d.text, item)
		end
	end
	d.cargo = M.renumber(left)
	all[key] = d
	kept:set(all)
end

-- The page's picks numbered 1, 2, 3… in the order made, no gap where one
-- left: the chips, the tokens they send, and (the browser tab following)
-- the marks on the page.
function M.renumber(list: { any }): { any }
	local n = 0
	local out = {}
	for _, item in list do
		if item.kind == "annotation" and item.element ~= nil and item.index then
			n += 1
			if item.index ~= n then
				local next_item = table.clone(item)
				next_item.index = n
				if type(item.annotation) == "table" then
					local a = table.clone(item.annotation)
					a.index = n
					next_item.annotation = a
				end
				item = next_item
			end
		end
		table.insert(out, item)
	end
	return out
end

-- Which of `sessions`' devices is this one.
local here_device = ui.computed(function(): string?
	for _, d in sessions.devices:get() or {} do
		if d.here then
			return d.id
		end
	end
	return nil
end)

-- The accounts' key for a device: "" for this one, whose engine answers
-- for itself; "cloud" for any of Cloud's (a chat's machine, or the
-- account's Cloud device), whose accounts are the vault's, every Cloud
-- machine's alike, as Settings → Providers shows them (this engine answers
-- for "cloud": crates/engine cloud_accounts). A machine can't be asked:
-- its accounts aren't its own, and asking would wake it.
local function account_key(device: string?): string
	if device == nil or device == here_device:get() then
		return ""
	end
	if cloud.on_device(device) or device == cloud.device() then
		return "cloud"
	end
	return device
end

-- Models of Cursor's own pool (Composer, Grok), not the third-party one.
local CURSOR_OWN = { "default", "auto", "composer", "vega", "grok", "cursor-grok" }
local function cursor_pool(model: string): string
	for _, prefix in CURSOR_OWN do
		if model == prefix or string.find(model, `^{prefix}[%-%.]`) then
			return "Cursor"
		end
	end
	return "Other"
end

-- What the ring reads: the fuller window; for Cursor, the pool the model
-- draws from, if that's reported. Nil: nothing reported.
local function account_used(account: any, harness: string, model: string): number?
	if harness == "cursor" then
		local id = string.match(model, "^[^/]+/(.+)$") or model
		local pool = cursor_pool(id)
		for _, w in account.windows do
			if w.label == pool then
				return w.used
			end
		end
	end
	local most: number? = nil
	for _, w in account.windows do
		most = if most then math.max(most, w.used) else w.used
	end
	return most
end

-- An account's windows in words, or why it has none to show: one not in use
-- says what's wrong with it, as a signed-out one's old figures would mislead.
local function account_detail(account: any): string
	if account.error and (not account.active or #account.windows == 0) then
		return account.error
	end
	local parts: { string } = {}
	for _, w in account.windows do
		table.insert(parts, `{w.label} {math.floor(w.used * 100 + 0.5)}%`)
	end
	return table.concat(parts, " · ")
end

-- Whether `harness` keeps a login per model provider (Pi, OpenCode,
-- Hermes): Zeron signs in to its providers one by one, or its accounts say
-- their provider.
local function per_provider(harness: string, list: { any }?): boolean
	if agents.logins:get()[harness] ~= nil then
		return true
	end
	for _, a in list or {} do
		if a.provider ~= nil then
			return true
		end
	end
	return false
end

-- The composer's accounts on `device` (ds.ModelPicker's `accounts`,
-- `account_group`, `on_account`, `on_accounts_open`), whichever computer it
-- is: asked for as it's first seen. An agent that keeps a login per model
-- provider shows only the model's provider's: switching its ChatGPT
-- account never touches its Claude one.
function M.model_accounts(device: () -> string?): any
	ui.effect(function()
		agents.watch_device(account_key(device()))
	end)
	local function listed(harness: string): { any }?
		local on = agents.on_device:get()[account_key(device())]
		return on and on[harness]
	end
	return {
		accounts = function(harness: string, model: string, provider: string?): { any }?
			local all = listed(harness)
			if not all then
				return nil
			end
			local list = all
			if provider and per_provider(harness, all) then
				list = {}
				for _, a in all do
					if a.provider == provider then
						table.insert(list, a)
					end
				end
			end
			local out = table.create(#list)
			for i, a in list do
				out[i] = {
					id = a.id,
					email = a.email,
					plan = a.plan,
					active = a.active,
					can_switch = a.can_switch,
					used = account_used(a, harness, model),
					detail = account_detail(a),
					windows = a.windows,
				}
			end
			return out
		end,
		-- The model's provider, named, and adding one of its accounts: Settings
		-- → Providers, on the composer's device, its sign-in under way there.
		account_group = function(harness: string, provider: string?): any
			if not provider or not per_provider(harness, listed(harness)) then
				return nil
			end
			local offer = (agents.logins:get()[harness] or {})[provider]
			local name = if offer then offer.name else nil
			for _, a in listed(harness) or {} do
				if not name and a.provider == provider then
					name = a.group
				end
			end
			if not name then
				return nil
			end
			local key = account_key(device())
			-- Another account: a sign-in, on the composer's device; else an
			-- API key, typed in Settings — on this Mac alone (a key never
			-- travels to another device).
			if offer and offer.login then
				return {
					name = name,
					add = locale.s("ds.provider_card.add_named_account", { name = offer.login }),
					on_add = function()
						agents.target(if key == "" then nil else key)
						route.settings("providers")
						agents.sign_in_to(harness, provider)
					end,
				}
			elseif offer and offer.key and key == "" then
				return {
					name = name,
					add = locale.s("ds.model_picker.add_key", { name = offer.key }),
					on_add = function()
						agents.target(nil)
						route.settings("providers")
						agents.key_request:set(`{harness}:{provider}`)
					end,
				}
			end
			return { name = name }
		end,
		on_account = function(harness: string, id: string)
			agents.switch_on(account_key(device()), harness, id):after(function() end, function()
				toast.show { title = locale.s("chat.composer.account_not_switched"), tone = "warning" }
			end)
		end,
		on_accounts_open = function()
			agents.refresh_device(account_key(device()))
		end,
	}
end

return M
