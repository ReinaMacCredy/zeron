-- A test's host has no `pdf.*` actions (src/pdf.rs), so stand-ins answer there; the app's own host
-- (`zeron-zgx fixture=pdf_fixture.luau --inspect`, a file set in `test_pdf_doc`) draws real pages.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local FileViewer = require("@ds/file_viewer")
local PdfView = require("@ds/pdf_view")

local calls = ui.state("test_pdf_calls", 0)
local last = ui.state("test_pdf_last", "")
local farthest = ui.state("test_pdf_farthest", 0)
local asked = ui.state("test_pdf_asked", ";")
local opened = ui.state("test_pdf_opened", "")
local revealed = ui.state("test_pdf_revealed", "")
local dark = ui.state("test_pdf_dark", true)
local width = ui.state("test_pdf_w", 0)

local function promise(ok: boolean, value: any): any
	return {
		after = function(_, done: (any) -> (), fail: ((any) -> ())?)
			ui.after(16, function()
				if ok then
					done(value)
				elseif fail then
					fail({ message = value })
				end
			end)
		end,
	}
end

if ui.shared("platform", ""):peek() == "" then
	PdfView.open = function(path: string): any
		if string.find(path, "locked", 1, true) then
			return promise(false, "It's locked with a password.")
		end
		local pages = table.create(300)
		for i = 1, 300 do
			pages[i] = if i % 10 == 0 then { w = 792, h = 612 } else { w = 612, h = 792 }
		end
		return promise(true, { pages = pages })
	end
	PdfView.page = function(_path: string, page: number, zoom: number): any
		calls:set(calls:peek() + 1)
		last:set(`{page}@{math.floor(zoom * 100 + 0.5) / 100}`)
		asked:set(`{asked:peek()}{last:peek()};`)
		farthest:set(math.max(farthest:peek(), page))
		return promise(true, { src = "assets/composer/transcript.png" })
	end
end

ui.effect(function()
	ui.theme.follow_system(false)
	ui.theme.current:set(if dark:get() then "zeron-dark" else "zeron-light")
end)

local doc = ui.state("test_pdf_doc", {
	path = "docs/sample.pdf",
	absolute = "/work/docs/sample.pdf",
	state = "ready",
	pdf = true,
	size = 123456,
} :: any)

return ui.column {
	w_full = true,
	h_full = true,
	theme = function(): string
		return if dark:get() then "zeron-dark" else "zeron-light"
	end,
	bg = function(): any
		return ui.raw(if dark:get() then "#111113" else "#f4f4f5", "a capture's backdrop: the window's glass snapshots white")
	end,
	items = "center",
	FileViewer {
		id = "pdf-viewer",
		doc = doc,
		root = "work",
		readonly = true,
		on_open_default = function(path: string)
			opened:set(path)
		end,
		on_reveal = function(path: string)
			revealed:set(path)
		end,
		w = function(): any
			local w = width:get()
			return if w > 0 then w else "100%"
		end,
		h_full = true,
	},
}
