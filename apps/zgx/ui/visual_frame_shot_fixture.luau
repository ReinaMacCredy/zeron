-- (Screenshot only: a reply with a framed visual, its caption, and a visual
-- still being built, as src/chat.rs lays them out, drawn by ds/transcript
-- on the real host: `zeron-zgx fixture=visual_frame_shot_fixture.luau --inspect`.)

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local T = require("@ds/transcript")

local SCRIPT = [[
local s = ui.state { metric = "p95" }
local days = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" }
local data = {
  p50 = { 88, 90, 112, 118, 116, 110, 114 },
  p95 = { 180, 183, 240, 251, 246, 238, 243 },
  p99 = { 236, 244, 355, 372, 364, 349, 358 },
}
return function()
  return ui.column { gap = 12,
    ui.tabs { options = { "p50", "p95", "p99" }, value = s.metric, on_change = function(v) s.metric = v end },
    ui.chart { type = "line", height = 200, x = days, unit = "ms",
      series = { { name = s.metric, values = data[s.metric] } } },
  }
end
]]

ui.command { id = "theme_light", title = "Light theme", run = function()
	ui.theme.follow_system(false)
	ui.theme.current:set("zeron-light")
end }
ui.command { id = "theme_dark", title = "Dark theme", run = function()
	ui.theme.follow_system(false)
	ui.theme.current:set("zeron-dark")
end }

local messages = ui.shared("transcript_messages", {
	{ id = "u1", role = "user", text = "Why did checkout p95 go up this week?" },
	{
		id = "a1",
		role = "assistant",
		text = "",
		streaming = true,
		blocks = {
			{ id = "t1", kind = "text", text = "p95 rose from Wednesday, the day the cart cache TTL went down:" },
			{
				id = "v:shot",
				kind = "visual",
				title = "Latency by day",
				source = SCRIPT,
				caption = "Source: metrics/checkout.csv, the last 7 days. Values in ms.",
			},
			{ id = "t2", kind = "text", text = "Here is a model of the TTL to try:" },
			{ id = "s2", kind = "visual_pending", title = "Cart cache TTL" },
		},
	},
} :: { any })

return ui.component(function(): Node
	return ui.column {
		id = "conversation",
		w_full = true,
		h_full = true,
		bg = "$background",
		T.Transcript { messages = messages },
	}
end)({})
