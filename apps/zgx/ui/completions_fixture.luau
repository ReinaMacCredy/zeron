-- ds/composer_mentions and screens/chat/completions on a plain field, the host
-- stubbed, wired as ds/composer wires them. Its `calls` notes each read,
-- search and run.

ui.theme.strict("warn")
local ds = require("@ds")
local Mentions = require("@ds/composer_mentions")
local Invocations = require("@app/screens/chat/invocations")
local references = require("@ds/references")
local files = require("@app/screens/files/api")
local Completions = require("@app/screens/chat/completions")
ds.setup()

local calls = ui.state("calls", "")
local function note(s: string)
	calls:append(s .. ";")
end

local function settled(value: any): any
	return {
		after = function(_self: any, ok: (any) -> (), _err: any)
			ok(value)
		end,
	}
end
local function refused(why: string): any
	return {
		after = function(_self: any, _ok: any, err: (any) -> ())
			err({ message = why })
		end,
	}
end
local NEVER = { after = function() end }

local reply = ui.state("reply", "rows")
local rows = ui.state("rows", {
	{ kind = "command", name = "compact", label = "/compact", detail = "Free up context · <focus>", link = "[/compact](zeron-invoke:7b7d)", plain = true },
	{ kind = "command", name = "btw", label = "/btw", detail = "Side chat", link = "/btw", plain = true, blocked = "Mock does not support compaction" },
	{ kind = "command", name = "review", label = "/review", detail = "Review the diff · <branch>", link = "[/review](zeron-invoke:7b7d)" },
	{ kind = "command", name = "doctor", label = "/doctor", detail = "Runs only in Claude Code's terminal", link = "/doctor", plain = true, terminal_only = true },
	{ kind = "skill", name = "code-review", label = "Code Review", detail = "Careful review", link = "[$code-review](zeron-invoke:7b7d)" },
} :: { any })
local invocations_stub = Invocations :: any
invocations_stub.read = function(_harness: string, target: any): any
	note(`invocations:{target.chat or target.space or "?"}`)
	local how = reply:peek()
	if how == "hold" then
		return NEVER
	elseif how == "rows" then
		return settled({ rows = rows:peek() })
	elseif how == "skills_failed" or how == "commands_failed" then
		local list = if how == "skills_failed" then "skills" else "commands"
		return settled({ rows = rows:peek(), failed = list, warning = `{list} sentence for old callers` })
	elseif how == "warning" then
		return settled({ rows = rows:peek(), warning = "Skills couldn't be listed" })
	end
	return refused(how)
end

-- Each search's answer in turn: a word it fails with, else the found files.
local script = ui.state("search_script", {} :: { string })
local found = ui.state("found", {
	{ path = "crates/ui/src/queue.rs", dir = false },
	{ path = "crates/ui", dir = true },
} :: { any })
local files_stub = files :: any
files_stub.search = function(_target: any, query: string): any
	note(`search:{query}`)
	local next = table.clone(script:peek())
	local first = table.remove(next, 1)
	script:set(next)
	if first == "hold" then
		return NEVER
	elseif first then
		return refused(first)
	end
	return settled(found:peek())
end

local draft = ui.state("draft", "")
ui.command {
	id = "encode",
	title = "Note the draft as it would be sent",
	run = function()
		note(`encoded:{Invocations.encode(references.encode("fixture", draft:peek()))}`)
	end,
}
local place = ui.state("place", "s1")
local field = ui.ref()
local completions = Completions.bind {
	key = "fixture",
	text = draft,
	agent = function(): string?
		return "claude-code"
	end,
	target = function(): any
		return { chat = place:get() }
	end,
	files = function(): any
		return { chat = place:get() }
	end,
	in_chat = function(): boolean
		return true
	end,
	run = function(name: string)
		note(`run:{name}`)
	end,
}

local Field = ui.component(function(): Node
	local m = Mentions.use {
		field = field,
		value = draft,
		sources = completions.lists,
		enabled = completions.enabled,
		empty = completions.empty,
		on_choose = completions.on_mention,
		refs = function(): string
			return "fixture"
		end,
	}
	local unit = references.unit(function(): string
		return "fixture"
	end, draft, field)
	local pill = ui.div {
		id = "pill",
		w_full = true,
		h = 60,
		ui.textarea {
			id = "field",
			ref = field,
			value = draft,
			w_full = true,
			h = 60,
			highlight = Mentions.highlight,
			active_descendant = m.descendant,
			on_caret = function(line: number, column: number, anchor: number, focus: number)
				unit.on_caret(line, column, anchor, focus)
				m.on_caret(line, column, anchor, focus)
			end,
			on_focus = function()
				m.on_focus(true)
			end,
			on_blur = function()
				m.on_focus(false)
			end,
			on_key = function(e)
				return m.keys(e) or unit.keys(e)
			end,
		},
	}
	return ui.popover(m.popover(pill))
end)

return ui.column {
	w_full = true,
	h_full = true,
	p = "$space.4",
	justify = "end",
	ui.button {
		id = "other",
		shrink = 0,
		on_click = function()
			note("other")
		end,
		"Other",
	},
	ui.div { id = "blank", grow = 1, w_full = true },
	Field {},
}
