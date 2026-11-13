-- Glass: what floats over the page, one recipe for all of it, so a menu,
-- a dropdown, a submenu and a dragged card are the same thing.
--
-- Four kinds. A `popup` stands in a window of its own (menus, dropdowns,
-- flyouts, hover cards): gpui's menu tone, opaque, since AppKit's material
-- under anything translucent reads lighter than gpui's blur of its own page
-- and shows what is under it. A `menu` floats in the window (a dragged card,
-- a jump pill): the menus' tint over a blur of what is under it. A `surface`
-- floats in the window in the pill's tone (a session's card, gpui's
-- `surface_bg`). Each has a hairline edge, the first two the menus' deep shadow. An
-- `island` is a bar resting
-- over content (a floating action bar): the island's lighter glass and close
-- shadow, with a fainter edge. Opaque (`material`: the setting, or the
-- system's transparency off), all turn solid and blur nothing.
--
-- `Glass { kind = "menu", w = 232, … }` is a column of it. Where the
-- panel is zgx's own (`ui.popover`, `ui.submenu`: `glass.style(props, "popup")`;
-- a drag preview's row: `"menu"`) it adds the glass to its props instead. The
-- corner is the panel's (8); a caller's `rounded` keeps its own (a dragged
-- row keeps a row's).

local material = require("@self/material")

export type Kind = "menu" | "island" | "popup" | "surface"

local function frosted(): boolean
	return material.see_through()
end

local M = {}

function M.fill(props: { [any]: any }): { [any]: any }
	props.bg = function()
		return if material.see_through() then "$zeron.composer.tint" else "$zeron.composer.solid"
	end
	props.backdrop_blur = function()
		return if material.see_through() then 16 else 0
	end
	return props
end

-- Adds the glass to `props` (changed in place, and returned). (A modal
-- opening closes every popup open outside it: zgx's own, since #1419.)
function M.style(props: { [any]: any }, kind: Kind?): { [any]: any }
	if kind == "island" then
		props.rounded = props.rounded or "$zeron.island.radius"
		props.bg = function()
			return if frosted() then "$zeron.island.tint" else "$zeron.menu.solid"
		end
		props.backdrop_blur = function()
			return if frosted() then "$zeron.island.blur" else 0
		end
		props.shadow = "$zeron.island.shadow"
		props.border = 1
		props.border_color = "$border.subtle"
		return props
	end
	props.rounded = props.rounded or "$radius.lg"
	if kind == "popup" then
		props.bg = function()
			return if material.see_through() then "$zeron.popup.tint" else "$zeron.popup.solid"
		end
		props.border = 1
		props.border_color = "$border"
		props.shadow = "$zeron.menu.shadow"
		return props
	end
	if kind == "surface" then
		M.fill(props)
		props.border = 1
		props.border_color = "$border"
		-- (A shadow under a see-through surface darkens the surface; gpui draws none on frost.)
		props.shadow = function()
			return if material.see_through() then "$zeron.composer.rest" else "$zeron.composer.shadow"
		end
		return props
	end
	-- (Where popups can't blur what's under them, Windows', solid too.)
	props.bg = function()
		return if material.menus_see_through() then "$zeron.menu.tint" else "$zeron.menu.solid"
	end
	props.backdrop_blur = function()
		return if material.menus_see_through() then "$zeron.menu.blur" else 0
	end
	props.border = 1
	props.border_color = "$border"
	props.shadow = "$zeron.menu.shadow"
	return props
end

export type Props = Merge<ContainerProps, { kind: Kind?, [number]: Child }>

-- A column of glass.
function M.Panel(props: Props): Node
	local kind = props.kind
	-- (Its props without `kind`, which is the panel's, not the column's.)
	local p: Props = table.clone(props)
	p.kind = nil
	M.style(p, kind)
	return ui.column(p)
end

return M
