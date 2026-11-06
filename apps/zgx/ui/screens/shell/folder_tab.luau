-- crates/ui shell/tabs.rs `folder_tab_backdrop`: the outline. What's in it is the card's own picture, left out of the bar's shade (screens/shell/frame `BAR_SHADE`).

local frame = require("@app/screens/shell/frame")

local M = {}

M.FLARE = 8
local CORNER = 8

local function outline(w: number, h: number): any
	local f, r, j = M.FLARE, CORNER, h - 1
	return ui.path()
		:move(0, j)
		:quad(f, j, f, j - f)
		:line(f, r)
		:quad(f, 0, f + r, 0)
		:line(w - f - r, 0)
		:quad(w - f, 0, w - f, r)
		:line(w - f, j - f)
		:quad(w - f, j, w, j)
end

export type Props = {
	id: string?,
	left: any,
	top: number,
	w: any,
	h: number,
}

function M.Shape(props: Props): Node
	return ui.canvas {
		id = props.id,
		absolute = true,
		left = props.left,
		top = props.top,
		w = props.w,
		h = props.h,
		pointer_events = "none",
		draw = function(c: any, w: number, h: number)
			c:stroke(outline(w, h), if frame.framed:get() then "$zeron.inset.framed_edge" else "$zeron.inset.edge", 0.5)
		end,
	}
end

return M
