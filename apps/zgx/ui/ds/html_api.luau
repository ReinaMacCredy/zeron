-- An agent page's host side (src/html_pages.rs), behind one module so tests
-- can stand in for it (ui/chat_fixture.luau): the page read and given its
-- bootstrap, its view, a dev server's reach, the sheet's source and save.
--
--   api.page(args, done)        `html.page`: done({ html, nonce }) or done(nil, why)
--   api.view(props)             zgx's web view (ui.webview)
--   api.web(id)                 the host's page for `id` (ui.web), kept across
--                               its row leaving the view; api.load(web, html,
--                               base), api.run(web, js), api.close(web)
--   api.retheme(args)           the script that swaps a shown page's theme
--   api.frame(agent, content)   { height, overflows, scrolls }
--   api.live(url)               { label, server, url } of a dev server, or nil
--   api.reach(url, done)        done(answers)
--   api.source(args)            what Source shows, or nil
--   api.save(args, done)        the page written where the person chose
--   api.file_name(title)        its download name
--   api.title(path, done)       done({ title? } or { missing = true })
--   api.picks(state)            the script that shows a page's picks
--   api.open_url(url)           in the default browser

local M = {}

-- (Without the host, as headless, a call fails rather than raising.)
local function call(name: string, arg: any): any
	local ok, p = pcall(ui.call, name, arg)
	return if ok then p else nil
end

local function sync(name: string, arg: any): any
	local ok, v = pcall(ui.call_sync, name, arg)
	return if ok then v else nil
end

local function settle(p: any, done: (any, string?) -> ())
	if not p then
		done(nil, "no host")
		return
	end
	p:after(function(v: any)
		done(v, nil)
	end, function(err: any)
		done(nil, tostring(err))
	end)
end

function M.page(args: any, done: (any, string?) -> ())
	settle(call("html.page", args), done)
end

function M.view(props: any): Node
	return (ui :: any).webview(props)
end

function M.web(id: string): any
	return (ui :: any).web(id, { transparent = true, profile = "ephemeral" })
end

function M.load(web: any, html: string, base: string)
	web:html(html, base)
end

function M.run(web: any, js: string)
	web:run(js)
end

function M.close(web: any)
	pcall(function()
		web:close()
	end)
end

function M.retheme(args: any): string?
	return sync("html.retheme", args)
end

function M.frame(agent: number, content: number?): any
	local out = sync("html.frame", { agent = agent, content = content })
	if type(out) == "table" then
		return out
	end
	local h = math.clamp(math.floor(agent + 0.5), 80, 2000)
	return { height = h, agent = h, overflows = false, scrolls = false }
end

function M.live(url: string): any
	return sync("html.live", url)
end

function M.reach(url: string, done: (boolean) -> ())
	settle(call("html.reach", url), function(v: any)
		done(v == true)
	end)
end

function M.source(args: any): string?
	return sync("html.source", args)
end

function M.save(args: any, done: (boolean) -> ())
	settle(call("html.save", args), function(v: any)
		done(v == true)
	end)
end

function M.file_name(title: string): string
	return sync("html.file_name", title) or `{title}.html`
end

function M.title(path: string, done: (any) -> ())
	settle(call("html.title", path), function(v: any)
		done(v or { missing = true })
	end)
end

function M.picks(state: any): string?
	return sync("html.picks_state", state)
end

function M.open_url(url: string)
	pcall(ui.invoke, "open_url", url)
end

return M
