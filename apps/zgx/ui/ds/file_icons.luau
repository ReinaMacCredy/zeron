-- A file's icon: the Material Icon Theme, in its own colours. Its exact
-- name first (`Cargo.toml`, `.gitignore`), then its longest compound
-- extension (`d.ts` before `ts`), then the generic file; a folder by its
-- name (`src`, `assets`), else the plain folder, drawn open while it is.
-- An icon the theme draws again for light themes is drawn twice, and the
-- theme says which shows.
--
-- `FileIcon { name = "init.luau", size = 16 }`, `FileIcon { name = "src",
-- dir = true, open = is_open }`. `FileIcon.of(name, dir)` is the icon's
-- name, `FileIcon.src(name, dir, dark)` its path, for a list that keeps
-- its own.

local map = require("@self/file_icons_map")

-- The map's tables, looked up by any name (a name it lacks is nil). (Cast:
-- the generated map's literals type as records of their own keys alone.)
local folders = map.folders :: { [string]: string }
local names = map.names :: { [string]: string }
local extensions = map.extensions :: { [string]: string }
local light = map.light :: { [string]: boolean }

local cache: { [string]: string } = {}

local function of(name: string, dir: boolean?, open: boolean?): string
	local key = if dir then (if open then "+" else "/") .. name else name
	local hit = cache[key]
	if hit then
		return hit
	end
	local lower = string.lower(name)
	local found: string?
	if dir then
		found = folders[lower] or map.folder
		if open then
			found ..= "-open"
		end
	else
		found = names[lower]
		if not found then
			-- `a.test.ts`: `test.ts`, then `ts`.
			local i = string.find(lower, ".", 2, true)
			while i and not found do
				found = extensions[string.sub(lower, i + 1)]
				i = string.find(lower, ".", i + 1, true)
			end
		end
	end
	local icon: string = found or map.file
	cache[key] = icon
	return icon
end

local function src(name: string, dir: boolean?, dark: boolean, open: boolean?): string
	local icon = of(name, dir, open)
	return `assets/icons/files/{if not dark and light[icon] then icon .. "_light" else icon}.svg`
end

-- (Read-only: a call through the module's `__call` checks its table
-- exactly otherwise.)
export type Props = {
	read name: string | () -> string,
	read dir: (boolean | () -> boolean)?,
	read open: (boolean | () -> boolean)?,
	read size: number?,
}

local function component(props: Props): Node
	local size = props.size or 16
	local name, dir, open = props.name, props.dir, props.open
	local live = type(name) == "function" or type(dir) == "function" or type(open) == "function"
	local function svg(dark: boolean, opacity: string?): Node
		local function path(): string
			local n = if type(name) == "function" then name() else name
			local d = if type(dir) == "function" then dir() else dir
			local o = if type(open) == "function" then open() else open
			return src(n, d == true, dark, o == true)
		end
		return ui.svg {
			src = if live then path else path(),
			w = size,
			h = size,
			absolute = true,
			left = 0,
			top = 0,
			opacity = opacity,
		}
	end
	return ui.div {
		w = size,
		h = size,
		shrink = 0,
		relative = true,
		-- (The name beside it says what it is.)
		aria_hidden = true,
		-- Both drawings stacked: the theme's own shows (`zeron.files.light`,
		-- 1 in light, 0 in dark), so a subtree themed apart from the window
		-- draws its own without the script knowing which it is in.
		svg(false, "$zeron.files.light"),
		svg(true, "$zeron.files.dark"),
	}
end

return setmetatable({ of = of, src = src }, {
	__call = function(_, props: Props): Node
		return component(props)
	end,
})
