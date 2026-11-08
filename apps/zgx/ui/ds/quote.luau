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

-- The quote's line in the draft.
function M.line(q: Quote): string
	return `> {q.who}: {M.excerpt(q.text)}`
end

-- `draft` with `line`, on a line of its own, and room to write after it.
function M.add(draft: string, line: string): string
	local base = string.gsub(draft, "%s+$", "")
	if base == "" then
		return `{line}\n\n`
	end
	return `{base}\n\n{line}\n\n`
end

-- What of `quotes` (each passage by its line) the draft still holds.
function M.kept(draft: string, quotes: { [string]: string }): { [string]: string }
	local out = {}
	for line, text in quotes do
		if string.find(draft, line, 1, true) then
			out[line] = text
		end
	end
	return out
end

-- The draft as sent: each quote's line, the whole passage.
function M.expand(draft: string, quotes: { [string]: string }): string
	local out = draft
	for line, text in quotes do
		local at, to = string.find(out, line, 1, true)
		if at and to then
			local who = string.match(line, "^> (.-): ") or ""
			local body = string.gsub(string.gsub(text, "%s+$", ""), "\n", "\n> ")
			out = `{string.sub(out, 1, at - 1)}> {who}:\n> {body}{string.sub(out, to + 1)}`
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
