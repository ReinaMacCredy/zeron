-- A reply with a ```markdown fence, mounted on its own for tests/md_fence.test.luau.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Markdown = require("@ds/markdown")

local REPLY = table.concat({
	"Here is the brief:",
	"",
	"```markdown",
	"# Handoff title",
	"",
	"- first **point**",
	"```",
}, "\n")

return ui.column {
	id = "reply",
	selectable = true,
	w_full = true,
	p = "$space.6",
	Markdown { text = REPLY },
}
