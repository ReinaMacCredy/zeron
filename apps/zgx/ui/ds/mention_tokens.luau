-- Mention tokens, in words alone (no part of the window, so the tests can
-- hold them to account): the token the caret is in, what a pick puts in
-- its place, how a list is filtered, a file's shortest own name, and the
-- draft's highlight. ds/composer_mentions wires these to the composer.

-- (By the `@ds` alias, not `@self`: the test runner resolves the one and
-- not the other.)
local markdown = require("@ds/markdown_edit")
local Icon = require("@ds/icon")

local M = {}

export type Trigger = "@" | "/" | "$"
export type Item = {
	-- What goes into the draft after the trigger (a path, a name).
	value: string,
	label: string,
	-- A line after the name: a file's folder, what a command does.
	detail: string?,
	-- A file's (its icon by its name), a folder's, or an icon of its own.
	file: boolean?,
	dir: boolean?,
	icon: Icon.Name?,
	key: string?,
	src: string?,
	heading: string?,
	attached: boolean?,
	searched: boolean?,
	blocked: string?,
	terminal_only: boolean?,
}
-- The items each trigger lists; nil while they load.
export type Sources = { [string]: { Item }? }
-- The token at the caret: its trigger, what has been typed after it, and
-- where it runs, in bytes from 0 (an input's selection's).
export type Token = { trigger: Trigger, query: string, from: number, to: number }

-- What a token is made of after its trigger.
local WORD = "[%w%./_:%-]"

