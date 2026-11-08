test("session stats: the prompt cache's words, as stats_view.rs's tests have them, and the card's countdown", function()
	local app = mount("session_stats_fixture.luau")
	local words = string.split(app:state("test_words"), "|")
	local expected = {
		"13.4s", "1m00s", "59m59s", "1h00m", "2h52m", "1d00h", "2d02h", "1mo00d", "1y01mo",
		"52m00s left · 1h", "52:00 left",
		"Expired 12m00s ago", "Expired",
		"Refreshing · 1h", "Refreshing",
		"nil",
		"3:12", "0:59", "0:00", "1:00:00", "0:00",
	}
	expect(#words).to_be(#expected)
	for i, w in expected do
		expect(words[i]).to_be(w)
	end
end)
