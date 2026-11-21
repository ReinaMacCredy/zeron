-- The shell fixture with a stub Bot host that edits `bots.list` and records `bots_calls`.

local node = require("./shell_fixture")
local store = require("@app/screens/bots/store")

local said: { string } = {}
local calls = ui.state("bots_calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local function bot(id: string, name: string, chat: string, extra: { [string]: any }?): any
	local b: { [string]: any } = {
		id = id,
		name = name,
		chat = chat,
		avatar = { glyph = "bot", frame = "squircle", color = "blue" },
		pinned = false,
		archived = false,
		unread = false,
	}
	for k, v in extra or {} do
		b[k] = v
	end
	return b
end

local bots: { any } = {
	bot("k1", "Release keeper", "a1", { pinned = true }),
	bot("k2", "Docs writer", "a2", { pinned = true, unread = true }),
	bot("k3", "Triage desk", "b1"),
	bot("k4", "Nightly sweep", "a3", { unread = true }),
	bot("k5", "Old helper", "b2", { archived = true }),
}
local pin_order: { string } = { "k1", "k2" }

local function publish()
	local at: { [string]: number } = {}
	for i, id in pin_order do
		at[id] = i
	end
	local pinned, rest = {}, {}
	for _, b in bots do
		table.insert(if b.pinned then pinned else rest, table.clone(b))
	end
	table.sort(pinned, function(a: any, b: any): boolean
		return (at[a.id] or 0) < (at[b.id] or 0)
	end)
	for _, b in rest do
		table.insert(pinned, b)
	end
	store.list:set({ state = "ready", bots = pinned })
end

local function done(v: any): any
	return {
		after = function(_self: any, ok: ((any) -> ())?)
			if ok then
				ok(v)
			end
		end,
	}
end

store.host.update = function(args: any): any
	local parts = {}
	for _, key in { "name", "pinned", "archived", "unread" } do
		if args[key] ~= nil then
			table.insert(parts, `{key}={tostring(args[key])}`)
		end
	end
	note(`update:{args.id}:{table.concat(parts, "+")}`)
	for _, b in bots do
		if b.id == args.id then
			for _, key in { "name", "pinned", "archived", "unread" } do
				if args[key] ~= nil then
					b[key] = args[key]
				end
			end
			if args.pinned == true then
				table.insert(pin_order, b.id)
			elseif args.pinned == false then
				local i = table.find(pin_order, b.id)
				if i then
					table.remove(pin_order, i)
				end
			end
		end
	end
	publish()
	return done(nil)
end
store.host.delete = function(args: any): any
	note(`delete:{args.id}`)
	for i, b in bots do
		if b.id == args.id then
			table.remove(bots, i)
			break
		end
	end
	publish()
	return done(nil)
end

ui.command { id = "fixture.bots_none", title = "Fixture: no Bots", menu = "Fixture", run = function()
	bots = {}
	publish()
end }

publish()
return node
