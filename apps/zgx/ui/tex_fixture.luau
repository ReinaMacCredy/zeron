ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local FileViewer = require("@ds/file_viewer")
local PdfView = require("@ds/pdf_view")
local TexView = require("@ds/tex_view")

local compiles = ui.state("test_tex_compiles", 0)
local synced = ui.state("test_tex_synced", "")
local revealed = ui.state("test_tex_revealed", 0)
local notes = ui.state("test_tex_notes", {} :: { any })
local result = ui.state("test_tex_result", {
	state = "failed",
	pdf = "/cache/view-1.pdf",
	root = "/work/cv.tex",
	problems = { { message = "Undefined control sequence.", line = 3, file = "/work/cv.tex" } },
} :: any)

local function promise(value: any): any
	return {
		after = function(_, done: (any) -> (), _fail: ((any) -> ())?)
			ui.after(16, function()
				done(value)
			end)
		end,
	}
end

TexView.compile = function(_path: string): any
	compiles:set(compiles:peek() + 1)
	return promise(result:peek())
end
TexView.sync = function(at: any): any
	synced:set(`{at.page}:{math.floor(at.x)}:{math.floor(at.y)}`)
	return promise({ file = "/work/cv.tex", line = 5, end_line = 6, text = "Hello world\nagain" })
end
PdfView.open = function(_path: string): any
	return promise({ pages = { { w = 612, h = 792 } } })
end
PdfView.page = function(_path: string, _page: number, _zoom: number): any
	return promise({ src = "assets/composer/transcript.png" })
end

local function change(id: string, edit: (n: any) -> ())
	local all = table.clone(notes:peek())
	for i, n in all do
		if n.id == id then
			local next_n = table.clone(n)
			edit(next_n)
			all[i] = next_n
		end
	end
	notes:set(all)
end
local function annotate(source: string): any
	return {
		list = function(): { any }
			local out: { any } = {}
			for _, n in notes:get() do
				if n.source == source then
					table.insert(out, n)
				end
			end
			return out
		end,
		add = function(text: string, anchor: any): string
			local all = table.clone(notes:peek())
			local id = `note-{#all + 1}`
			table.insert(all, { n = #all + 1, id = id, text = text, comment = "", source = source, anchor = anchor })
			notes:set(all)
			return id
		end,
		comment = function(id: string, text: string, _focus: boolean)
			change(id, function(n: any)
				n.comment = text
			end)
		end,
		remove = function(id: string)
			local all: { any } = {}
			for _, n in notes:peek() do
				if n.id ~= id then
					table.insert(all, n)
				end
			end
			notes:set(all)
		end,
		edit = function(): string?
			return nil
		end,
		edited = function() end,
	}
end

local reveal = ui.state("test_tex_reveal", nil :: any)
ui.effect(function()
	local at = reveal:get()
	if type(at) == "number" then
		revealed:set(at)
	end
end)

local doc = ui.state("test_tex_doc", {
	path = "cv.tex",
	absolute = "/work/cv.tex",
	state = "ready",
	tex = true,
	text = "\\documentclass{article}\n\\begin{document}\n\\oops\n\nHello world\nagain\n\\end{document}\n",
	size = 96,
} :: any)

return ui.column {
	w_full = true,
	h_full = true,
	FileViewer {
		id = "tex-viewer",
		doc = doc,
		root = "work",
		on_save = function() end,
		annotate = annotate,
		reveal_line = reveal,
		w_full = true,
		h_full = true,
	},
}
