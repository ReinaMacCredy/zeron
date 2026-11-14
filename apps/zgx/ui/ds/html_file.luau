local api = require("@self/html_api")

local M = {}

export type Props = { path: string, label: Label?, saved: () -> string }

local made = 0

local function file_url(path: string): string
	return "file://" .. (string.gsub(path, "[^%w%-%._~/]", function(c: string): string
		return string.format("%%%02X", string.byte(c))
	end))
end

M.View = ui.component(function(props: Props): Node
	made += 1
	local web = api.file_web(`file-viewer-page-{made}`)
	local links = {}
	local home: string? = nil
	local loading = false
	api.keep_page(web, function(url: string)
		api.leave(links, url)
	end, function(url: string): boolean
		if home then
			return url == home or string.sub(url, 1, #home + 1) == home .. "#"
		end
		if loading and string.sub(url, 1, 7) == "file://" then
			loading = false
			home = string.match(url, "^[^#]*")
			return true
		end
		return false
	end)
	local shown = false
	ui.effect(function()
		props.saved()
		if shown then
			web:reload()
			return
		end
		shown = true
		loading = true
		web:load(file_url(props.path))
	end)
	ui.on_cleanup(function()
		api.close(web)
	end)
	return api.view {
		id = "file-viewer-page",
		label = props.label,
		web = web,
		w_full = true,
		h_full = true,
		on_new_window = function(url: string)
			api.leave(links, url)
		end,
	}
end)

return M
