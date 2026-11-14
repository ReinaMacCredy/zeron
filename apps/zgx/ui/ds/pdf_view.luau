local locale = require("@self/locale")

local M = {}

M.open = function(path: string): any
	return ui.call("pdf.open", { path = path })
end
M.page = function(path: string, page: number, zoom: number): any
	return ui.call("pdf.page", { path = path, page = page, zoom = zoom })
end

local GAP = 16
local STEPS = { 0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 1, 1.25, 1.5, 2, 3, 4 }
local MIN_ZOOM, MAX_ZOOM = 0.1, 4
local SETTLE = 160
local WAIT = 60

export type Size = { w: number, h: number }
export type Box = { x: number, y: number, w: number, h: number }
export type Pick = { page: number, x: number, y: number, x2: number, y2: number, box: Box }
export type Props = {
	path: string | () -> string,
	label: string,
	zoom_label: State<string>,
	page_label: State<string>,
	commands: { [string]: () -> () },
	failed: (reason: string, again: () -> ()) -> Node,
	offset: State<number>?,
	on_pick: ((pick: Pick) -> ())?,
	page_layer: ((page: number, holder: Ref) -> Node?)?,
}

local function path_of(props: Props): string
	local p = props.path
	return if type(p) == "function" then p() else p
end

local function same(a: { Size }, b: { Size }): boolean
	if #a ~= #b then
		return false
	end
	for i, p in a do
		if p.w ~= b[i].w or p.h ~= b[i].h then
			return false
		end
	end
	return true
end

local function reason(err: any): string
	local why = if type(err) == "table" then err.message or err.error elseif err ~= nil then tostring(err) else nil
	return if type(why) == "string" and why ~= "" then why else ""
end

