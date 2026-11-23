-- Keys that reach the window with no text field holding them, on a chat or
-- a new one, go to the composer (Zeron's shell.rs `type_into_composer`,
-- `paste_into_composer`): the composer takes the keyboard and the window then
-- gives it the key's text, or the paste, as it would had it been focused. So
-- the first character isn't lost and an input method's isn't made up here.

local composer_surface = require("@ds/composer_surface")
local route = require("@app/screens/shell/route")
local lanes = require("@app/screens/chat/lanes")
local annotation_layer = require("@ds/annotation_layer")

local M = {}

local function take(): boolean
	local name = route.current:peek().name
	if name ~= "session" and name ~= "home" then
		return false
	end
	local session = route.current:peek().session
	if session and lanes.typing_takes and lanes.typing_takes(session) then
		return true
	end
	local field: any = (composer_surface :: any).field
	local el = field and field:peek()
	if not el then
		return false
	end
	el:focus()
	return true
end

M.take = take

-- A key came up through a control of the page or the panel (focus on one, not
-- parked): gpui's `typing_target` leaves those alone. Nothing focused, a key
-- reaches the window's root only.
local through_control = false
function M.through_control()
	through_control = true
	-- (Something on the way up may take the key, and the root never hears it.)
	ui.after(0, function()
		through_control = false
	end)
end

-- The window's on_key, first: a selection's pill takes its Escape and Tab
-- while it shows, with nothing holding the keyboard (ds/annotation_layer).
function M.pill_key(e: any): boolean
	if through_control or e.editing then
		return false
	end
	return annotation_layer.key(e)
end

-- Tab or Right arrow no one took, with a prediction showing: the composer
-- takes the keys with the whole prediction as its draft.
function M.prediction_key(e: any): boolean
	if through_control or e.editing or e["repeat"] then
		return false
	end
	if (e.key ~= "Tab" and e.key ~= "ArrowRight") or e.shift or e.cmd or e.ctrl or e.alt or e.meta then
		return false
	end
	local name = route.current:peek().name
	if name ~= "session" and name ~= "home" then
		return false
	end
	local fill = (composer_surface :: any).fill_prediction
	return fill ~= nil and fill() == true
end

-- The window's on_key, a key no one took. Never takes the key itself: the
-- window's text and paste follow it to the field just focused.
function M.key(e: any)
	local held = through_control
	through_control = false
	if held or e.editing or e["repeat"] then
		return
	end
	-- (`e.cmd` is Ctrl itself off macOS, so the paste chord is read from the keys held.)
	if e.key == "v" and e.meta ~= e.ctrl and not (e.shift or e.alt) then
		take()
		return
	end
	if e.cmd or e.ctrl or e.meta then
		return
	end
	if utf8.len(e.key) == 1 or e.key == "Space" then
		take()
	end
end

return M
