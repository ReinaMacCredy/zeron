-- A chat's links' context menus where they aren't the web's (ds/markdown's
-- `link_menu`; ds/links makes the entries):
--   a chat mentioned   Open, Copy link (its Zeron link, `zeron://open/…`)
--   a file in its folder   Open, Reveal in Finder (the chat on this Mac),
--                          Copy path (its whole path, on the chat's device)
--   a file outside it      Open (read only), Copy path
-- Anything else that isn't a file (a `#section`, a link still arriving) has
-- none: the message's own menu shows.
--
--   transcript { open_link = open, link_menu = LinkMenu(open, function() return store.session:peek() end) }

local ds = require("@ds")
local Links = require("@ds/links")
local FileLinks = require("@app/screens/chat/file_links")
local sessions = require("@account/sessions")
local files = require("@app/screens/files/api")

local function base_name(path: string): string
	return string.match(path, "([^/]+)/?$") or path
end

-- (From selected text most dotted words are code, not files: `string.find`.
-- Only one its folder lists is offered.)
local function listed(known: { [string]: boolean }, s: any, cwd: string, path: string): boolean
	local key = `{s.id}\0{path}`
	if known[key] == nil then
		known[key] = false
		local dir = string.match(path, "^(.*)/[^/]+$") or ""
		files.list({ chat = s.id, device = s.device, root = cwd }, dir):after(function(listing: any)
			known[key] = nil
			for _, e in (if type(listing) == "table" then listing.entries else nil) or {} do
				if e.path == path and not e.dir then
					known[key] = true
				end
			end
		end, function()
			known[key] = nil
		end)
	end
	return known[key] == true
end

return function(open: (target: string) -> (), session: () -> any?): (target: string, selected: boolean?) -> { Links.Entry }?
	local known: { [string]: boolean } = {}
	return function(target: string, selected: boolean?): { Links.Entry }?
		local chat = string.match(target, "^zeron%-chat:([%w%-_]+)$")
		if chat and not selected then
			return Links.chat {
				open = function()
					open(target)
				end,
				copy = function()
					sessions.link(chat):after(function(link: any)
						if type(link) == "string" then
							Links.copy(link)
						end
					end, function()
						ds.toast.show { title = ui.t("ds.links.link_not_ready"), tone = "danger" }
					end)
				end,
			}
		end
		if Links.web(target) or (chat and selected) then
			return nil
		end
		local s = session()
		local cwd = if s then s.cwd else nil
		local r = FileLinks.resolve(target, cwd)
		if not r then
			return {}
		end
		local path = r.path
		if r.outside then
			return Links.file {
				open = function()
					open(target)
				end,
				path = path,
				panel = selected,
			}
		end
		if selected and not (s and cwd and listed(known, s, cwd, path)) then
			return {}
		end
		local here = s ~= nil and (s.device == nil or s.device == sessions.now:peek().here)
		return Links.file {
			open = function()
				open(target)
			end,
			reveal = if here and s and cwd
				then function()
					files.reveal({ chat = s.id, device = s.device, root = cwd }, path):after(function() end, function()
						ds.toast.show { title = ui.t("files.workspace.not_revealed", { name = base_name(path) }), tone = "danger" }
					end)
				end
				else nil,
			path = if cwd then `{(string.gsub(cwd, "/+$", ""))}/{path}` else path,
			panel = selected,
		}
	end
end