local function pages_view(props: Props, list: { Size }, version: State<number>): Node
	local n = #list
	local widest = 1
	for _, p in list do
		widest = math.max(widest, p.w)
	end
	local view = ui.ref()
	local zoom = ui.use_state(1)
	local drawn = ui.use_state(0)
	local offset = props.offset or ui.use_state(0)
	local pan = ui.use_state(0)
	local width = ui.use_state(0)
	local fitted = true
	local settling: (() -> ())? = nil

	local function row_height(i: number, z: number): number
		return GAP + list[i].h * z + (if i == n then GAP else 0)
	end
	local tops_at, tops = -1, {} :: { number }
	local function tops_for(z: number): { number }
		if z ~= tops_at then
			local y = 0
			tops = table.create(n)
			for i = 1, n do
				tops[i] = y
				y += row_height(i, z)
			end
			tops_at = z
		end
		return tops
	end
	local function row_at(y: number, z: number): number
		local t = tops_for(z)
		local lo, hi = 1, n
		while lo < hi do
			local mid = (lo + hi + 1) // 2
			if t[mid] <= y then
				lo = mid
			else
				hi = mid - 1
			end
		end
		return lo
	end
	local function room(): number
		return width:peek()
	end
	local function content_w(z: number): number
		return widest * z + 2 * GAP
	end
	local function center_x(z: number): number
		local c = content_w(z)
		return if c <= room() then room() / 2 else c / 2 - pan:peek()
	end

	local function zoom_at(z: number, px: number, py: number)
		z = math.clamp(z, MIN_ZOOM, MAX_ZOOM)
		local z0 = zoom:peek()
		local w = room()
		if w <= 0 then
			return
		end
		local y = offset:peek() + py
		local i = row_at(y, z0)
		local f = (y - tops_for(z0)[i] - GAP) / (list[i].h * z0)
		local u = (px - center_x(z0)) / z0
		ui.batch(function()
			zoom:set(z)
			offset:set(math.max(0, tops_for(z)[i] + GAP + f * list[i].h * z - py))
			local c = content_w(z)
			pan:set(if c > w then math.clamp(c / 2 + u * z - px, 0, c - w) else 0)
		end)
		props.zoom_label:set(`{math.floor(z * 100 + 0.5)}%`)
		if drawn:peek() == 0 then
			drawn:set(z)
		else
			if settling then
				settling()
			end
			settling = ui.after(SETTLE, function()
				settling = nil
				drawn:set(zoom:peek())
			end)
		end
	end
	local function fit()
		fitted = true
		zoom_at((room() - 2 * GAP) / widest, room() / 2, 0)
	end
	local function step(dir: number)
		fitted = false
		local s = zoom:peek()
		local next_s = if dir > 0 then STEPS[#STEPS] else STEPS[1]
		if dir > 0 then
			for _, v in STEPS do
				if v > s + 0.005 then
					next_s = v
					break
				end
			end
		else
			for i = #STEPS, 1, -1 do
				if STEPS[i] < s - 0.005 then
					next_s = STEPS[i]
					break
				end
			end
		end
		zoom_at(next_s, room() / 2, (view.height or 0) / 2)
	end
	props.commands.zoom_in = function()
		step(1)
	end
	props.commands.zoom_out = function()
		step(-1)
	end
	props.commands.fit = fit
	props.commands.actual = function()
		fitted = false
		zoom_at(1, room() / 2, (view.height or 0) / 2)
	end
	props.commands.toggle = function()
		if fitted then
			props.commands.actual()
		else
			fit()
		end
	end

	ui.effect(function()
		local z = zoom:get()
		local at = offset:get()
		local i = row_at(at + (view.height or 0) / 2, z)
		props.page_label:set(locale.s("ds.file_viewer.pdf_page", { n = i, total = n }))
	end)

	local function page_left(i: number): number
		local z = zoom:get()
		local c = content_w(z)
		if c <= width:get() then
			return (room() - list[i].w * z) / 2
		end
		return GAP + (widest - list[i].w) * z / 2 - pan:get()
	end
	local function wheel(i: number, e: any): boolean?
		if e.cmd or e.ctrl then
			fitted = false
			zoom_at(zoom:peek() * math.exp(-e.dy * 0.01), e.local_x, tops_for(zoom:peek())[i] - offset:peek() + e.local_y)
			return true
		end
		local c = content_w(zoom:peek())
		if e.dx ~= 0 and c > room() then
			pan:set(math.clamp(pan:peek() + e.dx, 0, c - room()))
			return if e.dy == 0 then true else nil
		end
		return nil
	end

	local function row(i: number): Node
		local p = list[i]
		local src = ui.use_state(nil :: string?)
		local alive = true
		ui.on_cleanup(function()
			alive = false
		end)
		local near = ui.computed(function(): boolean
			local z = zoom:get()
			local at = offset:get()
			local vh = view.height or 0
			local top = tops_for(z)[i]
			return top < at + vh * 1.5 and top + row_height(i, z) > at - vh * 0.5
		end)
		ui.effect(function()
			local z = drawn:get()
			local wz = ui.window.zoom:get()
			version:get()
			if z <= 0 then
				return
			end
			-- Pictures only for pages by the view: what holds them stays bounded at any zoom.
			if not near:get() then
				src:set(nil)
				return
			end
			-- (A page a fast scroll only passes over is never drawn.)
			ui.after(if src:peek() then 0 else WAIT, function()
				M.page(path_of(props), i, z * wz):after(function(r: any)
					if alive and drawn:peek() == z then
						src:set(r.src)
					end
				end, function() end)
			end)
		end)
		local page = ui.ref()
		local pressed: any = nil
		local on_pick = props.on_pick
		return ui.div {
			w_full = true,
			h = function(): number
				return row_height(i, zoom:get())
			end,
			relative = true,
			on_wheel = function(e): boolean?
				return wheel(i, e)
			end,
			ui.div {
				ref = page,
				on_press = if on_pick
					then function(e: any)
						pressed = { x = e.local_x, y = e.local_y, wx = e.x or 0, wy = e.y or 0 }
					end
					else nil,
				on_release = if on_pick
					then function(e: any)
						local from = pressed
						pressed = nil
						if not from or e.local_x == nil then
							return
						end
						local z = zoom:peek()
						local wx, wy = e.x or from.wx, e.y or from.wy
						on_pick({
							page = i,
							x = from.x / z,
							y = from.y / z,
							x2 = e.local_x / z,
							y2 = e.local_y / z,
							box = { x = math.min(from.wx, wx), y = math.min(from.wy, wy), w = math.abs(wx - from.wx), h = math.abs(wy - from.wy) },
						})
					end
					else nil,
				absolute = true,
				top = GAP,
				left = function(): number
					return page_left(i)
				end,
				w = function(): number
					return p.w * zoom:get()
				end,
				h = function(): number
					return p.h * zoom:get()
				end,
				role = "img",
				label = locale.t("ds.file_viewer.pdf_page", { n = i, total = n }),
				bg = ui.raw("#ffffffff", "a page is paper: white in either appearance, as the host draws it"),
				border = 1,
				border_color = "$border.subtle",
				shadow = "$shadow.sm",
				overflow = "hidden",
				ui.show(
					function(): boolean
						return src:get() ~= nil
					end,
					ui.image {
						src = function(): string
							return src:get() or ""
						end,
						w_full = true,
						h_full = true,
						pointer_events = "none",
					}
				),
				if props.page_layer then props.page_layer(i, page) else nil,
			},
		}
	end

	local function measure()
		width:set(view.width or 0)
	end
	return ui.div {
		ref = view,
		w_full = true,
		h_full = true,
		focusable = true,
		label = props.label,
		focus = { border = 0 },
		on_resize = function()
			measure()
			if fitted then
				fit()
			else
				local c = content_w(zoom:peek())
				pan:set(math.clamp(pan:peek(), 0, math.max(0, c - room())))
			end
		end,
		on_mount = function()
			ui.after(0, function()
				measure()
				fit()
			end)
		end,
		on_pinch = function(e): boolean?
			fitted = false
			zoom_at(zoom:peek() * (1 + e.scale), e.local_x, e.local_y)
			return true
		end,
		on_key = function(e): boolean?
			if not e.cmd then
				return nil
			end
			if e.key == "=" or e.key == "+" then
				props.commands.zoom_in()
			elseif e.key == "-" then
				props.commands.zoom_out()
			elseif e.key == "0" then
				props.commands.fit()
			elseif e.key == "1" then
				props.commands.actual()
			else
				return nil
			end
			return true
		end,
		ui.list {
			id = "pdf-pages",
			w_full = true,
			h_full = true,
			offset = offset,
			scrollbar = true,
			ahead = 0,
			heights = function(): { number }
				local z = zoom:get()
				local hs = table.create(n)
				for i = 1, n do
					hs[i] = row_height(i, z)
				end
				return hs
			end,
			row = row,
		},
	}
end

local function component(props: Props): Node
	local phase = ui.use_state(nil :: string?)
	ui.effect(function()
		if string.sub(phase:get() or "", 1, 5) ~= "pages" then
			props.page_label:set("")
		end
	end)
	ui.on_cleanup(function()
		props.page_label:set("")
	end)
	local pages: { Size } = {}
	local why = ""
	local tries = ui.use_state(0)
	local version = ui.use_state(0)
	local shapes = 0
	local asked = 0
	ui.effect(function()
		tries:get()
		asked += 1
		local mine = asked
		M.open(path_of(props)):after(function(r: any)
			if mine ~= asked then
				return
			end
			if phase:peek() == `pages{shapes}` and same(pages, r.pages) then
				version:set(version:peek() + 1)
				return
			end
			pages = r.pages
			shapes += 1
			phase:set(`pages{shapes}`)
		end, function(err: any)
			if mine ~= asked then
				return
			end
			why = reason(err)
			phase:set("failed")
		end)
	end)
	local function again()
		ui.batch(function()
			phase:set(nil)
			tries:set(tries:peek() + 1)
		end)
	end
	return ui.presence(phase, function(which: string): Node
		if which == "failed" then
			return props.failed(why, again)
		end
		return pages_view(props, pages, version)
	end, { w_full = true, h_full = true })
end

M.View = ui.component(component)

return M
