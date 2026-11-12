ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local sessions = require("@account/sessions") :: any
local store = require("@app/screens/chat/store") :: any
local LivePill = require("@app/screens/shell/live_pill")

local said: { string } = {}
local calls = ui.state("calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end
local function settled(value: any): any
	return {
		after = function(_self: any, ok: (any) -> (), _err: any)
			ok(value)
		end,
	}
end
store.waiting_answer = function(args: any): any
	note(`answer:{args.id}:{args.request}:{args.decision}:{table.concat(args.values or {}, "+")}`)
	return settled(nil)
end
store.send = function(req: any): any
	note(`send:{req.session}:{req.text}`)
	return settled("m1")
end
store.roster_watch = function(docs: { string })
	note(`watch:{#docs}`)
end
sessions.seen = function(id: string)
	note(`seen:{id}`)
end

return ui.column {
	w_full = true,
	h_full = true,
	items = "end",
	p = "$space.3",
	ui.show(LivePill.shown, LivePill.Button {}),
	require("@ds/toast").Host {},
}
