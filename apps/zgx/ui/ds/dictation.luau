-- Dictation, as gpui's (crates/ui/src/dictation.rs and composer.rs): hold
-- the microphone or the shortcut to talk, let go to have it written into the
-- draft. Only words cross: the host records and transcribes on this
-- computer (src/voice.rs), and says how it goes on the shared
-- `dictation_events` ({ n, seq, kind, text? }).
--
--   Dictation.enabled          Settings → Voice's Dictation switch (gpui's
--                              `dictationEnabled`, off until turned on)
--   Dictation.input            the microphone chosen ("" the system's)
--   Dictation.key              the hold shortcut (gpui's `toggleDictation`, ⌘D)
--   Dictation.model            the host's speech model ({ ready, cache, size,
--                              downloading, progress, error? })
--   Dictation.available()      Voice on (Settings → Plugins), Dictation on,
--                              the model there and a recorder in this build:
--                              the mic shows, the shortcut holds
--   Dictation.session(draft)   one composer's: its phase, press / release,
--                              finish, cancel, dismiss
--   Dictation.backend          the host's start / finish / cancel (a test's
--                              fake stands in for it)
--
-- A phase: idle, requesting, listening, finalizing (active), no_speech,
-- tapped, denied, unavailable, failed (said under the composer until
-- dismissed or the next try).

local locale = require("@self/locale")

local M = {}

M.LIMIT_S = 60
M.TAP_MS = 300
M.FINALIZE_MS = 30000

M.enabled = ui.persist("dictation_enabled", false)
M.input = ui.persist("dictation_input", "")
M.key = ui.persist("dictation_key", "cmd+d")
M.model = ui.shared("voice_model", nil :: any)
M.events = ui.shared("dictation_events", nil :: any)
-- The Voice built-in (screens/plugins/builtins publishes it: ds can't require it).
M.voice_on = ui.shared("builtins.voice", true)
-- Whether a composer's dictation is live now (Settings → Voice can't remove the model then).
M.busy = ui.state("dictation_busy", false)
M.last = nil :: any

-- Whether a combination recorded elsewhere is the hold shortcut (⌘ is Ctrl off the Mac).
function M.is_key(combo: string): boolean
	local key = M.key:peek()
	if key == "" or combo == "" then
		return false
	end
	local ok, platform = pcall(function(): any
		return ui.shared("platform", ""):peek()
	end)
	return combo == key or (not (ok and platform == "macos") and combo == (string.gsub(key, "cmd", "ctrl")))
end

-- Whether a key down is the hold shortcut.
function M.matches(e: KeyEvent, combo_of: (KeyEvent) -> string?): boolean
	return M.is_key(combo_of(e) or "")
end

function M.available(): boolean
	local m = M.model:get()
	return M.voice_on:get() ~= false and M.enabled:get() == true and m ~= nil and m.ready == true and m.capable ~= false
end

local function promise_of(name: string, arg: any): any
	local ok, p = pcall(ui.call, name, arg)
	return if ok then p else nil
end

M.backend = {
	-- A session's number, once the host has one.
	start = function(input: string?): any
		return promise_of("dictation.start", { input = if input and input ~= "" then input else nil })
	end,
	finish = function(n: number)
		pcall(ui.invoke, "dictation.finish", n)
	end,
	cancel = function(n: number)
		pcall(ui.invoke, "dictation.cancel", n)
	end,
}

local ACTIVE = { requesting = true, listening = true, finalizing = true }
function M.active(phase: string): boolean
	return ACTIVE[phase] == true
end

-- What the phase says (gpui's `Phase::status`): title and line, or nil.
function M.status(phase: string, message: string?): (string?, string?)
	local function s(key: string): string
		return locale.s(`ds.composer.dictation.{key}`)
	end
	if phase == "requesting" then
		return s("ready_title"), s("ready_line")
	elseif phase == "listening" then
		return s("listening_title"), s("listening_line")
	elseif phase == "finalizing" then
		return s("finalizing_title"), s("finalizing_line")
	elseif phase == "no_speech" then
		return s("no_speech_title"), s("no_speech_line")
	elseif phase == "tapped" then
		return s("tapped_title"), s("tapped_line")
	elseif phase == "denied" or phase == "unavailable" then
		return s("unavailable_title"), message or ""
	elseif phase == "failed" then
		return s("stopped_title"), message or ""
	end
	return nil, nil
end

-- The button's name for what it does (gpui's `action_label`).
function M.action_label(phase: string): string
	local function s(key: string): string
		return locale.s(`ds.composer.dictation.{key}`)
	end
	if phase == "idle" or phase == "tapped" then
		return s("hold")
	elseif phase == "requesting" or phase == "listening" then
		return s("release")
	elseif phase == "finalizing" then
		return s("transcribing")
	end
	return s("retry")
end

export type Session = {
	phase: State<string>,
	message: State<string?>,
	dictating: State<boolean>,
	press: (source: string) -> (),
	release: (source: string) -> (),
	finish: () -> (),
	cancel: () -> (),
	dismiss: () -> (),
	toggle: () -> (),
}

