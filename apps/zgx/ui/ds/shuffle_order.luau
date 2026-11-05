
local M = {}

M.HISTORY_LIMIT = 8

function M.remember(history: { string }, source: string): { string }
	local out = { source }
	for _, path in history do
		if path ~= source and #out < M.HISTORY_LIMIT then
			table.insert(out, path)
		end
	end
	return out
end

function M.order(candidates: { string }, history: { string }, rank: () -> number): { string }
	local cooldown = math.clamp(#candidates - 2, 1, M.HISTORY_LIMIT)
	local recency: { [string]: number } = {}
	for i, path in history do
		if i > cooldown then
			break
		end
		recency[path] = cooldown - (i - 1)
	end
	local ranks: { [string]: number } = {}
	local out = table.clone(candidates)
	for _, path in out do
		ranks[path] = rank()
	end
	table.sort(out, function(a: string, b: string): boolean
		local ra, rb = recency[a] or 0, recency[b] or 0
		if ra ~= rb then
			return ra < rb
		end
		return ranks[a] < ranks[b]
	end)
	return out
end

return M
