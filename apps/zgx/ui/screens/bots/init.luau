local locale = require("@ds/locale")

local M = {}

local PAGE_W = 860

M.Page = ui.component(function(): Node
	return ui.scroll {
		id = "bots-page",
		w_full = true,
		h_full = true,
		items = "center",
		ui.column {
			w_full = true,
			max_w = PAGE_W,
			px = ui.raw(40, "the mockup's gallery inset"),
			pt = ui.raw(44, "the mockup's gallery inset"),
			pb = "$space.7",
			gap = "$space.2",
			ui.text { locale.s("bots.title"), role = "heading", text_size = "$font.size.xxl", line_height = ui.raw(34, "the heading's line"), weight = "$font.weight.semibold" },
			ui.text { id = "bots-blurb", locale.s("bots.gallery.blurb"), text_size = "$font.size.md", color = "$text.secondary", wrap = true },
		},
	}
end)

return M
