return {
	element = nil :: any,
	moved = ui.state("composer_surface_moved", 0),
	-- (A new chat's pill: how far it's raised for the Home widgets, 0 to 1, and its foot in window points.)
	lift = ui.state("composer_surface_lift", 0),
	foot = ui.state("composer_surface_foot", 0),
}
