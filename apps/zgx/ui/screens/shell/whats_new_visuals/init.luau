local motion = require("@ds/motion")
local K = require("@app/screens/shell/whats_new_visuals/kit")

local M = {}

export type Visual = { period: number, rest: number, build: (clock: K.Clock) -> Node }

local answer_pill = require("@app/screens/shell/whats_new_visuals/answer_pill")
local text_annotation = require("@app/screens/shell/whats_new_visuals/text_annotation")

local VISUALS: { [string]: Visual } = {
	["answer-pill"] = answer_pill.card,
	["answer-pill-hero"] = answer_pill.hero,
	["keep-awake"] = require("@app/screens/shell/whats_new_visuals/keep_awake"),
	["compaction-morph"] = require("@app/screens/shell/whats_new_visuals/compaction_morph"),
	["swipe-dot"] = require("@app/screens/shell/whats_new_visuals/swipe_dot"),
	["tab-drag"] = require("@app/screens/shell/whats_new_visuals/tab_drag"),
	["text-annotation"] = text_annotation.card,
	["text-annotation-hero"] = text_annotation.hero,
	["skills-page"] = require("@app/screens/shell/whats_new_visuals/skills_page"),
	["worktree-follow"] = require("@app/screens/shell/whats_new_visuals/worktree_follow"),
	["task-depends"] = require("@app/screens/shell/whats_new_visuals/task_depends"),
	["live-preview"] = require("@app/screens/shell/whats_new_visuals/live_preview"),
	["window-frame"] = require("@app/screens/shell/whats_new_visuals/window_frame"),
	["project-picker"] = require("@app/screens/shell/whats_new_visuals/project_picker"),
	["zgx-app"] = require("@app/screens/shell/whats_new_visuals/zgx_app"),
	["markdown-fence-render"] = require("@app/screens/shell/whats_new_visuals/markdown_fence_render"),
}

function M.has(id: string?): boolean
	return id ~= nil and VISUALS[id] ~= nil
end

-- A feature the shared toml gives no visual (gpui's test refuses ids it doesn't draw) takes ours by its id.
function M.of(item: any): string?
	local id = if item.visual == nil then item.id else item.visual
	return if M.has(id) then id else nil
end

function M.any(entry: any): boolean
	for _, f in entry.features do
		if M.of(f) then
			return true
		end
	end
	for _, i in entry.improvements do
		if M.of(i) then
			return true
		end
	end
	return false
end

function M.clock(name: string, on: () -> boolean): State<number>
	local ms = ui.state(name, 0)
	local running = ui.computed(function(): boolean
		return on() and not motion.reduced()
	end)
	ui.effect(function()
		if not running:get() then
			return
		end
		ms:set(0)
		local timer = anime.createTimer {
			frameRate = 60,
			onUpdate = function(a: Animation)
				ms:set(a.currentTime)
			end,
		}
		ui.on_cleanup(function()
			timer:cancel()
		end)
	end)
	return ms
end

M.Stage = ui.component(function(props: { visual: string, clock: State<number> }): Node
	local v = VISUALS[props.visual]
	local t = ui.computed(function(): number
		return if motion.reduced() then v.rest else (props.clock:get() / 1000) % v.period
	end)
	local clock: K.Clock = {
		t = function(): number
			return t:get()
		end,
		ms = function(): number
			return props.clock:get()
		end,
		reduced = motion.reduced,
	}
	return ui.row {
		id = `whats-new-visual-{props.visual}`,
		w_full = true,
		h_full = true,
		items = "center",
		justify = "center",
		aria_hidden = true,
		color = "$text.secondary",
		text_size = K.raw(12.5),
		opacity = function(): number
			return 1 - K.ramp(t:get(), v.period - 0.35, 300, K.EASE_IN)
		end,
		v.build(clock),
	}
end)

function M.Frame(props: { visual: string, clock: State<number>, h: number }): Node
	return ui.div {
		w_full = true,
		h = props.h,
		shrink = 0,
		overflow = "hidden",
		rounded = "$radius.lg",
		border = 1,
		border_color = "$border",
		bg = "$zeron.whats_new.frame",
		M.Stage { visual = props.visual, clock = props.clock },
	}
end

return M
