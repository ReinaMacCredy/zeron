return {
	element = nil :: any,
	moved = ui.state("composer_surface_moved", 0),
	-- (A new chat's pill: how far it's raised for the Home widgets, 0 to 1, and its foot in window points.)
	lift = ui.state("composer_surface_lift", 0),
	foot = ui.state("composer_surface_foot", 0),
	-- (Arriving on a new chat or on a session, how much of that page's own shows yet, 0 to 1.)
	hero = ui.state("composer_surface_hero", 1),
	thread = ui.state("composer_surface_thread", 1),
	-- (A send's page takes two long frames to come in; started under them, the composer's spring makes its second step three frames' worth.)
	TAKEOFF = 34,
}
