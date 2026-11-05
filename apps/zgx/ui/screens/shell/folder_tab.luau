-- crates/ui shell/tabs.rs `folder_tab_backdrop`.

local appearance = require("@app/screens/shell/appearance")
local art_state = require("@app/screens/shell/art")
local frame = require("@app/screens/shell/frame")

local M = {}

M.FLARE = 8
local CORNER = 8
-- (crates/ui `tab_fade`, 6 down to where the page's fade ends.)
local FADE_TOP, FADE_END = 6, 88

local SHAPE = ui.layer_shader([[
fn layer(input: LayerInput) -> vec4<f32> {
	let flare = 8.0;
	let corner = 8.0;
	let foot = input.size.y - 1.0;
	var x = input.p.x;
	if (x > input.size.x * 0.5) {
		x = input.size.x - x;
	}
	var edge = 0.0;
	if (x < flare) {
		let t = 1.0 - sqrt(max(1.0 - x / flare, 0.0));
		edge = foot - t * t * flare;
	} else if (x < flare + corner) {
		let t = 1.0 - sqrt((x - flare) / corner);
		edge = t * t * corner;
	}
	let a = clamp((input.p.y - edge) * input.scale + 0.5, 0.0, 1.0);
	return vec4<f32>(input.color.rgb, input.color.a * a);
}]])

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

local function fade(top: number, foot: number): Node
	local function strength(y: number): number
		return math.clamp((FADE_END - y) / (FADE_END - FADE_TOP), 0, 1)
	end
	local alpha = strength(foot) / strength(top)
	return ui.div {
		id = "session-tab-folder-fade",
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		bg = "$zeron.window",
		mask_gradient = { angle = 180, stops = { "#000f", `#000{string.format("%x", math.floor(alpha * 15 + 0.5))}` } },
		opacity = function(): number
			return appearance.chat_tab_shade:get() * 0.55 * strength(top)
		end,
	}
end

export type Props = {
	id: string?,
	left: any,
	top: number,
	w: any,
	h: number,
	art: boolean?,
	window_x: (() -> number)?,
}

function M.Shape(props: Props): Node
	local function framed(): boolean
		return frame.framed:get()
	end
	local sheet: { [any]: any } = {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		overflow = "hidden",
		layer_shader = SHAPE,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			bg = function(): string
				return if framed() then "$zeron.inset.framed_bg" else "$zeron.inset.bg"
			end,
		},
	}
	if props.art then
		local function art(): boolean
			return frame.page_art:get() ~= nil
		end
		table.insert(sheet, ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			ui.show(art, frame.Layer {
				id = "session-tab-folder-art",
				unfaded = true,
				kind = function(): string?
					return frame.page_art:get()
				end,
				origin = function(): { x: number, y: number }
					return { x = if props.window_x then props.window_x() else 0, y = props.top }
				end,
				left = function(): number
					return if framed() then 0 else art_state.card_left:get()
				end,
			}),
		})
		table.insert(sheet, ui.show(art, fade(props.top, props.top + props.h - 1)))
	end
	return ui.div {
		id = props.id,
		absolute = true,
		left = props.left,
		top = props.top,
		w = props.w,
		h = props.h,
		pointer_events = "none",
		ui.div(sheet),
		ui.canvas {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			draw = function(c: any, w: number, h: number)
				c:stroke(outline(w, h), if framed() then "$zeron.inset.framed_edge" else "$zeron.inset.edge", 0.5)
			end,
		},
	}
end

return M
