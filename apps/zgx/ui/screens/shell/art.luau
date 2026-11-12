-- Whether a new chat's wallpaper shows now: the page that draws it
-- (screens/shell/page) keeps this, and what stands over the art (the
-- composer's footer chips, their backing) reads it without loading the
-- appearance settings behind it.
--
--   art.showing   a state: true while the wallpaper is drawn

return {
	RAIL_W = 46,
	showing = ui.state("shell_art_showing", false),
	window = ui.state("shell_art_window", { w = 1240, h = 800 }),
	card_left = ui.state("shell_art_card_left", 0),
	page_left = ui.state("shell_art_page_left", 0),
	panel_w = ui.state("shell_files_shown_width", -1),
	panel_gap = ui.state("shell_art_panel_gap", 0),
	actions_w = ui.state("shell_titlebar_actions_w", 0),
	under_rail = ui.state("shell_art_under_rail", false),
	split = ui.state("shell_art_split", false),
	expanded = ui.state("shell_art_expanded", false),
	shade = ui.ref(),
	strip = ui.ref(),
	tint = ui.ref(),
	tab = ui.ref(),
	px = ui.state("shell_wallpaper_px", 0.5),
	py = ui.state("shell_wallpaper_py", 0.5),
}