-- One composer's dictation over its `draft`. `blocked()`: it can't start now
-- (the composer disabled).
function M.session(draft: State<string>, blocked: (() -> boolean)?): Session
	local phase = ui.use_state("idle")
	local message = ui.use_state(nil :: string?)
	local dictating = ui.use_state(false)
	local n: number? = nil
	local generation = 0
	local expected = ""
	local seen = 0
	local hold: { source: string, at: number? }? = nil

	local function set(p: string, why: string?)
		phase:set(p)
		message:set(why)
		local on = ACTIVE[p] == true
		if dictating:peek() ~= on then
			dictating:set(on)
		end
		if M.busy:peek() ~= on then
			M.busy:set(on)
		end
	end
	local function stop_host()
		if n then
			M.backend.cancel(n)
		end
		n = nil
	end
	local function cancel()
		generation += 1
		stop_host()
		hold = nil
		set("idle", nil)
	end
	-- (Letting go: what was said is transcribed; only the first finish ends the audio.)
	local function finish()
		local p = phase:peek()
		if p ~= "requesting" and p ~= "listening" then
			return
		end
		set("finalizing", nil)
		local mine = generation
		if n then
			M.backend.finish(n)
		end
		ui.after(M.FINALIZE_MS, function()
			if generation == mine and phase:peek() == "finalizing" then
				generation += 1
				stop_host()
				set("failed", locale.s("ds.composer.dictation.too_long"))
			end
		end)
	end
	local function complete(p: string, why: string?)
		generation += 1
		n = nil
		hold = nil
		set(p, why)
	end
	-- The words written where the draft ends, if it's still as dictation found it.
	local function land(text: string)
		local now = draft:peek()
		if now ~= expected then
			return
		end
		local words = string.gsub(text, "^%s*(.-)%s*$", "%1")
		if words == "" then
			return
		end
		local sep = if now == "" or string.find(now, "%s$") then "" else " "
		draft:set(now .. sep .. words)
		expected = draft:peek()
	end
	local function begin()
		if blocked and blocked() then
			return
		end
		if not M.available() then
			return
		end
		cancel()
		generation += 1
		local mine = generation
		expected = draft:peek()
		seen = (M.events:peek() or {}).seq or 0
		set("requesting", nil)
		local p = M.backend.start(M.input:peek())
		if p == nil then
			complete("unavailable", locale.s("ds.composer.dictation.no_host"))
			return
		end
		p:after(function(id: any)
			if generation ~= mine then
				if type(id) == "number" then
					M.backend.cancel(id)
				end
				return
			end
			n = if type(id) == "number" then id else nil
			-- (Let go before the host answered: finish now that it has.)
			if phase:peek() == "finalizing" and n then
				M.backend.finish(n)
			end
		end, function(err: any)
			if generation == mine then
				local w = if type(err) == "table" then err.message else err
				complete("failed", if type(w) == "string" and w ~= "" then w else locale.s("ds.composer.dictation.no_host"))
			end
		end)
		-- (The minute's limit: let go for them.)
		ui.after(M.LIMIT_S * 1000, function()
			if generation == mine then
				finish()
			end
		end)
	end
	ui.effect(function()
		local e = M.events:get()
		if type(e) ~= "table" or e.n ~= n or n == nil or (e.seq or 0) <= seen then
			return
		end
		seen = e.seq or 0
		local p = phase:peek()
		if e.kind == "listening" then
			if p == "requesting" then
				set("listening", nil)
			end
		elseif e.kind == "finalizing" then
			finish()
		elseif e.kind == "partial" then
			land(e.text or "")
		elseif e.kind == "final" then
			local text = e.text or ""
			if string.find(text, "%S") == nil then
				complete("no_speech", nil)
			else
				land(text)
				complete("idle", nil)
			end
		elseif e.kind == "denied" then
			complete("denied", e.text)
		elseif e.kind == "unavailable" then
			complete("unavailable", e.text or locale.s("ds.composer.dictation.no_host"))
		elseif e.kind == "failed" then
			complete("failed", e.text)
		elseif e.kind == "cancelled" then
			cancel()
		end
	end)
	-- Voice turned off (or Dictation, or the model gone) while it listens: it lets go.
	ui.effect(function()
		if not M.available() and ACTIVE[phase:peek()] then
			cancel()
		end
	end)
	-- A draft changed under it by something else: dictation lets go (gpui's `replace` refusing).
	ui.effect(function()
		local d = draft:get()
		if ACTIVE[phase:peek()] and d ~= expected then
			cancel()
		end
	end)
	local session: Session
	ui.on_cleanup(function()
		if ACTIVE[phase:peek()] then
			cancel()
		end
		if M.last == session then
			M.last = nil
		end
	end)
	local function press(source: string)
		local p = phase:peek()
		if ACTIVE[p] and hold and hold.source == source then
			return
		end
		hold = nil
		if p == "finalizing" then
			return
		elseif p == "requesting" or p == "listening" then
			hold = { source = source, at = nil }
			return
		end
		begin()
		if ACTIVE[phase:peek()] then
			hold = { source = source, at = ui.now() }
		end
	end
	local function release(source: string)
		local h = hold
		if not h or h.source ~= source then
			return
		end
		hold = nil
		local tapped = h.at ~= nil and ui.now() - h.at < M.TAP_MS
		local p = phase:peek()
		if (p == "requesting" or p == "listening") and tapped then
			generation += 1
			stop_host()
			set("tapped", nil)
		elseif p == "requesting" or p == "listening" then
			finish()
		end
	end
	local function toggle()
		local p = phase:peek()
		if p == "requesting" then
			cancel()
		elseif p == "listening" then
			finish()
		elseif p ~= "finalizing" then
			begin()
		end
	end
	session = {
		phase = phase,
		message = message,
		dictating = dictating,
		press = press,
		release = release,
		finish = finish,
		cancel = cancel,
		dismiss = function()
			if ACTIVE[phase:peek()] then
				cancel()
			else
				set("idle", nil)
			end
		end,
		toggle = toggle,
	}
	-- (The one a person last touched: a fixture drives it as a hold would.)
	local own_press = session.press
	session.press = function(source: string)
		M.last = session
		own_press(source)
	end
	M.last = M.last or session
	return session
end


return M
