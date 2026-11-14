local Csv = require("@ds/csv")

local function rows_of(text: string, sep: string?): string
	local rows, bad = Csv.parse(text, sep or ",")
	local out: { string } = {}
	for _, row in rows do
		table.insert(out, table.concat(row, "|"))
	end
	return `{table.concat(out, "/")}{if bad then ` bad:{bad}` else ""}`
end

test("file viewer csv: quoted newlines, escaped quotes and CRLF parse as RFC 4180", function()
	expect(rows_of('a,b\r\n"x\r\ny","say ""hi"""\r\n3,\r\n')).to_be('a|b/x\r\ny|say "hi"/3|')
	expect(rows_of('a,b\n"1\n2",z\n\n4,5')).to_be("a|b/1\n2|z/4|5")
	expect(rows_of("\239\187\191h\tq\n1\t\"a,b\"\n", "\t")).to_be("h|q/1|a,b")
end)

test("file viewer csv: an unclosed quote or text after a closing one is malformed, at its line", function()
	expect(rows_of('a,b\n1,"open\n2,3\n')).to_be("a|b bad:2")
	expect(rows_of('a,b\n\n"x"y,2\n')).to_be("a|b bad:3")
	expect(rows_of('a\n"x\ny"z\n')).to_be("a bad:3")
end)
