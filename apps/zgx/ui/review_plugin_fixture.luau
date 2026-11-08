-- The Review built-in (screens/chat/review): the chat page of
-- ui/chat_fixture.luau (its review host stood in for there) under the
-- window's title bar, where the chat's Review button stands. What
-- tests/review_plugin.test.luau drives.

local node = require("./chat_fixture")
local TitleBar = require("@app/screens/shell/title_bar")

return ui.overlay {
	w_full = true,
	h_full = true,
	node,
	TitleBar {
		collapsed = ui.state("review_fixture_collapsed", false),
		files = ui.state("review_fixture_files", false),
		art = function(): boolean
			return false
		end,
	},
}
