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

function M.corners(frame: string?, size: number): { number }
	local c = CORNERS[frame or ""] or CORNERS.squircle
	local out = {}
	for i = 1, 4 do
		out[i] = math.floor(c[i] * size + 0.5)
	end
	return out
end

function M.hex(color: string?): string
	return HEX[color or ""] or HEX.violet
end

export type Look = { glyph: string?, frame: string?, color: string?, image: string? }

-- Decoded here: an image avatar arrives as a `data:` URL, which `ui.image` would read as a file's path.
local decoded: { [string]: any } = {}
local waiting: { [string]: { (any) -> () } } = {}
local function bitmap(url: string, got: (any) -> ())
	if decoded[url] then
		got(decoded[url])
		return
	end
	if waiting[url] then
		table.insert(waiting[url], got)
		return
	end
	local base64 = string.match(url, "^data:image/[%w.+-]+;base64,(.+)$")
	if not base64 then
		return
	end
	waiting[url] = { got }
	ui.decode_image(base64):after(function(b: any)
		decoded[url] = b
		local all = waiting[url] or {}
		waiting[url] = nil
		for _, f in all do
			f(b)
		end
	end, function()
		waiting[url] = nil
	end)
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
	local inset = math.floor(size * 0.17 + 0.5)
	local screen = size - inset * 2
	local shown = ui.use_state(nil :: any)
	ui.effect(function()
		local image = look().image
		if not image or image == "" then
			shown:set(nil)
			return
		end
		bitmap(image, function(b: any)
			if look().image == image then
				shown:set(b)
			end
		end)
	end)
	return ui.div {
		id = props.id,
		relative = true,
		w = size,
		h = size,
		shrink = 0,
		rounded_tl = corner(1),
		rounded_tr = corner(2),
		rounded_br = corner(3),
		rounded_bl = corner(4),
		bg = function(): any
			return ui.raw(M.hex(look().color), "the Bot's own colour")
		end,
		aria_hidden = true,
		ui.div {
			absolute = true,
			left = inset,
			top = inset,
			w = screen,
			h = screen,
			rounded = ui.raw(math.floor(screen * 0.24 + 0.5), "the screen's corner, a share of its size"),
			bg = ui.raw("#111111", "the avatar's screen, dark in both themes"),
			color = ui.raw("#ffffff", "the glyph, white on the screen"),
			overflow = "hidden",
			items = "center",
			justify = "center",
			ui.presence(function(): string
				return if shown:get() then look().image or "" else ""
			end, function(image: string): Node
				if image ~= "" then
					return ui.image {
						src = shown:peek(),
						w = screen,
						h = screen,
						fit = "cover",
						aria_hidden = true,
					}
				end
				return ds.Icon {
					name = function(): any
						return look().glyph or "bot"
					end,
					size = math.floor(size * 0.44 + 0.5),
				}
			end, { w = screen, h = screen, items = "center", justify = "center" }),
		},
	}
end)

return M
