-- Settings float in a panel at most 750 tall, whatever the window: a row
-- under its list's or its page's fold is scrolled to before it's pressed,
-- typed in or pointed at, as a person does. `reachable(app)` is `app` with
-- that done for each of those; anything else goes to `app` as it is.

local M = {}

local function box(app: any, target: string): { y: number, h: number }?
	local ok, tree = pcall(app.tree, app, target)
	if not ok then
		return nil
	end
	local line = (tree :: string):match("[^\n]+")
	local y, h = (line or ""):match(" %-?[%d%.]+,(%-?[%d%.]+) [%d%.]+x([%d%.]+)")
	if not y then
		return nil
	end
	return { y = tonumber(y) :: number, h = tonumber(h) :: number }
end

-- Scrolls the list or the page so `target` stands a third of the way down it.
function M.into_view(app: any, target: string)
	local scroller = if string.find(target, "#settings-nav", 1, true) then "#settings-nav" else "#settings-scroll"
	local view, at = box(app, scroller), box(app, target)
	if not view or not at then
		return
	end
	app:wheel(scroller, { dy = at.y - (view.y + view.h / 3) })
	app:frames(5)
end

local ACTS = { click = true, double_click = true, right_click = true, hover = true, type = true, fill = true }

function M.reachable(app: any): any
	return setmetatable({}, {
		__index = function(_, name: string): any
			local real = app[name]
			if not ACTS[name] then
				return function(_self: any, ...: any): ...any
					return real(app, ...)
				end
			end
			return function(_self: any, target: string, ...: any): ...any
				local result = table.pack(pcall(real, app, target, ...))
				if result[1] then
					return table.unpack(result, 2, result.n)
				end
				if not string.find(tostring(result[2]), "scroll it into view", 1, true) then
					error(result[2], 0)
				end
				M.into_view(app, target)
				return real(app, target, ...)
			end
		end,
	})
end

return M
