-- A Bot's chat, over Zeron's ordinary transcript: its avatar and name
-- above the conversation (the name opens the Bot's menu), and what opens
-- the conversation: its greeting and what it offers to do first.

local ds = require("@ds")
local Menu = require("@app/screens/bots/menu")
local panel = require("@app/screens/bots/panel")
local store = require("@app/screens/bots/store")
local avatar = require("@app/screens/bots/avatar")
local templates = require("@app/screens/bots/templates")

local M = {}

M.HEAD_H = 112

-- The Bot whose chat `chat` is (nil: an ordinary chat).
function M.of(chat: string?): store.Bot?
	return store.of_chat(chat)
end

M.Header = ui.component(function(props: { bot: () -> store.Bot? }): Node
	local open = ui.use_state(false)
	local function bot(): store.Bot
		return props.bot() or ({ id = "", name = "", chat = "", avatar = {}, pinned = false, archived = false, unread = false } :: any)
	end
	local function needs(): number
		local b = props.bot()
		return if b then panel.needs(b) else 0
	end
	return ui.column {
		id = "bot-chat-head",
		w_full = true,
		h = M.HEAD_H,
		shrink = 0,
		items = "center",
		pt = "$space.2",
		avatar.Avatar {
			id = "bot-chat-avatar",
			look = function(): avatar.Look
				return bot().avatar
			end,
			size = 56,
		},
		Menu.Popup {
			open = open,
			label = ui.t("bots.menu.label"),
			placement = { side = "bottom", align = "center", offset = 6 },
			items = function(): { Menu.Entry }
				local b = props.bot()
				return if b then panel.menu(b) else {}
			end,
			anchor = ui.button {
				id = "bot-chat-pill",
				label = function(): string
					return bot().name
				end,
				mt = ui.raw(5, "the mockup's name pill"),
				px = ui.raw(13, "the mockup's name pill"),
				py = function(): number
					return ui.raw(if needs() > 0 then 4 else 5, "the mockup's name pill, two lines inside the header's height")
				end,
				max_w = 360,
				items = "center",
				rounded = "$radius.xl",
				border = 1,
				border_color = "$border.subtle",
				bg = "$surface.base",
				hover = { bg = "$hover" },
				active = { bg = "$pressed" },
				on_click = ui.toggle(open),
				ui.column {
					items = "center",
					min_w = 0,
					ds.FadeText {
						function(): string
							return bot().name
						end,
						text_size = ui.raw(13.5, "the mockup's name pill"),
						line_height = ui.raw(18, "the mockup's name pill"),
						weight = "$font.weight.medium",
						color = "$text.primary",
						max_w = 330,
					},
					ui.show(function(): boolean
						return needs() > 0
					end, ui.text {
						ui.t("bots.chat.needs", { n = needs }),
						id = "bot-chat-needs",
						text_size = "$font.size.sm",
						line_height = ui.raw(15, "the mockup's name pill"),
						color = "$warning",
					}),
				},
			},
		},
	}
end)

-- What opens the conversation: "Hi, I'm <name>.", the template's two lines,
-- then what it offers; once the person has answered, one line saying so.
M.Lead = ui.component(function(props: { bot: () -> store.Bot?, answered: () -> boolean, on_pick: (title: string) -> () }): Node
	local function name(): string
		local b = props.bot()
		return if b then b.name else ""
	end
	local function template(): templates.Template
		local b = props.bot()
		return templates.find(if b then b.template else nil)
	end
	local function line(words: any): Node
		return ui.text { words, w_full = true, wrap = true, text_size = "$font.size.md", line_height = ui.raw(22, "the transcript's line"), color = "$text.primary" }
	end
	local function pick(i: number): Node
		local function p(): templates.Pick
			return template().picks[i] or { icon = "pen", title = "", line = "" }
		end
		return ui.button {
			id = `bot-chat-pick-{i}`,
			label = function(): string
				return ui.t(p().title):get()
			end,
			w_full = true,
			px = ui.raw(10, "the mockup's card"),
			py = "$space.2",
			gap = "$space.3",
			items = "center",
			rounded = ui.raw(9, "the mockup's card"),
			bg = "#00000000",
			hover = { bg = "$hover" },
			active = { bg = "$pressed" },
			on_click = function()
				props.on_pick(ui.t(p().title):get())
			end,
			ui.div {
				w = 30,
				h = 30,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.lg",
				border = 1,
				border_color = "$border.subtle",
				color = "$text.primary",
				ds.Icon {
					name = function(): any
						return p().icon
					end,
					size = 16,
				},
			},
			ui.column {
				grow = 1,
				min_w = 0,
				ui.text {
					function(): string
						return ui.t(p().title):get()
					end,
					text_size = ui.raw(13.5, "the mockup's card"),
					weight = "$font.weight.medium",
					color = "$text.primary",
				},
				ui.text {
					function(): string
						return ui.t(p().line):get()
					end,
					text_size = ui.raw(12.5, "the mockup's card"),
					color = "$text.secondary",
				},
			},
			ui.div { shrink = 0, color = "$text.secondary", aria_hidden = true, ds.Icon { name = "alt-arrow-right", size = 14 } },
		}
	end
	return ui.column {
		id = "bot-chat-lead",
		w_full = true,
		gap = "$space.3",
		pb = ui.raw(18, "the mockup's card"),
		line(ui.t("bots.chat.hello", { name = name })),
		line(function(): string
			return ui.t(template().greeting[1]):get()
		end),
		line(function(): string
			return ui.t(template().greeting[2]):get()
		end),
		ui.presence(function(): string
			return if props.answered() then "answered" else "card"
		end, function(kind: string): Node
			if kind == "answered" then
				return ui.row {
					id = "bot-chat-answered",
					w_full = true,
					gap = "$space.2",
					items = "center",
					ui.text { ui.t("bots.chat.work", { name = name }), text_size = ui.raw(13.5, "the mockup's card"), color = "$text.primary" },
					ui.text { ui.t("bots.chat.answered"), text_size = ui.raw(13.5, "the mockup's card"), color = "$text.secondary" },
				}
			end
			return ui.column {
				id = "bot-chat-card",
				w_full = true,
				max_w = 600,
				p = ui.raw(6, "the mockup's card"),
				rounded = ui.raw(14, "the mockup's card"),
				border = 1,
				border_color = "$border.subtle",
				ui.text {
					ui.t("bots.chat.work", { name = name }),
					role = "heading",
					mx = ui.raw(10, "the mockup's card"),
					mt = ui.raw(10, "the mockup's card"),
					mb = "$space.2",
					text_size = "$font.size.md",
					weight = "$font.weight.medium",
					color = "$text.primary",
				},
				pick(1),
				pick(2),
				pick(3),
			}
		end, { w_full = true, mt = "$space.1" }),
	}
end)

return M
