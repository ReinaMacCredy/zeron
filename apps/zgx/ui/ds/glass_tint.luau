-- The window glass's tint for how much the desktop blurs behind it
-- (ds/material; no requires, for tests to read).

local M = {}

-- From this much blur, in points, the theme's own tint reads as frosted,
-- as it does over macOS's 60 points.
M.FULL = 40
-- With no blur at all, this much of the theme's see-through is left: the
-- desktop's colour through the tint, its shapes and words not.
M.LEAST = 0.25

-- How much of the theme's see-through to keep over `blur` points of blur
-- (1: all of it; nil, unknown: the theme's own).
function M.clarity(blur: number?): number
	if type(blur) ~= "number" or blur >= M.FULL then
		return 1
	end
	return M.LEAST + (1 - M.LEAST) * math.max(blur, 0) / M.FULL
end

-- `tint` ("#rrggbbaa") keeping `keep` of its see-through; nil where it has
-- none (opaque) or isn't a colour written so.
function M.thicker(tint: string, keep: number): string?
	local rgb, a = string.match(tint, "^#(%x%x%x%x%x%x)(%x%x)$")
	if not rgb then
		return nil
	end
	local opacity = (tonumber(a, 16) :: number) / 255
	local thick = 1 - (1 - opacity) * math.clamp(keep, 0, 1)
	return string.format("#%s%02x", rgb, math.floor(thick * 255 + 0.5))
end

function M.tuned(tint: string, coverage: number, dim: number?): string?
	local r, g, b = string.match(tint, "^#(%x%x)(%x%x)(%x%x)")
	if not r then
		return nil
	end
	local alpha = if dim then 1 - (1 - coverage) * dim else coverage
	local scale = coverage / math.max(alpha, 0.001)
	if not dim then
		scale = 1
	end
	local function ch(hex: string): number
		return math.floor((tonumber(hex, 16) :: number) * scale + 0.5)
	end
	return string.format("#%02x%02x%02x%02x", ch(r), ch(g), ch(b), math.floor(alpha * 255 + 0.5))
end

return M
