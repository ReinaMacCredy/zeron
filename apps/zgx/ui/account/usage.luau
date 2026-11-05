-- Token usage, as the host reads it from the engine (src/usage.rs): what this
-- device's agents spent, from their own session logs.
--
--   usage.summary(days)   a promise of { hourly, sessions, unpriced, cost,
--                         tokens, cached, uncached, output, savings, models,
--                         periods } (the shapes at src/usage.rs)
--
-- A field, so a fixture answers in the host's place.

local M = {}

function M.summary(days: number): any
	return ui.call("usage.summary", { days = days })
end

return M
