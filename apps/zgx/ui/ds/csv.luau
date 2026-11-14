local M = {}

local STEPS = 2000
local QUOTE, CR, LF = 34, 13, 10

export type Parser = { rows: { { string } }, columns: number, bad: number?, step: (most: number) -> boolean }

function M.parser(text: string, sep: string): Parser
	local p = { rows = {} :: { { string } }, columns = 0, bad = nil :: number? } :: any
	local sep_byte = string.byte(sep)
	local plain = `[{sep}\r\n]`
	local size = #text
	local at = if string.sub(text, 1, 3) == "\239\187\191" then 4 else 1
	local line = 1
	local function record(): { string }?
		local row: { string } = {}
		while true do
			local value: string
			if string.byte(text, at) == QUOTE then
				local from, k = at + 1, at + 1
				while true do
					local q = string.find(text, '"', k, true)
					if not q then
						p.bad = line
						return nil
					end
					if string.byte(text, q + 1) == QUOTE then
						k = q + 2
					else
						value = string.sub(text, from, q - 1)
						at = q + 1
						break
					end
				end
				if string.find(value, "\n", 1, true) then
					local _, n = string.gsub(value, "\n", "")
					line += n
				end
				if string.find(value, '""', 1, true) then
					value = string.gsub(value, '""', '"')
				end
				local after = string.byte(text, at)
				if after ~= nil and after ~= sep_byte and after ~= CR and after ~= LF then
					p.bad = line
					return nil
				end
			else
				local stop = string.find(text, plain, at) or (size + 1)
				value = string.sub(text, at, stop - 1)
				at = stop
			end
			table.insert(row, value)
			local b = string.byte(text, at)
			if b == sep_byte then
				at += 1
			else
				if b == CR then
					at += if string.byte(text, at + 1) == LF then 2 else 1
					line += 1
				elseif b == LF then
					at += 1
					line += 1
				end
				return row
			end
		end
	end
	function p.step(most: number): boolean
		for _ = 1, most do
			local b = string.byte(text, at)
			while b == CR or b == LF do
				at += if b == CR and string.byte(text, at + 1) == LF then 2 else 1
				line += 1
				b = string.byte(text, at)
			end
			if at > size then
				return true
			end
			local row = record()
			if not row then
				return true
			end
			table.insert(p.rows, row)
			if #row > p.columns then
				p.columns = #row
			end
		end
		return at > size
	end
	return p
end

function M.parse(text: string, sep: string): ({ { string } }, number?)
	local p = M.parser(text, sep)
	while not p.step(STEPS) do
	end
	return p.rows, p.bad
end

return M
