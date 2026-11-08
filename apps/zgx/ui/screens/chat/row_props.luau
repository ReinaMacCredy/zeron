-- What a transcript's rows act on for one chat (a secret's answer, Log in, Claude Code's /model, which links open), the same on the chat page, a split's pane, a lane and a run's log.

local store = require("@app/screens/chat/store")
local route = require("@app/screens/shell/route")
local FileLinks = require("@app/screens/chat/file_links")
local Shared = require("@app/screens/chat/shared")
local Computer = require("@app/screens/settings/computer")

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
	if props.open_link then
		props.file_link = function(target: string): boolean
			local s = peek()
			local r = FileLinks.resolve(target, if s then s.cwd else nil)
			return r ~= nil and not r.outside
		end
	end
	return props
end

-- The engine's `parse_computer_invocation`: up to three spaces, then /computer-use alone or before a space.
function M.invokes_computer(text: string): boolean
	local command, after = string.match(text, "^ ? ? ?(/[%w%-]+)(.*)$")
	return command ~= nil and string.lower(command) == "/computer-use" and (after == "" or string.match(after, "^[ \t\r\n]") ~= nil)
end

return M
