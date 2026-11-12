local motion = require("@ds/motion")
local glass = require("@ds/glass")
local status = require("@ds/status")
local boundary = require("@app/screens/shell/boundary")

local M = {}

local EASE = "cubicBezier(.22,1,.36,1)"
local PILL_AT, PILL_STAY, PILL_H, PILL_GAP = 450, 5200, 30, 14

M.carried = ui.shared("migration.carried", nil :: any)
local played = ui.state("arrival_played", false)
local fresh = M.carried:peek() ~= nil and not played:peek()
local migrating = M.carried:peek() ~= nil
M.frame = ui.state("arrival_frame", if fresh then 0 else 1)
M.frame_in = ui.state("arrival_frame_in", if fresh then 0 else 1)
M.phase = ui.state("arrival_phase", "")
local rise = ui.state("arrival_rise", 0)
local drop = ui.state("arrival_drop", 0)
local shown = ui.state("arrival_shown", 0)
M.counted = ui.state("arrival_counted", 0)
local started_at = ui.state("arrival_started_at", 0)
local moved_at = ui.state("arrival_moved_at", 0)

local function counts(): (number, number)
	local c = M.carried:peek()
	return if c then c.settings or 0 else 0, if c then c.drafts or 0 else 0
end

local function total(): number
	local s, d = counts()
	return s + d
end

function M.waiting(): boolean
	local p = M.phase:get()
	return M.carried:get() ~= nil and p ~= "gone"
end

function M.leave()
	local p = M.phase:peek()
	if p ~= "moving" and p ~= "moved" then
		return
	end
	M.phase:set("leaving")
	if not motion.reduced_now() then
		anime.animate(drop, { value = 6, duration = 160, ease = "in(2)" })
	end
	anime.animate(shown, {
		value = 0,
		duration = 160,
		ease = "in(2)",
		onComplete = function()
			M.phase:set("gone")
		end,
	})
end

local function count()
	local n = total()
	local step = math.max(1, math.ceil(n / 9))
	local function tick()
		if M.phase:peek() ~= "moving" then
			return
		end
		local next = math.min(n, M.counted:peek() + step)
		M.counted:set(next)
		if next < n then
			ui.after(45, tick)
		else
			ui.after(120, function()
				if M.phase:peek() == "moving" then
					moved_at:set(ui.now())
					M.phase:set("moved")
					ui.after(PILL_STAY, M.leave)
				end
			end)
		end
	end
	ui.after(200, tick)
end

local function pill_in()
	if M.phase:peek() ~= "" then
		return
	end
	M.counted:set(0)
	M.phase:set("moving")
	drop:set(0)
	if motion.reduced_now() then
		rise:set(1)
		shown:set(1)
	else
		rise:set(0)
		anime.animate(rise, { value = 1, duration = 260, ease = EASE })
		anime.animate(shown, { value = 1, duration = 180, ease = "out(2)" })
	end
	count()
end

local function start()
	if played:peek() or total() == 0 then
		M.frame:set(1)
		M.frame_in:set(1)
		return
	end
	played:set(true)
	local reduced = motion.reduced_now()
	M.frame:set(if reduced then 1 else 0)
	M.frame_in:set(0)
	if reduced then
		anime.animate(M.frame_in, { value = 1, duration = 150, ease = "linear" })
	else
		anime.animate(M.frame, { value = 1, duration = 400, ease = EASE })
		anime.animate(M.frame_in, { value = 1, duration = 300, ease = "out(2)" })
	end
	started_at:set(ui.now())
	ui.after(PILL_AT, pill_in)
end

-- (A reload drops timers and animations under way, the state kept: the run goes on from where it stood.)
if played:peek() then
	if M.frame:peek() < 1 or M.frame_in:peek() < 1 then
		anime.animate(M.frame, { value = 1, duration = 150, ease = EASE })
		anime.animate(M.frame_in, { value = 1, duration = 150, ease = EASE })
	end
	local p = M.phase:peek()
	if p == "" then
		ui.after(math.max(0, started_at:peek() + PILL_AT - ui.now()), pill_in)
	elseif p == "moving" or p == "moved" then
		rise:set(1)
		shown:set(1)
		if p == "moving" then
			count()
		else
			ui.after(math.max(0, moved_at:peek() + PILL_STAY - ui.now()), M.leave)
		end
	elseif p == "leaving" then
		shown:set(0)
		M.phase:set("gone")
	end
end

ui.effect(function()
	if M.carried:get() ~= nil and not played:peek() then
		ui.after(0, start)
	end
end)

local typed = ui.shared("chat.composer.text", "")
local was = typed:peek()
ui.effect(function()
	local now = typed:get()
	if now ~= was then
		was = now
		M.leave()
	end
end)

function M.frame_props(props: any): any
	if not migrating then
		return props
	end
	props.opacity = function(): number
		return M.frame_in:get()
	end
	props.scale = function(): number
		return 1.02 - 0.02 * M.frame:get()
	end
	props.blur = function(): number
		return 8 * (1 - M.frame:get())
	end
	return props
end

function M.said(): string
	local s, d = counts()
	if M.phase:get() == "moving" then
		return ui.t("shell.arrival.moving", { n = M.counted:get(), total = s + d }):get()
	end
	if s > 0 and d > 0 then
		return ui.t("shell.arrival.moved", { settings = s, drafts = d }):get()
	end
	return ui.t(if s > 0 then "shell.arrival.moved_settings" else "shell.arrival.moved_drafts", { settings = s, drafts = d }):get()
end

function M.Pill(): Node
	return ui.show(function(): boolean
		local p = M.phase:get()
		return p == "moving" or p == "moved" or p == "leaving"
	end, ui.row {
		id = "arrival",
		absolute = true,
		left = 0,
		right = 0,
		top = -(PILL_GAP + PILL_H),
		h = PILL_H,
		justify = "center",
		pointer_events = "none",
		ui.row(glass.style({
			id = "arrival-pill",
			rounded = "$radius.full",
			h = PILL_H,
			pl = "$space.2",
			pr = "$space.3",
			gap = "$space.2",
			items = "center",
			pointer_events = "auto",
			opacity = function(): number
				return shown:get()
			end,
			y = function(): number
				return 8 * (1 - rise:get()) + drop:get()
			end,
			ui.switch(function(): string
				return if M.phase:get() == "moving" then "moving" else "moved"
			end, {
				moving = status.Spinner { size = 14 },
				moved = status.StatusIcon { kind = "done", size = 14 },
			}),
			ui.text { M.said, text_size = "$font.size.sm", color = "$text.primary", wrap = false, tabular = true },
			ui.show(function(): boolean
				return M.phase:get() ~= "moving"
			end, ui.button {
				id = "arrival-whats-new",
				bg = "#00000000",
				color = "$accent",
				text_size = "$font.size.sm",
				hover = { color = "$text.primary" },
				on_click = function()
					M.leave()
					local whats_new: any = boundary.load("@app/screens/shell/whats_new")
					if whats_new then
						whats_new.open()
					end
				end,
				ui.t("shell.arrival.whats_new"),
			}),
		}, "menu") :: any),
	})
end

return M
