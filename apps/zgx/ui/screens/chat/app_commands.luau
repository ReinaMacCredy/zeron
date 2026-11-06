-- Zeron's own commands a send takes before any turn (crates/proto/src/
-- app_commands.rs): the first token exactly `/command`, the rest of the line
-- its argument. Leading indentation marks literal Markdown, so it never counts.

local store = require("@app/screens/chat/store")

local M = {}

-- The argument ("" when bare), or nil when the words aren't this command.
function M.argument(text: string, command: string): string?
	local head = `/{command}`
	if string.sub(text, 1, #head) ~= head then
		return nil
	end
	local rest = string.sub(text, #head + 1)
	if rest ~= "" and not string.find(rest, "^%s") then
		return nil
	end
	return string.match(rest, "^%s*(.-)%s*$") or ""
end

function M.side(text: string): string?
	return M.argument(text, "btw") or M.argument(text, "sidechat")
end

function M.compact(text: string): string?
	return M.argument(text, "compact")
end

-- `refuse` says why it can't go, by a catalogue key.
function M.run_compact(id: string?, session: any, running: boolean, focus: string, refuse: (key: string) -> (), failed: (why: string) -> ())
	if not id or not session then
		refuse("chat.composer.compact_needs_chat")
	elseif session.compacting then
		refuse("chat.composer.compact_in_progress")
	elseif running then
		refuse("chat.composer.compact_wait_turn")
	else
		local ok, answer = pcall(ui.call, "chat.compact", { id = id, focus = if focus ~= "" then focus else nil })
		if ok and answer then
			answer:after(function() end, function(e: any)
				failed(if type(e) == "table" and e.message then tostring(e.message) else tostring(e))
			end)
		end
	end
end

return M
