-- Voice, answered here as src/voice.rs's mock answers a mock Dev (no
-- microphone or model in a test host): Settings → Voice's host calls, and
-- dictation's backend (ds/dictation) as a fake microphone that hears
-- `voice_stub_transcript` once let go. Required by ui/settings_stub.luau;
-- tests/voice.test.luau drives it.
--
-- What was asked, in order, is `voice_stub_calls`. `ui.command`s:
--   voice_stub.dictation_on   Dictation turned on, its model here
--   voice_stub.silence        the next take hears nothing
--   voice_stub.deny           the next take is refused the microphone
--   voice_stub.press / .release   the last-touched composer's mic held / let go
--   voice_stub.download_done  a download under way finishes

local settings_host = require("@app/screens/settings/host") :: any
local Dictation = require("@ds/dictation") :: any

local calls = ui.state("voice_stub_calls", {} :: { string })
local transcript = ui.state("voice_stub_transcript", "Hello from a fake microphone")
local next_take = ui.state("voice_stub_next", "speech")
local function log(entry: string)
	calls:push(entry)
end

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

local SIZE = 670479942
local model = { ready = false, cache = false, size = SIZE, downloading = false, progress = 0 }
local function publish()
	Dictation.model:set(table.clone(model))
end
publish()

local seq = 0
local function event(n: number, kind: string, text: string?)
	seq += 1
	Dictation.events:set({ n = n, seq = seq, kind = kind, text = text })
end

local sessions = 0
local live: { [number]: boolean } = {}
Dictation.backend = {
	start = function(input: string?): any
		sessions += 1
		local n = sessions
		log(`start:{if input and input ~= "" then input else "default"}`)
		local take = next_take:peek()
		next_take:set("speech")
		live[n] = true
		ui.after(0, function()
			if not live[n] then
				return
			end
			if take == "deny" then
				live[n] = nil
				event(n, "denied", "Allow Microphone access for Zeron in System Settings → Privacy & Security, then retry.")
			else
				event(n, "listening")
			end
		end)
		if take == "silence" then
			live[-n] = true
		end
		return settled(n)
	end,
	finish = function(n: number)
		log(`finish:{n}`)
		if not live[n] then
			return
		end
		live[n] = nil
		local heard = if live[-n] then "" else transcript:peek()
		live[-n] = nil
		ui.after(0, function()
			event(n, "final", heard)
		end)
	end,
	cancel = function(n: number)
		log(`cancel:{n}`)
		live[n] = nil
	end,
}

local answers: { [string]: (arg: any) -> any } = {
	["voice.model"] = function()
		return settled(table.clone(model))
	end,
	["voice.inputs"] = function()
		return settled({ devices = { { id = "mock:built-in", name = "Mock Microphone" }, { id = "mock:usb", name = "USB Microphone" } }, default = "mock:built-in" })
	end,
	["voice.remote"] = function()
		return settled({})
	end,
	["voice.remove"] = function()
		if Dictation.busy:peek() then
			return settled(nil, "busy")
		end
		model.ready, model.cache = false, false
		publish()
		return settled(table.clone(model))
	end,
}

local before_call = settings_host.call
settings_host.call = function(name: string, arg: any?): any
	local a = answers[name]
	if a then
		log(name)
		return a(arg)
	end
	return before_call(name, arg)
end
local before_invoke = settings_host.invoke
settings_host.invoke = function(name: string, arg: any?)
	if name == "voice.download" then
		log(name)
		model.downloading, model.progress = true, math.floor(SIZE / 4)
		publish()
		return
	elseif name == "voice.cancel" then
		log(name)
		model.downloading, model.progress = false, 0
		model.error = "cancelled"
		publish()
		model.error = nil
		return
	end
	before_invoke(name, arg)
end

ui.command {
	id = "voice_stub.download_done",
	title = "The model's download finishes",
	run = function()
		model.downloading, model.progress, model.ready, model.cache = false, 0, true, true
		publish()
		Dictation.enabled:set(true)
	end,
}
ui.command {
	id = "voice_stub.dictation_on",
	title = "Dictation on, its model here",
	run = function()
		model.ready, model.cache = true, true
		publish()
		Dictation.enabled:set(true)
	end,
}
ui.command {
	id = "voice_stub.silence",
	title = "The next take hears nothing",
	run = function()
		next_take:set("silence")
	end,
}
ui.command {
	id = "voice_stub.deny",
	title = "The next take is refused the microphone",
	run = function()
		next_take:set("deny")
	end,
}
ui.command {
	id = "voice_stub.press",
	title = "Hold the mic",
	run = function()
		if Dictation.last then
			Dictation.last.press("button")
		end
	end,
}
ui.command {
	id = "voice_stub.release",
	title = "Let the mic go",
	run = function()
		if Dictation.last then
			Dictation.last.release("button")
		end
	end,
}
-- The Voice built-in (Settings → Plugins), as its switch sets it.
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
