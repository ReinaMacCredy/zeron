--!strict
-- A Bot's board fields on its tasks (crates/engine plugins/bots.rs `BotBoard`):
-- status, who claims it, folder and note, read for every pinned chat (a Bot)
-- and kept by task id.
local sessions = require("@account/sessions")

local M = {}

M.rows = ui.shared("tasks.bot_board", {} :: { [string]: any })

M.WORDS = { up_next = "Up next", working = "Working", in_review = "In review", complete = "Complete" }
M.TONES = { up_next = "$text.secondary", working = "$warning", in_review = "$accent", complete = "$success" }

function M.load()
	local bots = sessions.places:peek().pinned
	local got: { [string]: any } = {}
	local left = #bots
	if left == 0 then
		M.rows:set({})
		return
	end
	local function one()
		left -= 1
		if left == 0 then
			M.rows:set(got)
		end
	end
	for _, bot in bots do
		ui.call("bots.board", { botId = bot, action = "list" }):after(function(reply: any)
			for _, row in (reply and reply.tasks) or {} do
				local r = table.clone(row)
				r.bot = bot
				got[r.taskId] = r
			end
			one()
		end, one)
	end
end

function M.of(id: string): any
	return M.rows:get()[id]
end

-- One line for the task's row: "In review · Folder · claimed".
function M.said(id: string): string
	local r = M.of(id)
	if not r then
		return ""
	end
	return `{r.status or ""}|{r.folder or ""}|{r.claimedBy or ""}|{r.note or ""}`
end

return M
