-- What a transcript's rows act on for one chat (a secret's answer, Log in, Claude Code's /model, which links open), the same on the chat page, a split's pane, a lane and a run's log.

local store = require("@app/screens/chat/store")
local route = require("@app/screens/shell/route")
local FileLinks = require("@app/screens/chat/file_links")

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
	if props.open_link then
		props.file_link = function(target: string): boolean
			local s = peek()
			local r = FileLinks.resolve(target, if s then s.cwd else nil)
			return r ~= nil and not r.outside
		end
	end
	return props
end

return M
