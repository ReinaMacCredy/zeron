-- What a transcript's rows act on for one chat (a secret's answer, Log in, Claude Code's /model, which links open), the same on the chat page, a split's pane, a lane and a run's log.

local store = require("@app/screens/chat/store")
local route = require("@app/screens/shell/route")
local FileLinks = require("@app/screens/chat/file_links")
local FileCards = require("@app/screens/chat/file_cards")
local Shared = require("@app/screens/chat/shared")
local Computer = require("@app/screens/settings/computer")
local FilePaths = require("@app/screens/files/paths")
local boundary = require("@app/screens/shell/boundary")
local ConfigPanel = require("@app/screens/chat/config_panel")

local M = {}

-- `props` with them added: `chat()` its id, `session()` its session (followed), `peek()` the same unfollowed (a paragraph isn't drawn again as the session moves). A link's glyph only where `props.open_link` opens it.
function M.add(props: { [string]: any }, chat: () -> string?, session: () -> any, peek: () -> any): { [string]: any }
	props.on_sign_in = function()
		route.settings("providers")
	end
	props.current_model = function(): string?
		local s = session()
		return if s then s.model else nil
	end
	props.on_model = function(model: string)
		local id = chat()
		if id then
			store.configure(id, { model = model })
		end
	end
	props.on_secret = function(request: string, secret: string?): any
		local id = chat()
		return if id then store.answer_secret(id, request, secret) else nil
	end
	props.on_computer = function(request: string, question: string, answer: string): any
		local id = chat()
		return if id then store.answer_computer(id, request, question, answer) else nil
	end
	props.on_config_reopen = function()
		local id = chat()
		if id then
			ConfigPanel.reopen(id)
		end
	end
	props.computer_disclosure = Computer.shows_disclosure
	props.on_computer_approved = Computer.mark_approved
	props.on_computer_enable = function()
		local id = chat()
		if not id then
			return
		end
		local draft = Shared.draft_peek(id).text or ""
		if not M.invokes_computer(draft) then
			Shared.set_draft_of(id, "text", if string.match(draft, "^%s*$") then "/computer-use " else `/computer-use {draft}`)
		end
		Shared.focus:add(1)
	end
	-- (Loaded with its first block, off the page's mount.)
	props.review_block = function(_m: any): Node
		return (require :: any)("@app/screens/chat/review").Block({ chat = chat, open_link = props.open_link })
	end
	-- A shell block's Run: the chat's own terminal, on its device, wherever the panel can open one.
	props.run_code = {
		can = function(): boolean
			local s = session()
			return s ~= nil and FilePaths.target_of(nil, s) ~= nil
		end,
		run = function(text: string)
			local panel: any = boundary.load("@app/screens/panel")
			local s = peek()
			if panel and panel.run_in_terminal and s then
				panel.run_in_terminal(s, text)
			end
		end,
	}
	-- An inline question's card only while it's the one the chat waits on (an older one left open can't be answered here).
	props.ask_card = {
		asked = function(request: string): boolean
			local s = session()
			local a = if s then s.ask else nil
			return a ~= nil and a.request == request and a.question ~= nil and a.question.inline == true
		end,
		answer = function(request: string, answer: any): any
			local id = chat()
			return if id then store.answer(id, request, { values = answer.values, text = "", note = answer.note }) else nil
		end,
	}
	props.on_visual_send = function(text: string, title: string): boolean
		return M.visual_send(chat(), peek(), text, title)
	end
	if props.open_link then
		props.file_link = function(target: string): boolean
			local s = peek()
			local r = FileLinks.resolve(target, if s then s.cwd else nil)
			return r ~= nil
		end
		props.file_cards = FileCards(peek)
	end
	return props
end

-- Through `chat.send`, never as an answer: the host queues it behind a running turn (one waiting on a question too), and a question left open with no turn to wait in refuses it.
function M.visual_send(id: string?, s: any, text: string, title: string): boolean
	if not id or not s then
		return false
	end
	if s.ask ~= nil and s.running ~= true then
		Shared.refuse_in(id, ui.t("ds.visual.send_waits"):get())
		return false
	end
	local from = string.gsub(if title ~= "" then title else "Visual", "[\r\n]+", " ")
	store.send({ session = id, text = `{text}\n\n(Sent from "{from}", not typed.)` } :: any):after(function() end, function()
		Shared.refuse_in(id, ui.t("chat.composer.didnt_send"):get())
	end)
	return true
end

-- The engine's `parse_computer_invocation`: up to three spaces, then /computer-use alone or before a space.
function M.invokes_computer(text: string): boolean
	local command, after = string.match(text, "^ ? ? ?(/[%w%-]+)(.*)$")
	return command ~= nil and string.lower(command) == "/computer-use" and (after == "" or string.match(after, "^[ \t\r\n]") ~= nil)
end

return M
