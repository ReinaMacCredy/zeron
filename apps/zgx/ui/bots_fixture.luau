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

local made = 0
store.host.create = function(args: any): any
	made += 1
	local id, chat = `n{made}`, `c{made}`
	local a = args.avatar or {}
	note(`create:{args.name}:{tostring(args.template)}:{tostring(a.glyph)}/{tostring(a.frame)}/{tostring(a.color)}/{if a.image then "image" else "none"}`)
	table.insert(bots, 1, bot(id, args.name, chat, { avatar = a }))
	publish()
	return done({ id = id, chat = chat })
end
store.host.image = function(args: any): any
	local name = string.match(args.path, "[^/\\]+$") or args.path
	note(`image:{name}`)
	if string.find(name, "big", 1, true) then
		return {
			after = function(_self: any, _ok: ((any) -> ())?, no: ((any) -> ())?)
				if no then
					no("too_large")
				end
			end,
		}
	end
	-- (An 8 by 8 red JPEG.)
	return done("data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDABQODxIPDRQSEBIXFRQYHjIhHhwcHj0sLiQySUBMS0dARkVQWnNiUFVtVkVGZIhlbXd7gYKBTmCNl4x9lnN+gXz/2wBDARUXFx4aHjshITt8U0ZTfHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHz/wAARCAAIAAgDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwCrRRRXMe2f/9k=")
end

ui.command { id = "fixture.bots_none", title = "Fixture: no Bots", menu = "Fixture", run = function()
	bots = {}
	publish()
end }

publish()
return node
