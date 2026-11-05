-- A new chat's picture of your own, as the shipping app's "New thread
-- background" (crates/ui settings.rs, settings/appearance.rs): one image,
-- chosen, replaced, adjusted or removed. The host keeps a copy of it
-- (`wallpaper.keep`), so the original can move or go; replacing or
-- removing it lets that copy go (`wallpaper.forget`).
--
--   wallpaper.choose()       asks for a picture, keeps it, and shows it
--   wallpaper.remove()       no picture (a new chat opens on nothing)
--   wallpaper.missing()      whether the kept copy has gone
--   wallpaper.adjusting      the Adjust dialog's state (open, its draft)
--   wallpaper.problem_text() what went wrong, in the locale shown ("" nothing)
--   wallpaper.EffectSelect(state)  Zeron's effects, as a select

local ds = require("@ds")
local appearance = require("@app/screens/shell/appearance")
local host = require("@app/screens/settings/host")
local shuffle_order = require("@ds/shuffle_order")

local M = {}

-- Zeron's effects as a select (Appearance's over your picture, and on a
-- plugin's settings page over its wallpaper).
function M.EffectSelect(value: State<string>): Node
	return ds.Select {
		value = value,
		name = ui.t("settings.appearance.effect.label"),
		w = 160,
		options = function(): { any }
			local out = {}
			for _, e in ds.wallpaper.EFFECTS do
				table.insert(out, { value = e, label = ui.t(`settings.appearance.effect.{e}.label`):get() })
			end
			return out
		end,
	}
end

-- What went wrong ("" nothing: else its message's key).
M.problem = ui.state("wallpaper_problem", "")
function M.problem_text(): string
	local key = M.problem:get()
	return if key ~= "" then ui.t(key):get() else ""
end

-- Whether the picture kept has gone (its file removed by hand).
function M.missing(): boolean
	local image = appearance.image:get()
	if image == nil then
		return false
	end
	-- (Its header read: nothing there, or not a picture.)
	local ok, w = pcall(ui.image_size, image.path)
	return not ok or w == nil
end

local FORGET_AFTER = 1000

local function show_kept(kept: any, source: string)
	local was = appearance.image:peek()
	local before = appearance.source:peek()
	ui.batch(function()
		M.problem:set("")
		appearance.image:set({ path = kept.path, name = kept.name, w = kept.w, h = kept.h, x = 0.5, y = 0.5, zoom = 1 })
		appearance.choose("image")
		appearance.color:set(kept.color or "")
		local history = appearance.history:peek()
		if before ~= "" then
			history = shuffle_order.remember(history, before)
		end
		appearance.history:set(shuffle_order.remember(history, source))
		appearance.source:set(source)
	end)
	if was and was.path ~= kept.path then
		-- (After the crossfade: the old picture's still leaving, from that file.)
		ui.after(FORGET_AFTER, function()
			host.invoke("wallpaper.forget", { path = was.path })
		end)
	end
end

function M.choose()
	host.open_files({
		title = ui.t("settings.appearance.wallpaper.choose_title"):get(),
		filters = { { ui.t("settings.appearance.wallpaper.images"):get(), { "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff" } } },
	}):after(function(files: { File })
		local f = files[1]
		if not f then
			return
		end
		host.call("wallpaper.keep", { path = f.path }):after(function(kept: any)
			show_kept(kept, f.path)
		end, function(err: any)
			local word = if type(err) == "table" then err.message else nil
			M.problem:set(if word == "invalid" then "settings.appearance.wallpaper.unsupported" else "settings.appearance.wallpaper.couldnt_keep")
		end)
	end)
end

function M.remove()
	local was = appearance.image:peek()
	ui.batch(function()
		M.problem:set("")
		appearance.image:set(nil)
		appearance.source:set("")
		appearance.color:set("")
		if appearance.wallpaper:peek() == "image" then
			appearance.wallpaper:set("none")
		end
	end)
	if was then
		host.invoke("wallpaper.forget", { path = was.path })
	end
end

function M.choose_folder()
	ui.open_folder({ title = ui.t("settings.appearance.folder.choose_title"):get() }):after(function(folder: any)
		if not folder then
			return
		end
		folder:list("", { files = true, limit = 4096 }):after(function(entries: any)
			for _, entry in entries do
				local path = if entry.file then string.match(entry.file.path, "^(.*)/[^/]+$") else nil
				if path then
					M.set_folder(path)
					return
				end
			end
			M.problem:set("settings.appearance.folder.empty")
		end, function()
			M.problem:set("settings.appearance.folder.unreadable")
		end)
	end)
end

function M.set_folder(path: string)
	ui.batch(function()
		M.problem:set("")
		appearance.folder:set(path)
		appearance.history:set({})
	end)
end

local function shuffle_failed(key: string)
	M.problem:set(key)
	ds.toast.show { title = ui.t(key):get(), tone = "danger" }
end

local shuffling = false
function M.shuffle()
	local folder = appearance.folder:peek()
	if folder == "" then
		shuffle_failed("settings.appearance.folder.first")
		return
	end
	if shuffling then
		return
	end
	shuffling = true
	host.call("wallpaper.folder", { path = folder }):after(function(list: any)
		local history = appearance.history:peek()
		local source = appearance.source:peek()
		if source ~= "" then
			history = shuffle_order.remember(history, source)
		end
		local order = shuffle_order.order(if type(list) == "table" then list else {}, history, math.random)
		local function try(i: number)
			local path = order[i]
			if not path then
				shuffling = false
				shuffle_failed("settings.appearance.folder.empty")
				return
			end
			host.call("wallpaper.keep", { path = path }):after(function(kept: any)
				shuffling = false
				show_kept(kept, path)
			end, function()
				try(i + 1)
			end)
		end
		try(1)
	end, function()
		shuffling = false
		shuffle_failed("settings.appearance.folder.unreadable")
	end)
end

function M.ask_color()
	local image = appearance.image:peek()
	if not image or appearance.color:peek() ~= "" then
		return
	end
	host.call("wallpaper.color", { path = image.path }):after(function(color: any)
		local now = appearance.image:peek()
		if now and now.path == image.path and type(color) == "string" then
			appearance.color:set(color)
		end
	end, function() end)
end

M.adjusting = ui.state("wallpaper_adjusting", false)
M.draft = ui.state("wallpaper_adjust_draft", { x = 0.5, y = 0.5, zoom = 1 })
M.adjusting_path = ui.state("wallpaper_adjust_path", "")
M.preview_ready = ui.state("wallpaper_adjust_ready", "")

function M.adjust()
	local image = appearance.image:peek()
	if not image then
		return
	end
	ui.batch(function()
		M.draft:set({ x = image.x, y = image.y, zoom = image.zoom })
		M.adjusting_path:set(image.path)
		M.preview_ready:set("")
		M.adjusting:set(true)
	end)
end

function M.adjust_blocked(): string
	local image = appearance.image:get()
	if not image or image.path ~= M.adjusting_path:get() then
		return "settings.appearance.wallpaper.adjust.changed"
	elseif M.missing() then
		return "settings.appearance.wallpaper.image.missing"
	elseif M.preview_ready:get() ~= image.path then
		return "settings.appearance.wallpaper.adjust.preparing"
	end
	return ""
end

function M.keep_adjustment()
	if M.adjust_blocked() ~= "" then
		return
	end
	local image = appearance.image:peek()
	if image then
		local d = M.draft:peek()
		local next = table.clone(image)
		next.x, next.y, next.zoom = d.x, d.y, d.zoom
		appearance.image:set(next)
	end
	M.adjusting:set(false)
end

return M
