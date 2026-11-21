local ds = require("@ds")

local M = {}

M.FRAMES = { "squircle", "circle", "square", "arch", "bowl", "leaf" }
local CORNERS: { [string]: { number } } = {
	squircle = { 0.28, 0.28, 0.28, 0.28 },
	circle = { 0.5, 0.5, 0.5, 0.5 },
	square = { 0.1, 0.1, 0.1, 0.1 },
	arch = { 0.4, 0.4, 0.16, 0.16 },
	bowl = { 0.16, 0.16, 0.4, 0.4 },
	leaf = { 0.36, 0.12, 0.36, 0.12 },
}

M.COLORS = { "red", "yellow", "green", "forest", "cyan", "blue", "violet", "pink", "plum", "rust", "gray", "graphite" }
local HEX: { [string]: string } = {
	red = "#ef4335",
	yellow = "#e6bf2e",
	green = "#3fbf5f",
	forest = "#1f6b57",
	cyan = "#55c7d6",
	blue = "#1f6feb",
	violet = "#8b83e6",
	pink = "#d955d6",
	plum = "#b0126f",
	rust = "#a54a36",
	gray = "#8e8e8e",
	graphite = "#454545",
}

M.GLYPHS = {
	"bot", "search", "pen", "tasks", "inbox", "bell", "duo-book", "checklist", "star", "laptop",
	"refresh", "clock-circle", "tuning", "widget", "git-merge", "magic-stick", "shield-check", "file-code", "lightbulb-minimalistic", "target",
	"fast-tier", "list", "git-branch", "usage-chart", "smartphone", "leaf", "folder", "eye", "calendar", "flag",
	"user-rounded", "tag", "archive-minimalistic", "document", "gallery", "settings", "pull-request", "home", "terminal", "check-circle",
}

function M.hex(color: string?): string
	return HEX[color or ""] or HEX.violet
end

export type Look = { glyph: string?, frame: string?, color: string?, image: string? }

local function rgb(hex: string): (number, number, number)
	return tonumber(string.sub(hex, 2, 3), 16) or 0, tonumber(string.sub(hex, 4, 5), 16) or 0, tonumber(string.sub(hex, 6, 7), 16) or 0
end

local function mix(a: string, b: string, t: number): string
	local r1, g1, b1 = rgb(a)
	local r2, g2, b2 = rgb(b)
	local function m(x: number, y: number): number
		return math.floor(x + (y - x) * t + 0.5)
	end
	return string.format("#%02x%02x%02x", m(r1, r2), m(g1, g2), m(b1, b2))
end

local function light(hex: string): boolean
	local r, g, b = rgb(hex)
	return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 > 0.56
end

M.Avatar = ui.component(function(props: { look: Look | (() -> Look), size: number, id: string? }): Node
	local size = props.size
	local function look(): Look
		local l = props.look
		return if type(l) == "function" then l() else l
	end
	local function corner(at: number): () -> any
		return function(): number
			local c = CORNERS[look().frame or ""] or CORNERS.squircle
			return ui.raw(math.floor(c[at] * size + 0.5), "the frame's corner, a share of its size")
		end
	end
	return ui.div {
		id = props.id,
		w = size,
		h = size,
		shrink = 0,
		rounded_tl = corner(1),
		rounded_tr = corner(2),
		rounded_br = corner(3),
		rounded_bl = corner(4),
		overflow = "hidden",
		gradient = function(): any
			local c = M.hex(look().color)
			return { angle = 180, stops = { mix(c, "#ffffff", 0.22), c, mix(c, "#000000", 0.12) } }
		end,
		color = function(): any
			return ui.raw(if light(M.hex(look().color)) then "#1b1b1f" else "#ffffff", "the glyph, whichever reads on the Bot's colour")
		end,
		aria_hidden = true,
		ui.presence(function(): string
			local image = look().image
			return if image and image ~= "" then "image" else "glyph"
		end, function(kind: string): Node
			if kind == "image" then
				return ui.image {
					src = function(): string
						return look().image or ""
					end,
					w = size,
					h = size,
					fit = "cover",
					aria_hidden = true,
				}
			end
			return ui.div {
				w = size,
				h = size,
				items = "center",
				justify = "center",
				ds.Icon {
					name = function(): any
						return look().glyph or "bot"
					end,
					size = math.floor(size * 0.54 + 0.5),
				},
			}
		end, { w = size, h = size }),
	}
end)

return M
