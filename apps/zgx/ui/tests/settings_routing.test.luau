-- Settings → Routing (screens/settings/routing) on ui/shell_fixture.luau with ui/settings_stub.luau's engine.

local PAGE = "#settings-page"
local NAV = "#settings-nav"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 2000)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click(`{NAV} >> role:tab "Routing"`)
	app:frames(30)
	return app
end
local function calls(app: any): string
	return table.concat(app:state("settings_stub_calls") or {}, ",")
end
local function count(app: any, what: string): number
	return select(2, string.gsub(calls(app), what, ""))
end
local function has(app: any, scope: string, words: string): boolean
	return app:tree(scope):find(words, 1, true) ~= nil
end
local function pick(app: any, button: string, item: string)
	app:click(`{PAGE} >> role:button "{button}"`)
	app:frames(10)
	expect(app:wait(`role:menuitem "{item}"`, { timeout_ms = 15000 })).to_be(true)
	app:click(`role:menuitem "{item}"`)
	app:frames(20)
end
local function again(app: any)
	app:click(`{NAV} >> role:tab "General"`)
	app:frames(20)
	app:click(`{NAV} >> role:tab "Routing"`)
	app:frames(30)
end

test("Routing: each role off at first, routed to an agent and a model at once, with its quota, multiplier and what the agents are told; kept when the page opens again", function()
	local app = open()
	expect(calls(app)).to_contain("settings.engine:GetRoutingSettings")
	expect(calls(app)).to_contain("settings.engine:GetDelegationSettings")
	for _, role in { "worker", "validator", "research", "design", "general" } do
		expect(app:has(`#routing-role-{role}`)).to_be(true)
	end
	expect(has(app, "#routing-role-worker", "Agent picks harness and model for this role")).to_be(true)
	expect(has(app, "#routing-guidance", "Nothing yet")).to_be(true)
	pick(app, "Harness for worker", "Codex")
	expect(count(app, "settings.engine:SetRoutingSettings")).to_be(1)
	expect(app:has(`{PAGE} >> role:button "Model for worker"`)).to_be(true)
	pick(app, "Model for worker", "GPT-5.5")
	expect(count(app, "settings.engine:SetRoutingSettings")).to_be(2)
	expect(has(app, "#routing-role-worker", "Codex 42%")).to_be(true)
	expect(has(app, "#routing-role-worker", "0.5x")).to_be(true)
	expect(has(app, "#routing-guidance", "worker → codex gpt-5.5")).to_be(true)
	-- Claude Code reports no quota: said where its quota would be.
	pick(app, "Harness for validator", "Claude Code")
	expect(has(app, "#routing-role-validator", "Claude: no quota data")).to_be(true)
	expect(has(app, "#routing-role-validator", "No quota data, falls back after the first limit error")).to_be(true)
	expect(app:has(`{PAGE} >> role:button "Reasoning for validator"`)).to_be(true)
	pick(app, "Harness for research", "Same as parent")
	expect(has(app, "#routing-role-research", "Parent's model")).to_be(true)
	again(app)
	expect(app:tree(`{PAGE} >> role:button "Harness for worker"`)).to_contain("Codex")
	expect(app:tree(`{PAGE} >> role:button "Model for worker"`)).to_contain("GPT-5.5")
	expect(app:tree(`{PAGE} >> role:button "Harness for research"`)).to_contain("Same as parent")
	expect(app:error()).to_be(nil)
end)

test("Routing: when out of quota, the fallback list, mid-run, native sub-agents and delegation go to the engine; a refusal puts the last back", function()
	local app = open()
	expect(app:tree(`{PAGE} >> role:button "When the routed harness is out of quota"`)).to_contain("Native sub-agent (Recommended)")
	pick(app, "When the routed harness is out of quota", "Next in fallback list")
	-- Nothing routed yet: an empty roles table must not go out as a list.
	expect(app:has("#routing-refused")).to_be(false)
	pick(app, "Add fallback", "Codex GPT-5.5")
	expect(has(app, "#routing-fallback", "Codex GPT-5.5")).to_be(true)
	pick(app, "Add fallback", "Claude Code Opus")
	app:click(`#routing-fallback >> role:button "Remove Codex GPT-5.5"`)
	app:frames(20)
	expect(has(app, "#routing-fallback", "Codex GPT-5.5")).to_be(false)
	expect(has(app, "#routing-fallback", "Claude Code Opus")).to_be(true)
	app:click(`{PAGE} >> role:radio "10 s"`)
	app:frames(20)
	pick(app, "Native sub-agents", "Always allowed")
	app:click('#routing-maxDepth >> role:button "Increase"')
	app:frames(20)
	expect(calls(app)).to_contain("settings.engine:SetDelegationSettings")
	again(app)
	expect(app:tree(`{PAGE} >> role:button "When the routed harness is out of quota"`)).to_contain("Next in fallback list")
	expect(has(app, "#routing-fallback", "Claude Code Opus")).to_be(true)
	expect(app:tree(`{PAGE} >> role:radio "10 s"`)).to_contain("checked")
	expect(app:tree(`{PAGE} >> role:button "Native sub-agents"`)).to_contain("Always allowed")
	expect(app:tree("#routing-maxDepth")).to_contain("3")
	app:command("settings_stub.refuse_next")
	pick(app, "Native sub-agents", "Off")
	expect(app:has("#routing-refused")).to_be(true)
	expect(app:tree(`{PAGE} >> role:button "Native sub-agents"`)).to_contain("Always allowed")
end)
