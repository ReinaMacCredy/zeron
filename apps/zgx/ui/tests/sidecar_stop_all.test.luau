-- The Sidecar's Stop all (screens/sidecar, through screens/chat/roster's
-- `stoppable`): what it stops is the shipping app's `stoppable_runs`, the
-- agents the harness can stop and the shells where shells can be stopped,
-- never every task the session lists.

local Roster = require("../screens/chat/roster")

local RUNS = {
	agents = {
		{ id = "a1", state = "working", stoppable = true },
		{ id = "a2", state = "completed", stoppable = false },
		{ id = "d1", state = "working", stoppable = true, delegated = true },
	},
}

local function ids(list: { any }): string
	local out = {}
	for _, x in list do
		table.insert(out, x.id)
	end
	return table.concat(out, ",")
end

test("stop all: the agents that can stop and the shells, not a task of another kind", function()
	local can = Roster.stoppable(RUNS, {
		id = "c",
		tasks = { { id = "sh1", task_kind = "shell" }, { id = "sh2" }, { id = "ag1", task_kind = "agent" }, { id = "mon", task_kind = "monitor" } },
	})
	expect(ids(can.agents)).to_be("a1,d1")
	expect(ids(can.shells)).to_be("sh1,sh2")
end)

test("stop all: shells stay where the device can't stop them, the agents still go", function()
	local can = Roster.stoppable(RUNS, { id = "c", shells_stoppable = false, tasks = { { id = "sh1", task_kind = "shell" } } })
	expect(ids(can.agents)).to_be("a1,d1")
	expect(#can.shells).to_be(0)
end)

test("stop all: nothing that can stop, nothing to offer", function()
	local can = Roster.stoppable({ agents = { { id = "a2", state = "completed", stoppable = false } } }, { id = "c", tasks = { { id = "ag1", task_kind = "agent" } } })
	expect(#can.agents + #can.shells).to_be(0)
end)
