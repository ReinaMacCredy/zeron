-- A reply's code blocks with a chat's terminal to run its shell blocks in, a stand-in that keeps what it's given (`test_ran`), and a stand-in python sandbox whose results the test sets (`test_py_runs`), for tests/md_fence.test.luau.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Markdown = require("@ds/markdown")

local ran = ui.state("test_ran", {} :: { string })
local py_ok = ui.state("test_py_available", true)
local py_runs = ui.state("test_py_runs", { ['print("restored")'] = { state = "done", output = "restored\n", ms = 240, exit = 0 } } :: { [string]: any })
local py_asked = ui.state("test_py_asked", {} :: { string })

local function py_set(code: string, result: any)
	py_runs:set_key({ code }, result)
end

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
	"$ printf '%s\\n' \\",
	"> hello",
	"hello",
	"> not a continuation",
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
	"",
	"```python",
	"print('hit')",
	"```",
	"",
	"```py",
	'print("restored")',
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
			python = {
				can = function(): boolean
					return py_ok:get()
				end,
				result = function(code: string): any
					return py_runs:get()[code]
				end,
				run = function(code: string)
					py_asked:push(`run:{code}`)
					py_set(code, { state = "running" })
				end,
				stop = function(code: string)
					py_asked:push(`stop:{code}`)
					py_set(code, { state = "stopped", output = "partial\n", ms = 600, exit = 137 })
				end,
				clear = function(code: string)
					py_asked:push(`clear:{code}`)
					py_set(code, nil)
				end,
			},
		},
	},
}
