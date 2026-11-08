-- Settings → Usage's numbers, answered here as the host would (no engine in
-- a test host). Required by ui/settings_stub.luau; tests/usage.test.luau
-- drives it. What was asked is `usage_stub_calls` (a state, each window's
-- days); `usage_stub.fail_next` fails the next read.

local usage = require("@account/usage")

local calls = ui.state("usage_stub_calls", {} :: { number })
local fail = ui.state("usage_stub_fail", false)

local function settled(value: any, word: string?): any
	local p = { done = true }
	function p.after(_self: any, ok: ((any) -> ())?, bad: ((any) -> ())?)
		if word then
			if bad then
				bad({ message = word })
			end
		elseif ok then
			ok(value)
		end
	end
	return p
end

usage.summary = function(days: number): any
	calls:push(days)
	if fail:peek() then
		fail:set(false)
		return settled(nil, "failed")
	end
	if days == 90 then
		return settled {
			hourly = false, sessions = 0, unpriced = 0, cost = "$0.00", tokens = "0", cached = "0", uncached = "0", output = "0", savings = "$0.00",
			providers = {}, models = {}, periods = {},
		}
	end
	return settled {
		hourly = days == 1,
		sessions = 3,
		unpriced = 2,
		cost = "$12.34",
		tokens = "1.5M",
		cached = "900K",
		uncached = "400K",
		output = "200K",
		savings = "$4.20",
		providers = {
			{ harness = "claude-code", name = "Claude Code", sessions = 2, cost = "$12.00", tokens = "1.2M", cost_share = "97%", tokens_share = "80%" },
			{ harness = "codex", name = "Codex", sessions = 1, cost = "$0.34", tokens = "300K", cost_share = "3%", tokens_share = "20%" },
		},
		models = {
			{ key = "claude-code/opus", name = "opus", harness = "claude-code", cost = "$12.00", tokens = "1.2M", cost_share = "97%", tokens_share = "80%" },
			{ key = "codex/gpt", name = "gpt", harness = "codex", cost = "$0.34", tokens = "300K", cost_share = "3%", tokens_share = "20%" },
		},
		periods = {
			{ start = 1793000000, cost = "$0.00", tokens = "0", cost_n = 0, tokens_n = 0 },
			{ start = 1793086400, cost = "$12.34", tokens = "1.5M", cost_n = 12.34, tokens_n = 1500000 },
		},
	}
end

ui.command {
	id = "usage_stub.fail_next",
	title = "The next usage read fails",
	run = function()
		fail:set(true)
	end,
}

return {}
