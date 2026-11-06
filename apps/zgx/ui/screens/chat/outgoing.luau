-- What a send carries, from what the draft holds: the window's composer and a
-- side chat's send the same.
--
--   local out = Outgoing.parts(typed, text, carried)
--   out.words, out.text (with its quotes, picks and comments), out.attachments,
--   out.appshots, out.shown (the chips the prompt shows at once)

local Quote = require("@ds/quote")
local Annotation = require("@ds/annotation")
local Comments = require("@app/screens/chat/comments")
local Invocations = require("@app/screens/chat/invocations")
local Appshots = require("@app/screens/chat/appshots")
local shared = require("@app/screens/chat/shared")

local M = {}

local function shown(list: { any }, quotes: { Quote.Quote }, picks: { { [string]: any } }, said: { Comments.Comment }): { any }
	local out = {}
	for _, a in list do
		table.insert(out, {
			kind = if a.kind == "image" then "image" else "file",
			name = a.name,
			src = if a.kind == "image" then a.path else nil,
			path = a.path,
		})
	end
	-- And each passage it carries, as a chip (as the host's copy shows
	-- them: chat.rs `split_quotes`).
	for _, q in quotes do
		table.insert(out, { kind = "file", icon = "cursor", name = Quote.where(q), meta = Quote.excerpt(q.text) })
	end
	-- And each element picked on a page (chat.rs `split_annotations`).
	for _, chip in Annotation.shown(picks) do
		table.insert(out, chip)
	end
	-- And its review comments, one chip (chat.rs `comments_chip`).
	if #said > 0 then
		local places: { string } = {}
		for _, c in said do
			table.insert(places, Comments.place(c))
		end
		table.insert(out, {
			kind = "file",
			icon = "chat-round-line",
			name = ui.t("chat.composer.comments", { n = #said }):get(),
			meta = table.concat(places, ", "),
		})
	end
	return out
end

-- `typed`, `carried`: what `shared.take` handed out; `text`: the words when
-- nothing was typed (a prompt sent without being typed).
function M.parts(typed: string, text: string, carried: { any }): { [string]: any }
	local attachments = {}
	-- Passages attached from the conversation: a block after the words
	-- (Quote.with_quotes), each with who said it and when.
	local quotes: { Quote.Quote } = {}
	-- Elements picked on a page in the panel's browser: each a token
	-- after the words (Annotation.with_annotations), which the engine
	-- expands for the agent.
	local picks: { { [string]: any } } = {}
	-- Review comments left on files: a block after the words
	-- (screens/chat/comments), each its file and line.
	local comments: { Comments.Comment } = {}
	for _, item in carried do
		if item.kind == "comment" then
			table.insert(comments, { file = item.file, line = item.line, body = item.body, to = item.to, side = item.side, snippet = item.snippet })
		elseif item.path then
			-- (A project's file: on the session's computer already.)
			-- (An Appshot goes as its picture; what it shows of the app, below.)
			table.insert(attachments, { kind = if item.kind == "appshot" then "image" else item.kind, path = item.path, name = item.name, rel = item.rel, on_host = item.on_host })
		elseif item.kind == "annotation" and item.quote then
			table.insert(quotes, { text = item.quote, who = item.name, from = item.from })
		elseif item.kind == "annotation" and item.annotation then
			table.insert(picks, item)
		elseif item.kind == "chat" and item.chat then
			-- Another chat mentioned (a session dropped from the sidebar):
			-- its link in the words, the engine's to read out to the agent.
			local link = shared.chat_link(item.name, item.chat)
			typed = if typed ~= "" then `{typed} {link}` elseif text ~= "" then `{text} {link}` else link
		end
	end
	-- (Each command or skill picked: its canonical reference.)
	local words = Invocations.encode(if typed ~= "" then typed else text)
	return {
		words = words,
		text = Comments.with_comments(Annotation.with_annotations(Quote.with_quotes(words, quotes), picks), comments),
		attachments = attachments,
		appshots = Appshots.of_cargo(carried),
		shown = shown(attachments, quotes, picks, comments),
	}
end

return M
