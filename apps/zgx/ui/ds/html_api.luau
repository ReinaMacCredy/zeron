-- src/html_pages.rs behind one module, so ui/chat_fixture.luau can stand in for it.

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

-- `boot` runs in a world of the app's own (zgx `isolated`, macOS), where the page's scripts can't reach it.
function M.web(id: string, boot: string?): any
	-- (A keyed page outlives a reload with the scripts it was made with: a page read anew gets a new one.)
	M.close((ui :: any).web(id))
	return (ui :: any).web(id, { transparent = true, profile = "ephemeral", isolated = if boot then { boot } else nil })
end

-- A stored page keeps its document, as gpui's `page_navigation` keeps it: a
-- link goes to `on_link` (the browser) and nothing navigates the page, a
-- script's `location.href` included, before it loads. (An old zgx has no
-- `decide_navigation`: the card's `on_navigate` puts the page back.)
function M.keep_page(web: any, on_link: (string) -> (), home: ((url: string) -> boolean)?)
	local ok, decide = pcall(function()
		return web.decide_navigation
	end)
	if not ok or decide == nil then
		return
	end
	web:decide_navigation(function(r: any): boolean
		if not r.main_frame then
			return string.sub(r.url, 1, 5) ~= "file:"
		end
		if r.url == "about:blank" or string.sub(r.url, 1, 12) == "about:blank#" then
			return true
		end
		if home and home(r.url) then
			return true
		end
		if r.kind == "link" then
			on_link(r.url)
		end
		return false
	end)
end

-- (No base: `about:blank`, as gpui loads a page, so relative references fail closed.)
function M.load(web: any, html: string)
	web:html(html)
end

function M.run(web: any, js: string)
	web:run(js)
end

function M.chain(web: any, edges: number)
	pcall(function()
		web:scroll_chain({
			up = bit32.btest(edges, 1),
			down = bit32.btest(edges, 2),
			left = bit32.btest(edges, 4),
			right = bit32.btest(edges, 8),
		})
	end)
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

function M.forget(args: any)
	sync("html.forget", args)
end

function M.picks(state: any): string?
	return sync("html.picks_state", state)
end

function M.open_url(url: string)
	pcall(ui.invoke, "open_url", url)
end

-- A link leaving a stored page opens in the browser: an http(s) URL with a host and no user info, once a second at most.
function M.leave(e: any, url: string)
	local host = string.match(url, "^https?://([^/?#]*)")
	local t = ui.now()
	if host and host ~= "" and not string.find(host, "@", 1, true) and t - (e.opened or -1e9) >= 1000 then
		e.opened = t
		M.open_url(url)
	end
end

function M.file_web(id: string): any
	M.close((ui :: any).web(id))
	return (ui :: any).web(id, { profile = "ephemeral", messages = false })
end

return M
