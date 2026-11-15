-- The service cannot read the window's settings, so the window hands the folder over.

local zeron = require("./zeron")("yab-bridge")

local function push(root: any)
	zeron.service():call("configure", { clone_root = root or "" }):after(function() end, function(err: any)
		print(`yab-bridge: could not hand over the clone folder: {zeron.reason(err)}`)
	end)
end

push(zeron.setting("clone_root"):peek())

zeron.on_setting(function(id: string, value: any)
	if id == "clone_root" then
		push(value)
	end
end)
