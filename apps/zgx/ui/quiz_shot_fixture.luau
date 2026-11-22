-- (Screenshot only: ds/quiz in a visual's frame and a message a visual sent,
-- with no host: `zgx-run --inspect ui/quiz_shot_fixture.luau`. The frame
-- here copies ds/visual's, which needs src/visuals.rs to run a script.)

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
ui.theme.follow_system(false)
ui.theme.current:set("zeron-light")
local Quiz = require("@ds/quiz")
local UserMessage = require("@ds/user_message")
local Icon = require("@ds/icon")

local QUESTIONS = {
	{
		kind = "single",
		question = "What does the learning rate set?",
		hint = "Think of one step of the update.",
		options = {
			{ text = "How many layers the network has", why = "That is the architecture, fixed before training." },
			{ text = "How far each step moves the weights", correct = true, why = "Each update is the rate times the gradient." },
			{ text = "How much data a batch holds", why = "That is the batch size." },
			{ text = "How many epochs it trains for" },
		},
	},
	{
		kind = "multi",
		question = "Which make training unstable?",
		options = {
			{ text = "A learning rate far too large", correct = true, why = "Steps overshoot the minimum." },
			{ text = "Exploding gradients", correct = true, why = "Updates grow without bound." },
			{ text = "A validation set", why = "It only measures." },
		},
	},
	{
		kind = "truefalse",
		question = "Gradient descent always finds the global minimum.",
		options = { { text = "True" }, { text = "False", correct = true, why = "It can settle in a local one." } },
	},
}

local sent = ui.state("sent", {} :: { string })

local function frame(title: string, body: Node): Node
	return ui.column {
		class = "visual",
		w_full = true,
		border = 1,
		border_color = "$border",
		rounded = ui.raw(12, "the visual's frame, as ds/visual's"),
		pt = ui.raw(11, "the visual's frame, as ds/visual's"),
		px = ui.raw(14, "the visual's frame, as ds/visual's"),
		pb = ui.raw(12, "the visual's frame, as ds/visual's"),
		ui.row {
			w_full = true,
			min_h = 24,
			mb = ui.raw(10, "the visual's frame, as ds/visual's"),
			items = "center",
			gap = "$space.3",
			ui.text { title, literal = true, grow = 1, text_size = "$zeron.visual.text.sm.size", weight = "$font.weight.medium", color = "$text.primary" },
			ui.row { gap = "$space.1", items = "center", color = "$text.tertiary", text_size = "$font.size.xs", Icon { name = "refresh", size = 13 }, ui.text { "Reset", literal = true } },
		},
		body,
		ui.text { "The answers are in the visual: practice, not an exam.", literal = true, mt = ui.raw(10, "the caption, as ds/visual's"), text_size = "$zeron.visual.text.xs.size", color = "$text.tertiary" },
	}
end

return ui.scroll {
	id = "shot",
	w_full = true,
	h_full = true,
	bg = "$background",
	ui.column {
		id = "reply",
		w_full = true,
		max_w = 720,
		p = "$space.6",
		gap = "$space.6",
		UserMessage { id = "shot-u1", text = "Explain the quiz questions I got wrong: 1, 2.", from = "Quick check: gradient descent" },
		frame("Quick check: gradient descent", Quiz {
			questions = QUESTIONS,
			essay = { question = "Why does too large a learning rate make the loss grow?", placeholder = "A sentence or two" },
			on_send = function(text: string): boolean
				sent:push(text)
				return true
			end,
		}),
	},
}
