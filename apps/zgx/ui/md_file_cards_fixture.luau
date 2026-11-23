ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Markdown = require("@ds/markdown")
local files_stub = (require :: any)("@app/screens/files/api")
local FileCards = require("@app/screens/chat/file_cards")

local calls = ui.state("test_calls", {} :: { string })
local gone = ui.state("test_gone", {} :: { string })
local live = ui.state("test_live", false)
local extra = ui.state("test_extra", {} :: { string })

local REPORT = { "# Checkout latency, week 47", "", "p95 rose from 180 ms to 240 ms on Wednesday." }
for i = 4, 40 do
	table.insert(REPORT, `Line {i} of the report.`)
end
local CSV = { "day,p50,p95,p99", "2026-11-16,88,180,236", "2026-11-17,90,183,244", "2026-11-18,112,240,355" }
for i = 5, 169 do
	table.insert(CSV, `2026-12-{i},90,180,240`)
end
local TEXT: { [string]: string } = {
	["checkout-latency-report.md"] = table.concat(REPORT, "\n") .. "\n",
	["checkout-latency.csv"] = table.concat(CSV, "\n") .. "\n",
	["notes.txt"] = "one\ntwo\n",
}
local SIZES: { [string]: number } = { ["checkout-latency-report.md"] = 4100, ["checkout-latency.csv"] = 5200, ["chart.png"] = 81000, ["notes.txt"] = 8 }

local function later(value: any): any
	return {
		after = function(_self: any, ok: (any) -> (), _err: any)
			ui.after(30, function()
				ok(value)
			end)
		end,
	}
end
files_stub.list = function(_target: any, dir: string): any
	calls:push(`list:{dir}`)
	local entries = {}
	if dir == "" then
		for name, size in SIZES do
			if not table.find(gone:peek(), name) then
				table.insert(entries, { name = name, path = name, dir = false, kind = "file", size = size, ignored = false, readonly = false })
			end
		end
		for _, name in extra:peek() do
			table.insert(entries, { name = name, path = name, dir = false, kind = "file", size = 10, ignored = false, readonly = false })
		end
	end
	return later({ can_move = false, can_delete = false, truncated = false, entries = entries })
end
files_stub.read = function(_target: any, path: string): any
	return later({ path = path, text = TEXT[path], size = SIZES[path], encoding = "utf8", truncated = false })
end
files_stub.reveal = function(target: any, path: string): any
	calls:push(`reveal:{target.chat}:{target.root}:{path}`)
	return later(nil)
end

local REPLY = table.concat({
	"Proposal: put the TTL back at **300 s**. The report and the raw numbers:",
	"",
	"[checkout-latency-report.md](checkout-latency-report.md)",
	"checkout-latency.csv",
	"",
	"- chart.png",
	"- notes.txt",
	"",
	"string.find",
	"",
	"notes.txt missing.md",
	"",
	"later.md",
	"",
	"See checkout-latency-report.md for the rest.",
}, "\n")
local reply = ui.state("test_reply", REPLY)

local session = { id = "s1", cwd = "/work/atlas" }
local view = ui.ref()
local offset = ui.state("test_offset", 0)
return ui.scroll {
	ref = view,
	id = "reply",
	selectable = true,
	w = 640,
	h = 900,
	p = "$space.6",
	bg = "$background",
	Markdown {
		text = function(): string
			return reply:get()
		end,
		key = "file-cards",
		window = {
			key = "file-cards",
			view = view,
			offset = offset,
			live = function(): boolean
				return live:get()
			end,
		},
		group = "reply:f1",
		open = function(target: string)
			calls:push(`open:{target}`)
		end,
		pictures = {
			resolve = function(src: string): any
				return if src == "chart.png" then { state = "ready", src = "assets/composer/shell.png", path = src, here = true } else nil
			end,
			fetch = function(_src: string) end,
		},
		file_cards = FileCards(function(): any
			return session
		end),
	},
}
