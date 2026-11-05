-- crates/ui shell/switcher.rs `session_thumbs`: slots only, so tests read it alone.

local M = {}

function M.Book(most: number): any
	local pictures: { [string]: { src: string, slot: string } } = {}
	local used: { string } = {}
	local book = {}
	-- (A failed `take` leaves all as it was: no slot freed while another may hold its number.)
	function book.take(id: string, take: (slot: string) -> string?)
		local had = pictures[id]
		local oldest: string? = nil
		local slot: string
		if had then
			slot = had.slot
		elseif #used < most then
			slot = tostring(#used + 1)
		else
			oldest = used[1]
			slot = pictures[oldest :: string].slot
		end
		local src = take(slot)
		if not src then
			return
		end
		if oldest then
			pictures[oldest] = nil
			table.remove(used, 1)
		elseif had then
			table.remove(used, table.find(used, id) :: number)
		end
		pictures[id] = { src = src, slot = slot }
		table.insert(used, id)
	end
	function book.src(id: string): string?
		local p = pictures[id]
		return if p then p.src else nil
	end
	return book
end

return M
