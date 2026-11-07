local locale = require("@ds/locale")

local M = {}

export type Row = {
	id: string,
	chat: string?,
	doc: string,
	title: string,
	tag: string?,
	mark: string?,
	provider: string,
	icon: string?,
	reasoning: string?,
	state: "working" | "completed" | "stopped" | "failed",
	started: number?,
	delegated: boolean?,
	child: string?,
	asking: boolean?,
	depth: number,
	stoppable: boolean?,
	frozen: boolean?,
	tokens: number?,
}

local SPAWN = { running = "working", done = "completed", failed = "failed", stopped = "stopped" }
local DELEGATION = { running = "working", completed = "completed", cancelled = "stopped", interrupted = "stopped", failed = "failed" }
local HARNESS_NAMES = { ["claude-code"] = "Claude Code", codex = "Codex", grok = "Grok", cursor = "Cursor", mock = "Mock", opencode = "OpenCode" }

local function name_of(harness: string?): string?
	return if harness then HARNESS_NAMES[harness] or harness else nil
end

local function spawn_row(sub: any, depth: number, input: any): Row?
	local state = SPAWN[sub.status or ""]
	if not state then
		return nil
	end
	local usage = (input.usage or {})[sub.ref]
	local mark = sub.agent or input.agent
	return {
		id = sub.call or sub.ref,
		chat = input.chat,
		doc = sub.ref,
		title = sub.title or locale.s("chat.lanes.subagent"),
		tag = sub.model or sub.kind or name_of(input.harness),
		mark = mark,
		provider = mark or input.harness or "agent",
		icon = if sub.agent then nil else input.icon,
		reasoning = sub.reasoning or input.reasoning,
		state = state :: any,
		started = sub.at,
		depth = depth,
		stoppable = input.harness == "claude-code" and state == "working" and sub.call ~= nil,
		frozen = state ~= "working",
		tokens = if usage then usage.tokens else nil,
	}
end

local function with_nested(sub: any, depth: number, input: any, out: { Row }, top: boolean): boolean
	local row = spawn_row(sub, depth, input)
	if not row then
		return false
	end
	local nested: { Row } = {}
	local usage = (input.usage or {})[sub.ref]
	for _, inner in (if usage then usage.spawns else nil) or {} do
		with_nested(inner, depth + 1, input, nested, false)
	end
	local busy = row.state == "working"
	for _, n in nested do
		busy = busy or n.state == "working"
	end
	if top and not busy and not sub.recent then
		return false
	end
	table.insert(out, row)
	for _, n in nested do
		table.insert(out, n)
	end
	return true
end

-- The shipping app's `stoppable_runs`.
function M.stoppable(runs: any, session: any): { agents: { any }, shells: { any } }
	local agents: { any } = {}
	for _, r in (if runs then runs.agents else nil) or {} do
		if r.stoppable then
			table.insert(agents, r)
		end
	end
	local shells: { any } = {}
	if session and session.shells_stoppable ~= false then
		for _, t in session.tasks or {} do
			if t.task_kind == nil or t.task_kind == "shell" then
				table.insert(shells, t)
			end
		end
	end
	return { agents = agents, shells = shells }
end

function M.runs(input: any): { agents: { Row }, children: { Row }, listed: { Row } }
	local agents: { Row } = {}
	local docs: { [string]: boolean } = {}
	local turn_at = input.turn_at or 0
	for _, sub in input.subs or {} do
		docs[sub.ref] = true
		with_nested(sub, 0, input, agents, true)
	end
	local children: { Row } = {}
	for _, c in input.children or {} do
		if docs[c.id] then
			continue
		end
		local asking = c.status == "input" or c.status == "awaitingInput"
		local moving = asking or c.status == "working"
		if c.delegation then
			local state = DELEGATION[c.delegation] or (if moving then "working" else "completed")
			local started = c.delegated_at or c.created
			if state == "working" or (started or 0) >= turn_at then
				table.insert(agents, {
					id = c.id,
					chat = input.chat,
					doc = c.id,
					title = c.title,
					tag = c.model or name_of(c.harness),
					mark = c.mark or c.agent,
					provider = c.mark or c.agent or c.harness or "agent",
					icon = c.agent_icon,
					reasoning = c.reasoning,
					state = state :: any,
					started = started,
					delegated = true,
					asking = asking,
					depth = 0,
					stoppable = state == "working",
					frozen = state ~= "working",
					tokens = ((input.usage or {})[c.id] or {} :: any).tokens,
				})
			end
		elseif moving or ((c.created or 0) >= turn_at and c.last_message ~= nil) then
			table.insert(children, {
				id = c.id,
				chat = input.chat,
				doc = c.id,
				title = c.title,
				tag = locale.s(`ds.pills.kind.{c.kind or "agent"}`),
				mark = c.mark or c.agent,
				provider = c.mark or c.agent or c.harness or "agent",
				icon = c.agent_icon,
				state = (if moving then "working" elseif c.status == "failed" then "failed" else "completed") :: any,
				started = c.created,
				child = c.kind or "agent",
				asking = asking,
				depth = 0,
			})
		end
	end
	local listed: { Row } = {}
	for _, r in agents do
		table.insert(listed, r)
	end
	for _, r in children do
		table.insert(listed, r)
	end
	return { agents = agents, children = children, listed = listed }
