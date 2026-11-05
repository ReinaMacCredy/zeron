-- crates/ui shell/navigation_focus.rs.

local boundary = require("@app/screens/shell/boundary")
local chat_shared = require("@app/screens/chat/shared")

return function()
	-- (Once what the move shows has been built.)
	ui.after(0, function()
		local panel: any = boundary.load("@app/screens/panel")
		if panel and panel.keyboard_back and panel.keyboard_back() then
			return
		end
		chat_shared.focus:add(1)
	end)
end
