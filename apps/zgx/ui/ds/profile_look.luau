-- What an agent profile looks like and says wherever it shows (the model
-- chip's profile half, the profiles card and its edit page, Settings →
-- Agent profiles), as Zeron's do (crates/ui/src/agent_profiles.rs and
-- pickers/profiles.rs): its brand's badge, the line under its name, the line
-- saying how a chat has drifted from it, and a long field's token count.
--
-- A profile, as the host lists it (src/agent_profiles.rs `profile_value`):
-- `{ id, name, harness, harness_label, mark, icon?, model?, model_label?,
-- effort?, effort_label?, access?, options, notes, instructions, tools_off,
-- by_agent }`. A chat's against it (`applied`): `{ id, name, harness, mark,
-- icon?, edited, savable, drift = { { kind, from?, to?, names? } } }`.

local AgentMark = require("@self/agent_mark")
local locale = require("@self/locale")

local M = {}

export type Profile = {
	id: string,
	name: string,
	harness: string,
	harness_label: string?,
	mark: string?,
	icon: string?,
	model: string?,
	model_label: string?,
	effort: string?,
	effort_label: string?,
	access: string?,
	options: { [string]: string }?,
	notes: string?,
	instructions: string?,
	tools_off: { string }?,
	by_agent: boolean?,
}
export type Change = { kind: string, from: string?, to: string?, names: { string }? }
export type Applied = {
	id: string,
	name: string,
	harness: string,
	mark: string?,
	icon: string?,
	edited: boolean,
	savable: boolean,
	drift: { Change },
}
export type List = { state: string, error: string?, profiles: { Profile } }
-- What a profile is made from (and saved as): a profile's fields, without its
-- id and marks.
export type Draft = {
	name: string,
	harness: string,
	model: string?,
	effort: string?,
	access: string?,
	options: { [string]: string },
	notes: string,
	instructions: string,
	tools_off: { string },
}

-- gpui's `PROFILE_NAME_MAX`: the chip's name, held to it.
M.NAME_MAX = 96
-- The engine's limits on a profile's long fields, in characters
-- (crates/engine/src/agent_profiles.rs).
M.NOTES_MAX = 2000
M.INSTRUCTIONS_MAX = 16000

-- An agent's mark on its quiet tile (gpui's `brand_badge`: the tile's corner
-- 28% of its side, the glyph 58%).
function M.badge(p: { harness: string, mark: string?, icon: string?, harness_label: string? }, size: number): Node
	local glyph = math.floor(size * 0.58 + 0.5)
	return ui.column {
		w = size,
		h = size,
		shrink = 0,
		rounded = ui.raw(math.floor(size * 0.28 + 0.5), "gpui's brand badge: its corner 28% of its side"),
		items = "center",
		justify = "center",
		bg = "$zeron.secondary.bg",
		aria_hidden = true,
		AgentMark { label = p.harness_label or p.harness, mark = p.mark, icon = p.icon, seed = p.harness, size = glyph, bare = true },
	}
end

local function access_word(id: string?): string?
	if id == nil then
		return nil
	end
	return locale.s(`ds.composer.access.{id}`)
end
M.access_word = access_word

-- The modes an agent takes (gpui's `offered_access_modes`): every one while
-- it isn't listed, Full access alone when its catalog names none.
function M.modes(agent: { access_modes: { string }? }?): { string }
	if agent == nil or agent.access_modes == nil then
		return { "supervised", "edits", "full" }
	end
	return if #agent.access_modes > 0 then agent.access_modes else { "full" }
end

-- "Codex · GPT-5.6 · Supervised · X-High · 2 tools off" (gpui's `profile_line`):
-- only what the profile sets.
function M.line(p: Profile | Draft, harness_label: string?, model_label: string?): string
	local parts: { string } = { harness_label or (p :: any).harness_label or p.harness }
	local model = model_label or (p :: any).model_label or p.model
	if model then
		table.insert(parts, model)
	end
	local mode = access_word(p.access)
	if mode then
		table.insert(parts, mode)
	end
	local effort = (p :: any).effort_label or p.effort
	if effort then
		table.insert(parts, effort)
	end
	local off = #(p.tools_off or {})
	if off > 0 then
		table.insert(parts, locale.s("ds.profiles.tools_off", { n = off }))
	end
	return table.concat(parts, " · ")
end

-- gpui's `draft_summary`: the name first, when it has one.
function M.draft_line(d: Draft, harness_label: string?, model_label: string?): string
	local line = M.line(d, harness_label, model_label)
	local name = string.match(d.name or "", "^%s*(.-)%s*$") or ""
	return if name == "" then line else `{name} · {line}`
end

-- How a chat has moved from its profile, each change its own words
-- (gpui's `drift_line`).
function M.drift_line(drift: { Change }): string
	local parts: { string } = {}
	for _, c in drift do
		if c.kind == "effort" then
			table.insert(parts, locale.s("ds.profiles.drift_effort", { from = c.from or "", to = c.to or "" }))
		elseif c.kind == "mode" then
			table.insert(parts, `{access_word(c.from) or ""} → {access_word(c.to) or ""}`)
		elseif c.kind == "options" then
			table.insert(parts, locale.s("ds.profiles.drift_options", { names = table.concat(c.names or {}, ", ") }))
		elseif c.kind == "tools" then
			table.insert(parts, locale.s("ds.profiles.drift_tools"))
		elseif c.kind == "instructions" then
			table.insert(parts, locale.s("ds.profiles.drift_instructions"))
		else
			table.insert(parts, `{c.from or ""} → {c.to or ""}`)
		end
	end
	return table.concat(parts, " · ")
end

-- gpui's `format_tokens` (crates/proto/src/usage.rs) up to millions.
local function tokens(n: number): string
	local function trim(s: string): string
		if string.find(s, ".", 1, true) then
			s = string.gsub(string.gsub(s, "0+$", ""), "%.$", "")
		end
		return s
	end
	if n >= 999500 then
		return trim(string.format("%.2f", n / 1e6)) .. "M"
	elseif n >= 1e5 then
		return string.format("%.0fK", n / 1e3)
	elseif n >= 1e3 then
		return trim(string.format("%.1f", n / 1e3)) .. "K"
	end
	return tostring(n)
end

-- "~N / M tokens" under a long field opened out, at the engine's four
-- characters a token.
function M.token_count(text: string, limit: number): string
	local chars = utf8.len(text) or #text
	return locale.s("ds.profiles.token_count", { n = tokens(math.ceil(chars / 4)), max = tokens(math.ceil(limit / 4)) })
end

-- gpui's `filter_profiles`: name and notes, any case.
function M.matches(p: Profile, query: string): boolean
	local q = string.lower(string.match(query, "^%s*(.-)%s*$") or "")
	if q == "" then
		return true
	end
	return string.find(string.lower(p.name), q, 1, true) ~= nil or string.find(string.lower(p.notes or ""), q, 1, true) ~= nil
end

-- A profile's draft, to edit it (gpui's `draft`).
function M.draft_of(p: Profile): Draft
	return {
		name = p.name,
		harness = p.harness,
		model = p.model,
		effort = p.effort,
		access = p.access,
		options = table.clone(p.options or {}),
		notes = p.notes or "",
		instructions = p.instructions or "",
		tools_off = table.clone(p.tools_off or {}),
	}
end

function M.blank(harness: string): Draft
	return { name = "", harness = harness, options = {}, notes = "", instructions = "", tools_off = {} }
end

return M
