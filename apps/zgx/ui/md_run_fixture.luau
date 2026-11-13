-- A reply's code blocks with a chat's terminal to run its shell blocks in, a stand-in that keeps what it's given (`test_ran`), for tests/md_fence.test.luau.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Markdown = require("@ds/markdown")

local ran = ui.state("test_ran", {} :: { string })

local LONG = "curl -fsSL https://example.com/install.sh | env PREFIX=$HOME/.local VERSION=1.2.3 CHANNEL=stable sh -s -- --yes --no-modify-path --quiet"

local REPLY = table.concat({
	"```bash",
	"cargo test -p zeron-engine\tquiet",
	"```",
	"",
	"```",
	"ls -la",
	"```",
	"",
	"```text",
	"echo plain",
	"```",
	"",
	"```console",
	"$ cd /tmp",
	"% make all",
	"❯ ./run",
	"output stays",
	"```",
	"",
	"```console",
	"only output, no prompt",
	"```",
	"",
	"```rust",
	"fn main() {}",
	"```",
	"",
	"```json",
	'{ "a": 1 }',
	"```",
	"",
	"```sh",
	LONG,
	"```",
}, "\n")

return ui.column {
	id = "reply",
	selectable = true,
	w = 420,
	p = "$space.6",
	Markdown {
		text = REPLY,
		run_code = {
			can = function(): boolean
				return true
			end,
			run = function(text: string)
				ran:push(text)
			end,
		},
	},
}
