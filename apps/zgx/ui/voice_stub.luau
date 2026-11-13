-- Voice, answered here for a test host: Settings → Voice's Codex voice
-- rows (`voice.remote`: none of your devices streams voice) and the Voice
-- built-in turned off and on. Required by ui/settings_stub.luau;
-- tests/voice.test.luau drives it. (Dictation's own host is played by
-- tests/dictation.rs and ui/dictation_fixture.luau.)
--
--   voice_stub.builtin_off / .builtin_on   the Voice built-in's switch

local settings_host = require("@app/screens/settings/host") :: any

local function settled(value: any): any
	local p = { done = true }
	function p.after(_self: any, ok: ((any) -> ())?, _bad: ((any) -> ())?)
		if ok then
			ok(value)
		end
	end
	return p
end

local before_call = settings_host.call
settings_host.call = function(name: string, arg: any?): any
	if name == "voice.remote" then
		return settled({})
	end
	return before_call(name, arg)
end

for _, which in { "on", "off" } do
	ui.command {
		id = `voice_stub.builtin_{which}`,
		title = `Voice built-in {which}`,
		run = function()
			require("@app/screens/plugins/builtins").set("voice", which == "on")
		end,
	}
end

return {}
