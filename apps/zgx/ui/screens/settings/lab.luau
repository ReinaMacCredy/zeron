-- Settings → Experimental (Lab): agents' access, on Compaction's states (its tool settings).

local ds = require("@ds")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local compaction = require("@app/screens/settings/compaction")

local M = {}

M.ID = "experimental"
M.TITLE = "Experimental"

search.index(M.ID, ui.t("settings.lab.title"), ui.t("settings.lab.terms"), {
	{ key = "full_token", title = ui.t("settings.lab.full_token.label"), section = ui.t("settings.lab.access_section"), terms = ui.t("settings.lab.full_token.terms") },
})

local tools = compaction.tools
local refused = compaction.refused

-- The tool settings changed by `change`: shown now, back if refused.
local function save_tools(change: (any) -> ())
	local t = tools:peek()
	if not t or not t.settings then
		return
	end
	local next = table.clone(t.settings)
	change(next)
	local shown = table.clone(t)
	shown.settings = next
	tools:set(shown)
	compaction.engine("SetToolSettings", next):after(function(v: any)
		if v ~= nil then
			tools:set(v)
		end
		refused:set(nil)
	end, function(err: any)
		tools:set(t)
		refused:set(compaction.says(err))
	end)
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

function M.view(): Node
	visits.once("experimental", compaction.load)
	local function ready(): boolean
		return tools:get() ~= nil
	end
	return ds.Settings.Page {
		title = ui.t("settings.lab.title"),
		description = ui.t("settings.lab.description"),
		trail = ui.div { id = "lab-beta", ds.Badge { ui.t("settings.lab.beta"), tone = "accent" } },
		ui.show(function(): boolean
			return compaction.unread:get() ~= nil
		end, ds.Callout {
			tone = "warning",
			title = ui.t("settings.lab.unread"),
			body = function(): string
				return compaction.unread:get() or ""
			end,
			action = {
				label = ui.t("common.retry") :: any,
				on_click = function()
					visits.again("experimental", compaction.load)
				end,
			},
		}),
		ui.show(function(): boolean
			return not ready() and compaction.unread:get() == nil
		end, ui.text { ui.t("settings.compaction.loading"), color = "$text.secondary", text_size = "$font.size.sm" }),
		ui.show(ready, ds.Settings.Section {
			title = ui.t("settings.lab.access_section"),
			ds.Settings.Group {
				row("full_token", {
					label = ui.t("settings.lab.full_token.label"),
					description = ui.t("settings.lab.full_token.description"),
					control = compaction.engine_switch(ui.t("settings.lab.full_token.label"), function(): boolean?
						local t = tools:get()
						return if t and t.settings then t.settings.agentsUseFullToken == true else nil
					end, function(on: boolean)
						save_tools(function(next: any)
							next.agentsUseFullToken = on
						end)
					end),
				}),
			},
			ui.show(function(): boolean
				return refused:get() ~= nil
			end, ui.text {
				function(): string
					return refused:get() or ""
				end,
				id = "lab-refused",
				role = "alert",
				wrap = true,
				px = "$space.4",
				py = "$space.2",
				text_size = "$font.size.sm",
				color = "$danger",
			}),
		}),
	}
end

return M
