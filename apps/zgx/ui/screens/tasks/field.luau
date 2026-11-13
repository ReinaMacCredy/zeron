local locale = require("@ds/locale")
local mentions = require("@ds/composer_mentions")
local references = require("@ds/references")
local store = require("@app/screens/tasks/store")
local chat_store = require("@app/screens/chat/store")
local Completions = require("@app/screens/chat/completions")
local Invocations = require("@app/screens/chat/invocations")

local M = {}

local TASK_LIMIT, TASK_IDLE_LIMIT, DEPENDS_MAX = 8, 5, 20
local TASK_COLUMNS = { todo = "tasks.column.todo", working = "tasks.column.working", needs = "tasks.column.needs", done = "tasks.column.done" }
local TASK_GLYPHS = { todo = "check-circle", working = "refresh", needs = "danger-triangle", done = "check" }

function M.depends(words: string): { string }
	local known = store.parts:peek().by_id
	local out: { string } = {}
	for id in string.gmatch(words, "%]%(zeron%-task:([^%)%s]+)%)") do
		if known[id] and not table.find(out, id) and #out < DEPENDS_MAX then
			table.insert(out, id)
		end
	end
	return out
end

local function task_rows(query: string): { any }
	local scoped = string.lower(string.sub(query, 1, 5)) == "task "
	local q = string.lower(string.match(if scoped then string.sub(query, 6) else query, "^%s*(.-)%s*$") or "")
	local p = store.parts:get()
	local found: { store.Task } = {}
	local function take(list: { store.Task })
		for _, t in list do
			if q == "" or string.find(string.lower(store.title_of(t)), q, 1, true) then
				table.insert(found, t)
			end
		end
	end
	take(p.open)
	local limit = TASK_IDLE_LIMIT
	if q ~= "" or scoped then
		take(p.done)
		limit = TASK_LIMIT
	end
	local out: { any } = {}
	for i, t in found do
		if i > limit then
			break
		end
		local title = store.title_of(t)
		local column = store.state(t).column
		local where = store.project_name(t.project) or locale.s("tasks.no_project")
		table.insert(out, {
			value = `task:{t.id}`,
			key = `task-{t.id}`,
			task = t.id,
			label = title,
			detail = if column == "todo" then where else `{locale.s(TASK_COLUMNS[column])} · {where}`,
			icon = TASK_GLYPHS[column],
			link = `[{(string.gsub(title, "([%[%]\\])", "\\%1"))}](zeron-task:{t.id})`,
			searched = true,
		})
	end
	return out
end

local function column_of(id: string): string?
	local t = store.parts:peek().by_id[id]
	return if t then store.state(t).column else nil
end

function M.shown(key: string, raw: string): string
	if references.round_trips(key, raw) then
		return references.decode(key, raw)
	end
	return raw
end

export type Opts = {
	key: string,
	refs: () -> string,
	text: State<string>,
	field: any,
	line: number,
	on_files: (files: { any }) -> (),
}

function M.use(opts: Opts)
	local c = Completions.bind {
		key = opts.key,
		text = opts.text,
		agent = function(): string?
			local m = chat_store.new_chat_model()
			return if m then m.harness else nil
		end,
		target = function(): Invocations.Target?
			return { home = true }
		end,
		files = function(): any
			return nil
		end,
		in_chat = function(): boolean
			return false
		end,
		run = function() end,
		task = true,
	}
	local sources = ui.computed(function(): any
		local given = table.clone(c.lists:get() or {})
		local text = opts.text:get()
		local t = mentions.token(text, #text)
		given["@"] = if t and t.trigger == "@" then task_rows(t.query) else {}
		return given
	end)
	local m = mentions.use {
		field = opts.field,
		value = opts.text,
		sources = sources,
		titles = { ["@"] = locale.s("tasks.capture.tasks") },
		empty = setmetatable({ ["@"] = locale.s("tasks.capture.what_tasks") }, { __index = c.empty }) :: any,
		enabled = c.enabled,
		on_choose = c.on_mention,
		refs = opts.refs,
	}
	local unit = references.unit(opts.refs, opts.text, opts.field)
	local function keys(e: any): boolean
		return m.keys(e) or unit.keys(e)
	end
	local function textarea(props: { [string]: any }): Node
		local own_key = props.on_key
		props.ref = opts.field
		props.value = opts.text
		props.highlight = references.highlight(opts.refs)
		props.chips = references.chips(opts.refs, opts.text, opts.line, column_of)
		props.active_descendant = m.descendant
		props.on_caret = function(line: number, column: number, anchor: number, focus: number)
			unit.on_caret(line, column, anchor, focus)
			m.on_caret(line, column, anchor, focus)
		end
		props.on_focus = function()
			m.on_focus(true)
		end
		props.on_blur = function()
			m.on_focus(false)
		end
		props.on_paste = function(e: any): boolean
			if e.files and #e.files > 0 then
				opts.on_files(e.files)
				return true
			end
			if e.text and m.paste(e.text) then
				return true
			end
			if e.text == nil then
				chat_store.paste_image():after(function(path: string?)
					if type(path) == "string" and path ~= "" then
						opts.on_files({ { name = "image.png", path = path } })
					end
				end, function() end)
				return true
			end
			return false
		end
		props.on_key = function(e: any): boolean
			if keys(e) then
				return true
			end
			return if own_key then own_key(e) else false
		end
		local grow = props.grow
		props.grow = nil
		if not grow then
			return ui.popover(m.popover(ui.textarea(props)))
		end
		local measured = ui.use_state(grow.min)
		props.line_height = opts.line
		props.h = function(): number
			return math.clamp(measured:get(), grow.min, grow.max)
		end
		return ui.div {
			relative = true,
			w_full = true,
			ui.text {
				function(): string
					return opts.text:get() .. "\u{200b}"
				end,
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				opacity = 0,
				pointer_events = "none",
				aria_hidden = true,
				wrap = true,
				text_size = props.text_size,
				weight = props.weight,
				line_height = opts.line,
				on_resize = function(size: any)
					if math.abs(measured:peek() - size.h) >= 0.5 then
						measured:set(size.h)
					end
				end,
			},
			ui.popover(m.popover(ui.textarea(props))),
		}
	end
	return { textarea = textarea, menu = m }
end

return M
