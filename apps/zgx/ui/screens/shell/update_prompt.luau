-- Zeron's "Check for Updates…" prompt (crates/ui shell.rs `render_update_prompt`), on About & updates' own states so the two never disagree.

local ds = require("@ds")
local about = require("@app/screens/settings/about")

local M = {}

M.shown = ui.state("update_prompt_open", false)

function M.open()
	M.shown:set(true)
	local ok = pcall(about.check)
	if not ok then
		about.doing:set("failed")
	end
end

type Buttons = { close: string, act: string?, run: (() -> ())? }

local function t(key: string): string
	return ui.t(key):get()
end

local function buttons(): Buttons
	local a, doing = about.now:get(), about.doing:get()
	local u = about.describe(a, doing)
	if doing == "checking" then
		return { close = t("common.cancel") }
	elseif (a.self_updating and a.flow == "downloading") or doing == "updating" then
		return { close = t("shell.update_prompt.hide") }
	elseif u.accent and u.action then
		return { close = t("shell.update_prompt.later"), act = u.action, run = about.update }
	elseif u.action and u.action == t("common.retry") then
		return { close = t("common.close"), act = t("common.try_again"), run = about.check }
	end
	return { close = t("shell.update_prompt.ok") }
end

M.Prompt = ui.component(function(): Node
	local function close()
		M.shown:set(false)
	end
	return ds.Dialog {
		open = M.shown,
		title = function(): string
			return about.describe(about.now:get(), about.doing:get()).title
		end,
		body = function(): string
			return about.describe(about.now:get(), about.doing:get()).line
		end,
		cancel = false,
		ui.row {
			id = "update-prompt-buttons",
			w_full = true,
			gap = "$space.2",
			justify = "end",
			items = "center",
			ds.Button {
				function(): string
					return buttons().close
				end,
				tone = "secondary",
				on_click = close,
			},
			ui.show(function(): boolean
				return buttons().act ~= nil
			end, ds.Button {
				function(): string
					return buttons().act or ""
				end,
				tone = "primary",
				on_click = function()
					local run = buttons().run
					if run then
						run()
					end
				end,
			}),
		},
	}
end)

return M
