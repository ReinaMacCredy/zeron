-- Whether a new chat's wallpaper shows now: the page that draws it
-- (screens/shell/page) keeps this, and what stands over the art (the
-- composer's footer chips, their backing) reads it without loading the
-- appearance settings behind it.
--
--   art.showing   a state: true while the wallpaper is drawn

return {
	showing = ui.state("shell_art_showing", false),
	window = ui.state("shell_art_window", { w = 1240, h = 800 }),
	card_left = ui.state("shell_art_card_left", 0),
	page_left = ui.state("shell_art_page_left", 0),
}
