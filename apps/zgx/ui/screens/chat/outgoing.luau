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
local HtmlPicks = require("@ds/html_picks")
local Carried = require("@app/screens/chat/carried")

local M = {}

-- The badges staged text makes on the sent message (as the host reads them
-- back, chat.rs `quote_badges`): one for its annotations, one for its plain
-- selections, each listing what it quotes.
function M.badges(quotes: { Quote.Staged }): { any }
	local notes, plain = {}, {}
	for _, q in quotes do
		if q.comment ~= nil then
			local c = string.match(q.comment, "^%s*(.-)%s*$") or ""
			table.insert(notes, { n = #notes + 1, path = q.source, text = q.text, comment = if c ~= "" then c else nil })
		else
			table.insert(plain, { text = q.text })
		end
	end
	local out = {}
	if #notes > 0 then
		table.insert(out, { kind = "badge", badge = "annotations", name = "", details = notes })
	end
	if #plain > 0 then
		table.insert(out, { kind = "badge", badge = "selections", name = "", details = plain })
	end
	return out
end

local function shown(list: { any }, quotes: { Quote.Staged }, picks: { { [string]: any } }, said: { Comments.Comment }, cards: { HtmlPicks.Card }, chats: { any }): { any }
	local out = {}
	-- (As chat.rs `page_picks` reads them back.)
	for _, c in cards do
		for _, p in c.picks do
			table.insert(out, { kind = "pick", name = p.label, card = c.card, group = p.group or "", code = if (p.group or "") ~= "" then p.code else p.label, label = p.label })
		end
	end
	for _, a in list do
		table.insert(out, {
			kind = if a.kind == "image" then "image" else "file",
			name = a.name,
			src = if a.kind == "image" then a.path else nil,
			path = a.path,
		})
	end
	for _, b in M.badges(quotes) do
		table.insert(out, b)
	end
	-- And each element picked on a page (chat.rs `split_annotations`).
	for _, chip in Annotation.shown(picks) do
		table.insert(out, chip)
	end
	-- And each chat attached, a card with its agent's mark (chat.rs `chat_chip`).
	for _, c in chats do
		table.insert(out, { kind = "chat", name = c.name, chat = c.chat, mark = c.mark })
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
	-- Text selected and staged (annotations, plain selections): Zeron's
	-- blocks after the words and the tokens (Quote.with_quotes).
	local quotes: { Quote.Staged } = {}
	-- Elements picked on a page in the panel's browser: each a token
	-- after the words (Annotation.with_annotations), which the engine
	-- expands for the agent.
	local picks: { { [string]: any } } = {}
	-- Review comments left on files: a block after the words
	-- (screens/chat/comments), each its file and line.
	local comments: { Comments.Comment } = {}
	local chosen: { any } = {}
	local chats: { { name: string, chat: string, mark: string? } } = {}
	for _, item in carried do
		if item.kind == "pick" then
			table.insert(chosen, item)
		elseif item.kind == "comment" then
			table.insert(comments, { file = item.file, line = item.line, body = item.body, to = item.to, side = item.side, snippet = item.snippet })
		elseif item.path then
			-- (A project's file: on the session's computer already.)
			-- (An Appshot goes as its picture; what it shows of the app, below.)
			table.insert(attachments, { kind = if item.kind == "appshot" then "image" else item.kind, path = item.path, name = item.name, rel = item.rel, on_host = item.on_host })
		elseif item.kind == "quote" then
			table.insert(quotes, { text = item.text, comment = item.comment, source = item.source, terminal = item.terminal })
		elseif item.kind == "annotation" and item.quote then
			table.insert(quotes, { text = item.quote })
		elseif item.kind == "annotation" and item.annotation then
			table.insert(picks, item)
		elseif item.kind == "chat" and item.chat then
			-- Another chat attached (a session dropped from the sidebar): in
			-- a block after the words (Carried.with_chats), the sent message
			-- a card for it (src/chat.rs `split_chats`).
			table.insert(chats, { name = item.name, chat = item.chat, mark = item.mark })
		end
	end
	-- (A mention of an attachment no longer attached leaves as its plain label.)
	local words = Carried.settle(Invocations.encode(if typed ~= "" then typed else text), carried)
	local cards = HtmlPicks.cards(chosen)
	return {
		words = words,
		text = Carried.with_chats(Comments.with_comments(Quote.with_quotes(Annotation.with_annotations(HtmlPicks.with_picks(words, cards), picks), quotes), comments), chats),
		attachments = attachments,
		appshots = Appshots.of_cargo(carried),
		shown = shown(attachments, quotes, picks, comments, cards, chats),
	}
end

return M