-- The token the caret is in, if any: its trigger, what has been typed
-- before the caret (the query), and where the whole token runs, to its
-- end past the caret too, so a pick replaces all of it (a caret in the
-- middle of `@queue.rs` picking another file leaves no tail behind). A
-- trigger starts a token only at a start, after a space, or after an
-- opening bracket.
function M.token(text: string, caret: number): Token?
	local before = string.sub(text, 1, caret)
	local function starts(at: number): boolean
		local prev = if at > 1 then string.sub(before, at - 1, at - 1) else ""
		return prev == "" or string.match(prev, "[%s%(%[{>]") ~= nil
	end
	-- Where the token ends: over its characters after the caret (a
	-- command's never cross a slash).
	local function ends(chars: string): number
		local rest = string.match(string.sub(text, caret + 1), "^" .. chars .. "*") or ""
		return caret + #rest
	end
	-- Inside a fenced block, nothing.
	local function fenced(at: number): boolean
		local fences = 0
		for line in string.gmatch(string.sub(text, 1, at), "[^\n]*") do
			if string.match(line, "^%s*```") then
				fences += 1
			end
		end
		return fences % 2 == 1
	end
	-- `@task …` runs past spaces to the caret, the scope's own (composer.rs `task_scope_token`).
	local line = string.match(before, "[^\n]*$") or ""
	local mark = string.match(line, "^.*()@")
	if mark then
		local at_task = #before - #line + mark
		local q = string.sub(before, at_task + 1)
		if string.lower(string.sub(q, 1, 5)) == "task " and starts(at_task) and not fenced(at_task) then
			local rest = string.match(string.sub(text, caret + 1), "^%S*") or ""
			return { trigger = "@" :: Trigger, query = q, from = at_task - 1, to = caret + #rest }
		end
	end
	-- `@` and `$`: back over the token's characters (a path's slashes among
	-- them) to the trigger.
	local at = #before
	while at > 0 do
		local ch = string.sub(before, at, at)
		if not string.match(ch, WORD) then
			break
		end
		at -= 1
	end
	local trigger = string.sub(before, at, at)
	if (trigger == "@" or trigger == "$") and starts(at) then
		local query = string.sub(before, at + 1)
		local to = ends(WORD)
		-- "$12" is money and "$HOME" a variable, wherever the caret sits in
		-- them; "$a/b" a path.
		if trigger == "$" then
			local word = string.sub(text, at + 1, to)
			if string.match(word, "^[%d%u]") or string.find(word, "/", 1, true) then
				return nil
			end
		end
		if fenced(at) then
			return nil
		end
		return { trigger = trigger :: Trigger, query = query, from = at - 1, to = to }
	end
	-- `/`: the run after the last slash, where that slash starts a word (a
	-- path's slashes never do), with no slash in the run.
	local s = string.find(before, "/[%w%._:%-]*$")
	if s and starts(s) and not fenced(s) then
		return { trigger = "/", query = string.sub(before, s + 1), from = s - 1, to = ends("[%w%._:%-]") }
	end
	return nil
end

-- What a pick puts in the draft: the token's whole run replaced by the
-- item's token and one space (none added where a space already follows).
-- Pure, for the tests: `(from, to, text)`, bytes from 0 as a field's
-- selection counts them, and the token written (a file by its name).
function M.pick(text: string, caret: number, item: Item, sources: Sources): (number?, number?, string?, string?)
	local t = M.token(text, caret)
	if not t then
		return nil, nil, nil, nil
	end
	-- (An action, as Zeron's own `/model`: taken, its token goes.)
	if (item :: any).consume then
		return t.from, t.to, "", ""
	end
	-- (A task: its link, `[title](zeron-task:<id>)`, as Zeron's task mention.)
	if (item :: any).task_link then
		local link = (item :: any).task_link :: string
		local after = string.sub(text, t.to + 1, t.to + 1)
		return t.from, t.to, if after == " " then link else `{link} `, link
	end
	if item.attached then
		local words = if string.sub(text, t.to + 1, t.to + 1) == " " then item.label else `{item.label} `
		return t.from, t.to, words, item.label
	end
	local word = if item.file then M.short(sources[t.trigger] or {}, item) else item.value
	local token = `{t.trigger}{word}`
	local after = string.sub(text, t.to + 1, t.to + 1)
	local replacement = if after == " " then token else `{token} `
	return t.from, t.to, replacement, token
end

-- The items that match what was typed: those whose name or value holds it,
-- the ones starting with it first.
function M.filter(items: { Item }, query: string): { Item }
	local q = string.lower(query)
	if q == "" then
		return items
	end
	local starts: { Item } = {}
	local holds: { Item } = {}
	for _, it in items do
		local label = string.lower(it.label)
		local value = string.lower(it.value)
		-- (The host's search is fuzzy: its answers needn't hold the query.)
		if it.searched then
			table.insert(holds, it)
		elseif string.sub(label, 1, #q) == q or string.sub(value, 1, #q) == q then
			table.insert(starts, it)
		elseif string.find(label, q, 1, true) or string.find(value, q, 1, true) then
			table.insert(holds, it)
		end
	end
	for _, it in holds do
		table.insert(starts, it)
	end
	return starts
end

function M.attachments(items: { Item }, query: string): { Item }
	local function plain(s: string): string
		return string.lower((string.gsub(s, "%s", "")))
	end
	local wanted = plain(query)
	local out: { Item } = {}
	for _, it in items do
		if string.find(plain(it.label), wanted, 1, true) then
			local row = table.clone(it)
			row.attached = true
			row.key = `attachment:{it.value}`
			table.insert(out, row)
		end
	end
	return out
end

-- The shortest end of a file's path that tells it from the others of its
-- name: its name alone, else a folder more until no other ends the same.
function M.short(items: { Item }, item: Item): string
	local function tail(path: string, parts: number): string
		local segments: { string } = {}
		for seg in string.gmatch(path, "[^/]+") do
			table.insert(segments, seg)
		end
		local from = math.max(1, #segments - parts + 1)
		return table.concat(segments, "/", from)
	end
	local parts = 1
	while parts < 8 do
		local mine = tail(item.value, parts)
		local clash = false
		for _, other in items do
			if other.value ~= item.value and tail(other.value, parts) == mine then
				clash = true
				break
			end
		end
		if not clash then
			return mine
		end
		parts += 1
	end
	return item.value
end

-- The draft's highlight: markdown as it is typed, and each token
-- mentioned in its type. A span's state is the markdown's (a fenced block).
function M.highlight(line: string, state: number): ({ { any } }, number)
	-- (A pasted megabyte on one line, shown in the field: plain, as Zeron's past 128 KiB.)
	if #line > 128 * 1024 then
		return {}, state
	end
	local spans, next = markdown.highlight(line, state)
	if state ~= 0 then
		return spans, next
	end
	local at = 1
	while true do
		local s, e = string.find(line, "[@/$]" .. WORD .. "+", at)
		if not s or not e then
			break
		end
		local from = s
		local token = string.sub(line, s, e)
		local prev = if from > 1 then string.sub(line, from - 1, from - 1) else ""
		local ok = prev == "" or string.match(prev, "[%s%(%[{>]") ~= nil
		local trigger = string.sub(token, 1, 1)
		if ok and trigger == "$" and (string.match(token, "^%$[%d%u]") or string.find(token, "/", 2, true)) then
			ok = false
		end
		if ok and trigger == "/" and string.find(token, "/", 2, true) then
			ok = false
		end
		if ok then
			-- A token ending in punctuation gives it back to the sentence.
			local last = e
			while last > from and string.match(string.sub(line, last, last), "[%.:,]") do
				last -= 1
			end
			if last > from then
				local ink = if trigger == "@"
					then "$zeron.composer.token.file"
					elseif trigger == "$" then "$zeron.composer.token.skill"
					else "$zeron.composer.token.action"
				table.insert(spans, { from, from, "$zeron.composer.token.trigger" } :: any)
				table.insert(spans, { from + 1, last, ink, weight = 500 } :: any)
			end
		end
		at = e + 1
	end
	return spans, next
end

return M
