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

ui.command {
	id = "bot_asks",
	title = "The first Bot's chat asks for approval",
	run = function()
		ui.shared("chat.asks", {} :: { [string]: string }):set({ s2 = "approval" })
	end,
}

return app
