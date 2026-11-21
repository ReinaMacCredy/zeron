-- The chat fixture with two of its chats a Bot's: s2 (a conversation under way, its Bot archived) and e1 (a new Bot's, nothing said yet: the chat fixture's `go_empty`).

local app = require("./chat_fixture")
local store = require("@app/screens/bots/store")

local function bot(id: string, name: string, chat: string, template: string?, archived: boolean?): any
	return {
		id = id,
		name = name,
		chat = chat,
		avatar = { glyph = "bot", frame = "squircle", color = "blue" },
		pinned = false,
		archived = archived == true,
		unread = false,
		template = template,
	}
end

store.list:set({ state = "ready", bots = { bot("k1", "Release keeper", "s2", nil, true), bot("k2", "Flake hunter", "e1", "tests") } })

-- The chat page says what an agent asks (`chat.asks`): the first Bot's chat, an approval.
ui.command {
	id = "bot_asks",
	title = "The first Bot's chat asks for approval",
	run = function()
		ui.shared("chat.asks", {} :: { [string]: string }):set({ s2 = "approval" })
	end,
}

return app
