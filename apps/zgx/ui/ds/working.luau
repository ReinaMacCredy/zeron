-- That an agent is working on a reply, as Zeron's transcript says it under
-- the turn: the working loader, a word for it ("Noodling…") that rolls to
-- another every seven seconds, and how long it has been at it.
--
-- `Working { since = seconds }`: `since` in `ui.now() / 1000` seconds, when
-- the turn began (or a function of it); `seed` picks where the words start
-- (a chat's own). `sending`: the message is on its way, its turn not yet
-- begun: "Sending…" and no clock, as Zeron's trailer bridges that gap
-- (transcript.rs `sending_bridge`); the word and the clock start with the
-- turn.

local RollText = require("@self/roll_text")
local status = require("@self/status")
local locale = require("@self/locale")

export type Props = {
	since: number | () -> number,
	seed: number?,
	sending: (() -> boolean)?,
	says: ((sending: boolean) -> string?)?,
	rate: (() -> number?)?,
}

local SAYS: { [string]: string } = {
	starting = "ds.working.starting_cloud",
	waking = "ds.working.waking_cloud",
	consulting = "ds.working.consulting",
}

-- Zeron's words (transcript.rs, `FLAVOUR_WORDS`, their keys), each held
-- this long.
local WORDS = {
	"ds.working.words.zeroning",
	"ds.working.words.thinking",
	"ds.working.words.pondering",
	"ds.working.words.scheming",
	"ds.working.words.brewing",
	"ds.working.words.weaving",
	"ds.working.words.tinkering",
	"ds.working.words.musing",
	"ds.working.words.composing",
	"ds.working.words.sifting",
	"ds.working.words.untangling",
	"ds.working.words.distilling",
	"ds.working.words.sketching",
	"ds.working.words.plotting",
	"ds.working.words.riffing",
	"ds.working.words.combobulating",
	"ds.working.words.percolating",
	"ds.working.words.marinating",
	"ds.working.words.noodling",
	"ds.working.words.puzzling",
	"ds.working.words.conjuring",
}
local ROTATE = 7

return ui.component(function(props: Props): Node
	local now = ui.use_state(ui.now() / 1000)
	-- Once a second: the clock shows whole seconds, and the word turns on
	-- one of them.
	ui.every(1000, function()
		now:set(ui.now() / 1000)
	end)
	local seed = props.seed or 0
	local function since(): number
		local given: any = props.since
		if type(given) == "function" then
			local call: () -> number = given
			return call()
		end
		return given
	end
	local function sending(): boolean
		return props.sending ~= nil and props.sending()
	end
	local function seconds(): number
		return math.max(0, math.floor(now:get() - since()))
	end
	local function said(): string?
		return if props.says then props.says(sending()) else nil
	end
	local function rate(): number?
		return if props.rate and not sending() then props.rate() else nil
	end
	return ui.row {
		h = 26,
		gap = "$space.2",
		items = "center",
		selectable = false,
		label = locale.t("ds.working.label"),
		status.StatusIcon { kind = "working", size = 16 },
		RollText {
			function(): string
				local say = said()
				if say == "queued" then
					return locale.s("ds.working.queued")
				elseif say and SAYS[say] then
					return locale.s("ds.working.word", { word = locale.s(SAYS[say]) })
				elseif sending() then
					return locale.s("ds.working.sending")
				end
				local word = WORDS[(seed + seconds() // ROTATE) % #WORDS + 1]
				return locale.s("ds.working.word", { word = locale.s(word) })
			end,
			text_size = "$font.size.sm",
			color = function(): string
				return if said() == "queued" then "$warning" else "$text.secondary"
			end,
		},
		ui.show(function(): boolean
			return not sending()
		end, ui.div {
			id = "working-clock",
			RollText {
				function(): string
					return status.elapsed(seconds())
				end,
				tabular = true,
				text_size = "$font.size.xs",
				color = "$text.tertiary",
			},
		}),
		ui.show(function(): boolean
			return rate() ~= nil
		end, ui.text {
			function(): string
				return locale.s("ds.working.rate", { n = string.format("%d", rate() or 0) })
			end,
			id = "working-rate",
			tabular = true,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		}),
	}
end)
