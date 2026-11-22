-- A passage quoted from a conversation, in words alone: who said it, a short
-- excerpt, how it reads in the draft, and the chip it makes in the
-- composer's strip. ds/transcript finds the passage; the composer carries it.
--
-- Mentioned, a quote is one line of the draft (`> Opus 5.5: the excerpt…`),
-- the whole passage kept beside the draft by that line, as a mention keeps
-- the path its token stands for: delete the line and the passage goes with
-- it. Sending puts the whole passage back (`expand`). Text annotated or
-- selected goes after the prompt in Zeron's own blocks (`with_quotes`).

local M = {}

-- `from`: where in the conversation it was said (its message's time), for
-- the agent to find it.
export type Quote = { text: string, who: string, from: string? }

-- An excerpt's most characters.
local EXCERPT = 72

-- `text` on one line, cut at a word to a short excerpt.
function M.excerpt(text: string, most: number?): string
	local line = string.gsub(string.gsub(text, "%s+", " "), "^ ", "")
	line = string.gsub(line, " $", "")
	local limit = most or EXCERPT
	if utf8.len(line) == nil or (utf8.len(line) :: number) <= limit then
		return line
	end
	local cut = string.sub(line, 1, (utf8.offset(line, limit + 1) :: number) - 1)
	local word = string.match(cut, "^(.*%S)%s+%S*$")
	return `{word or cut}…`
end

-- `text` on one line, uncut past what a chip can show: a chip fades its
-- detail at its own edge, as the others do.
function M.flat(text: string): string
	local line = string.gsub(string.gsub(text, "%s+", " "), "^ ", "")
	return string.sub(line, 1, 240)
end

-- What of a selection's first word it missed, from `source`, the text it
-- came from: a drag begun inside a word still quotes the word.
function M.lead(selected: string, source: string): string
	local head = string.match(selected, "^[^\n]+")
	if not head then
		return ""
	end
	-- (The few characters that open it: past them, markdown's markers may
	-- part the source from what shows.)
	local at = string.find(source, string.sub(head, 1, 12), 1, true)
	if not at then
		return ""
	end
	local from = at
	while from > 1 and string.match(string.sub(source, from - 1, from - 1), "[%w_'\128-\255]") do
		from -= 1
	end
	return string.sub(source, from, at - 1)
end

function M.where(q: Quote): string
	return if q.from then `{q.who}, {q.from}` else q.who
end

function M.same(a: Quote, b: Quote): boolean
	return a.text == b.text and a.who == b.who and a.from == b.from
end

-- The quote's line in the draft.
function M.line(q: Quote): string
	return `> {q.who}: {M.excerpt(q.text)}`
end

-- `line` where it stands as a whole line of `s`, from byte `init`.
local function find_line(s: string, line: string, init: number?): (number?, number?)
	local from = init or 1
	while true do
		local at, to = string.find(s, line, from, true)
		if not at or not to then
			return nil, nil
		end
		if (at == 1 or string.sub(s, at - 1, at - 1) == "\n") and (to == #s or string.sub(s, to + 1, to + 1) == "\n") then
			return at, to
		end
		from = at + 1
	end
end

-- The quote's line, apart from another passage's that reads the same in
-- `quotes` (a draft's, by line): numbered on, so each sends its own.
function M.line_in(quotes: { [string]: string }, q: Quote): string
	local base = M.line(q)
	local line, n = base, 1
	while quotes[line] ~= nil and quotes[line] ~= q.text do
		n += 1
		line = `{base} ({n})`
	end
	return line
end

-- `draft` with `line`, on a line of its own, and room to write after it.
function M.add(draft: string, line: string): string
	local base = string.gsub(draft, "%s+$", "")
	if base == "" then
		return `{line}\n\n`
	end
	return `{base}\n\n{line}\n\n`
end

-- `draft` with `line` on a line of its own at byte `at` (the caret: nil or
-- the end, after the rest as `add` puts it), and where the caret goes: the
-- start of what followed it.
function M.insert(draft: string, line: string, at: number?): (string, number)
	if at == nil or at >= #(string.gsub(draft, "%s+$", "")) then
		local out = M.add(draft, line)
		return out, #out
	end
	local before = string.gsub(string.sub(draft, 1, math.max(0, at)), "[ \t]+$", "")
	local after = string.gsub(string.sub(draft, math.max(0, at) + 1), "^[ \t]+", "")
	local lead = if before == "" or string.sub(before, -2) == "\n\n" then "" elseif string.sub(before, -1) == "\n" then "\n" else "\n\n"
	local trail = if string.sub(after, 1, 2) == "\n\n" then "" elseif string.sub(after, 1, 1) == "\n" then "\n" else "\n\n"
	local head = `{before}{lead}{line}{trail}`
	return head .. after, #head
end

-- What of `quotes` (each passage by its line) the draft still holds.
function M.kept(draft: string, quotes: { [string]: string }): { [string]: string }
	local out = {}
	for line, text in quotes do
		if find_line(draft, line) then
			out[line] = text
		end
	end
	return out
end

-- The draft as sent: each of a quote's lines, the whole passage.
function M.expand(draft: string, quotes: { [string]: string }): string
	local out = draft
	for line, text in quotes do
		local who = string.match(line, "^> (.-): ") or ""
		local body = string.gsub(string.gsub(text, "%s+$", ""), "\n", "\n> ")
		local whole = `> {who}:\n> {body}`
		local at, to = find_line(out, line)
		while at and to do
			out = `{string.sub(out, 1, at - 1)}{whole}{string.sub(out, to + 1)}`
			at, to = find_line(out, line, at + #whole)
		end
	end
	return out
end

-- The chip in the composer's strip (ds/composer_cargo).
function M.chip(q: Quote, id: string): { [string]: any }
	return { id = id, kind = "annotation", name = q.who, meta = M.flat(q.text), quote = q.text, from = q.from }
end

-- What staged text adds to a prompt, byte for byte as Zeron writes it
-- (crates/proto quotes.rs `with_quotes`), so either app reads the other's:
-- annotations (a quote with a comment, even an empty one) numbered under
-- their header, then plain selections quoted under theirs. `source`: the
-- file it was selected in.
export type Staged = { text: string, comment: string?, source: string?, terminal: boolean? }

M.ANNOTATION_HEADER = "Annotations (numbered; each quotes the text the user selected, then the user's comment on it):"
M.SELECTION_HEADER = "Selected text from this conversation (quoted):"
M.TERMINAL_HEADER = "Selected text from this chat's terminal (quoted):"
M.ONLY = "Refer to the selected text below."

local function trimmed(s: string): string
	return string.match(s, "^%s*(.-)%s*$") or s
end

function M.quoted(text: string): string
	local lines: { string } = {}
	for line in string.gmatch(trimmed(text) .. "\n", "([^\n]*)\n") do
		table.insert(lines, if line == "" then ">" else `> {line}`)
	end
	return table.concat(lines, "\n")
end

local function entry(n: number, q: Staged): string
	local from = if q.source then ` from {q.source}` else ""
	local comment = trimmed(q.comment or "")
	local said = if comment == "" then "User comment: (none)" else `User comment:\n{M.quoted(comment)}`
	return `{n}. Selected text{from}:\n{M.quoted(q.text)}\n{said}`
end

function M.with_quotes(text: string, quotes: { Staged }): string
	if #quotes == 0 then
		return text
	end
	local out = if text == "" then M.ONLY else text
	local entries: { string } = {}
	for _, q in quotes do
		if q.comment ~= nil then
			table.insert(entries, entry(#entries + 1, q))
		end
	end
	if #entries > 0 then
		out = `{out}\n\n{M.ANNOTATION_HEADER}\n{table.concat(entries, "\n\n")}`
	end
	for _, group in { { false, M.SELECTION_HEADER }, { true, M.TERMINAL_HEADER } } do
		local plain: { string } = {}
		for _, q in quotes do
			if q.comment == nil and (q.terminal == true) == group[1] then
				table.insert(plain, M.quoted(q.text))
			end
		end
		if #plain > 0 then
			out = `{out}\n\n{group[2]}\n{table.concat(plain, "\n\n")}`
		end
	end
	return out
end

return M
