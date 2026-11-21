local M = {}

export type Pick = { icon: string, title: string, line: string }
export type Template = {
	id: string,
	name: string,
	tag: string,
	desc: string,
	glyph: string,
	color: string,
	greeting: { string },
	picks: { Pick },
}

M.SCRATCH = "scratch"

local function t(id: string, glyph: string, color: string, picks: { string }): Template
	local out: { Pick } = {}
	for i, icon in picks do
		table.insert(out, { icon = icon, title = `bots.templates.{id}.pick{i}`, line = `bots.templates.{id}.pick{i}_line` })
	end
	return {
		id = id,
		name = `bots.templates.{id}.name`,
		tag = `bots.templates.{id}.tag`,
		desc = `bots.templates.{id}.desc`,
		glyph = glyph,
		color = color,
		greeting = { `bots.templates.{id}.hello`, `bots.templates.{id}.ask` },
		picks = out,
	}
end

M.LIST = {
	t("scratch", "bot", "violet", { "target", "list", "pen" }),
	t("releases", "target", "cyan", { "list", "git-merge", "document" }),
	t("docs", "duo-book", "green", { "search", "pen", "document" }),
	t("triage", "inbox", "blue", { "tag", "search", "list" }),
	t("upkeep", "refresh", "forest", { "git-branch", "shield-check", "archive-minimalistic" }),
	t("tests", "checklist", "yellow", { "search", "file-code", "clock-circle" }),
	t("reviews", "eye", "plum", { "pull-request", "shield-check", "list" }),
	t("incidents", "bell", "red", { "clock-circle", "document", "search" }),
} :: { Template }

function M.find(id: string?): Template
	for _, x in M.LIST do
		if x.id == id then
			return x
		end
	end
	return M.LIST[1]
end

return M
