-- Where a chat works, as one chip under the composer: a folder and its
-- name, opening "Working directory" (the folder with its path, and Add
-- folder…). ds/composer's `place`:
--
--   place = FolderChip.place { folder = function() return { name = …, path = … } end, on_add = add }

local ds = require("@ds")
local Icon = require("@ds/icon")
local FadeText = require("@ds/fade_text")
local Menu = require("@ds/menu")

local M = {}

export type Folder = { name: string?, path: string? }

-- The path as the person reads it: their home as "~".
function M.short(path: string?, home: string?): string
	if not path then
		return ""
	end
	if home and home ~= "" and string.sub(path, 1, #home) == home then
		return "~" .. string.sub(path, #home + 1)
	end
	return path
end

function M.place(opts: {
	folder: () -> Folder,
	on_add: () -> (),
	-- Whether "Work in"'s own rows stay under it (a new chat's device, project, checkout and branch are chosen nowhere else).
	keeps: (fixed: boolean) -> boolean,
}): any
	local function name(): string
		return opts.folder().name or ui.t("chat.folder.none"):get()
	end
	local function line(p: ds.ComposerPlace): { any }
		local natural = ui.ref()
		if p.on_measures then
			local told = p.on_measures
			ui.effect(function()
				if natural.width then
					told(natural.width, 0, 0)
				end
			end)
		end
		return {
			ui.text { name, ref = natural, absolute = true, left = 0, top = 0, opacity = 0, pointer_events = "none", aria_hidden = true },
			ui.div { id = "composer-folder-glyph", shrink = 0, w = 12, h = 12, items = "center", justify = "center", Icon { name = "folder", size = 12 } },
			ui.show(function(): boolean
				return not p.folded()
			end, FadeText { name, roll = true, id = "composer-folder", max_w = 220, shrink = 1, fade = 16 }),
		}
	end
	local function head(rest: Node?, fixed: boolean): Node
		return ui.column {
			id = "composer-folder-menu",
			w_full = true,
			Menu.Heading { ui.t("chat.folder.title"), first = true },
			ui.menu_item(Menu.look {
				id = "composer-folder-current",
				label = name,
				disabled = true,
				cursor = "default",
				hover = {},
				active = {},
				Menu.glyph("folder", "$text.secondary"),
				FadeText { name, text_size = "$font.size.md", color = "$text.secondary", shrink = 0, max_w = 180 },
				ui.div { grow = 1, min_w = "$space.2" },
				FadeText {
					function(): string
						return opts.folder().path or ""
					end,
					id = "composer-folder-path",
					text_size = "$font.size.md",
					color = "$text.secondary",
					shrink = 1,
					min_w = 0,
				},
				ui.div { shrink = 0, color = "$text.secondary", aria_hidden = true, Icon { name = "check", size = 14 } },
			}),
			Menu.Rule(),
			Menu.Row { id = "composer-folder-add", icon = "lucide-folder-plus", label = ui.t("chat.folder.add"), on_click = opts.on_add },
			if rest then ui.show(function(): boolean
				return opts.keeps(fixed)
			end, ui.column { w_full = true, Menu.Rule(), rest }) else nil,
		}
	end
	return { line = line, head = head, width = 400 }
end

return M