end

function M.working(rows: { Row }): number
	local n = 0
	for _, r in rows do
		if r.state == "working" then
			n += 1
		end
	end
	return n
end

function M.lines(rows: { Row }): { any }
	local families: { { Row } } = {}
	for _, r in rows do
		if r.depth == 0 or #families == 0 then
			table.insert(families, { r })
		else
			table.insert(families[#families], r)
		end
	end
	local out: { any } = {}
	local previous: { Row } = {}
	for _, family in families do
		if M.working(family) > 0 then
			for _, r in family do
				table.insert(out, { kind = "run", run = r, key = r.id })
			end
		else
			for _, r in family do
				table.insert(previous, r)
			end
		end
	end
	if #previous > 0 then
		table.insert(out, { kind = "previous", key = "previous", n = #previous })
		for _, r in previous do
			table.insert(out, { kind = "run", run = r, key = r.id })
		end
	end
	return out
end

function M.asking(rows: { Row }): number
	local n = 0
	for _, r in rows do
		if r.asking then
			n += 1
		end
	end
	return n
end

function M.tiles(rows: { Row }): { any }
	local at: { Row } = {}
	for _, r in rows do
		if r.state == "working" then
			table.insert(at, r)
		end
	end
	table.sort(at, function(a: Row, b: Row): boolean
		if (a.asking == true) ~= (b.asking == true) then
			return a.asking == true
		end
		return (a.started or 0) > (b.started or 0)
	end)
	local out: { any } = {}
	local by: { [string]: any } = {}
	for _, r in at do
		local key = r.provider
		local tile = by[key]
		if tile then
			tile.count += 1
			tile.asking = tile.asking or r.asking == true
		else
			tile = { key = key, mark = r.mark, icon = r.icon, asking = r.asking == true, count = 1 }
			by[key] = tile
			table.insert(out, tile)
		end
	end
	return out
end

function M.fan_label(count: number, asking: number): string
	if asking > 0 then
		return locale.s("ds.pills.fan_asking", { n = count, k = asking })
	end
	return locale.s("ds.pills.fan", { n = count })
end

function M.label(rows: { Row }): (string?, string)
	local working = M.working(rows)
	if #rows == 1 then
		return rows[1].title, "· " .. locale.s(`ds.pills.state.{rows[1].state}`)
	end
	return nil, locale.s("ds.pills.working", { n = working })
end

function M.done(rows: { Row }): string?
	local n = 0
	for _, r in rows do
		if r.state == "completed" then
			n += 1
		end
	end
	return if #rows > 1 and n > 0 then locale.s("ds.pills.done", { n = n }) else nil
end

function M.heading(rows: { Row }): string
	local working = M.working(rows)
	if working == 0 then
		return locale.s("ds.pills.subagents")
	end
	return locale.s("ds.pills.heading", { n = working, total = #rows })
end

function M.state_text(r: Row, now: number): string
	if r.state == "working" then
		if r.asking then
			return locale.s("ds.pills.asking_one")
		end
		return if r.started then M.elapsed(now - r.started) else locale.s("ds.pills.state.working")
	end
	return locale.s(`ds.pills.row.{r.state}`)
end

function M.elapsed(secs: number): string
	local s = math.max(0, math.floor(secs))
	if s < 60 then
		return `{s}s`
	end
	local mins = math.floor(s / 60)
	if mins < 60 then
		return string.format("%dm %02ds", mins, s % 60)
	end
	return string.format("%dh %02dm", math.floor(mins / 60), mins % 60)
end

function M.compact(n: number): string
	if n >= 1000000 then
		return string.format("%.2fM", n / 1000000)
	elseif n >= 1000 then
		return string.format("%.1fk", n / 1000)
	end
	return tostring(math.floor(n))
end

return M
