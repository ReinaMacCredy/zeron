-- The Review built-in (screens/chat/review): the chat page of
-- ui/chat_fixture.luau (its review host stood in for there) under the
-- window's title bar, where the chat's Review button stands. What
-- tests/review_plugin.test.luau drives.

local node = require("./chat_fixture")
local TitleBar = require("@app/screens/shell/title_bar")
local Builtins = require("@app/screens/plugins/builtins")

-- The Review built-in, on or off in Settings → Plugins.
for _, status in { "on", "off" } do
	ui.command {
		id = `review_builtin_{status}`,
		title = `Review: the built-in {status}`,
		run = function()
			Builtins.set("review", status == "on")
		end,
	}
end

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
