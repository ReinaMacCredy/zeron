local K = require("@app/screens/shell/whats_new_visuals/kit")

local NEW_AT, MADE_AT, PIN_AT = 0.8, 2.2, 3.2

local function header(label: string): Node
	return ui.row { h = 16, px = K.raw(5), items = "center", K.text(9.5, "$text.tertiary", label) }
end

local function rail_row(label: string): Node
	return ui.row {
		h = 20,
		px = K.raw(5),
		items = "center",
		gap = K.raw(5),
		K.glyph("folder", 10, "$text.secondary"),
		K.clip(10.5, "$text.primary", label),
	}
end

local function folder(label: string, props: { [any]: any }?): Node
	local row: { [any]: any } = props or {}
	row.h = 22
	row.px = K.sp(6)
	row.items = "center"
	row.gap = K.sp(6)
	table.insert(row, 1, K.glyph("folder", 11, "$text.secondary"))
	table.insert(row, 2, K.clip(11, "$text.primary", label))
	return ui.row(row)
end

local function build(clock: K.Clock): Node
	local made = ui.computed(function(): boolean
		return clock.t() >= MADE_AT
	end)
	local function open(): number
		return K.ramp(clock.t(), NEW_AT, 200, K.EASE_OUT)
	end
	local function pinned(): number
		return K.ramp(clock.t(), PIN_AT, 240, K.EASE_OUT)
	end
	local function rise(): number
		return K.ramp(clock.t(), PIN_AT, 220, K.EASE_OUT)
	end
	local function name(): string
		return K.typed("search", clock.t(), NEW_AT + 0.3, 0.12)
	end
	local rail = ui.column {
		shrink = 0,
		w = 92,
		p = K.sp(6),
		border_sides = { 0, 1, 0, 0 },
		border_color = "$border",
		header("Pinned"),
		rail_row("shop"),
		ui.show(function(): boolean
			return pinned() > 0
		end, ui.div {
			y = function(): number
				return (1 - rise()) * 6
			end,
			opacity = rise,
			rail_row("search"),
		}),
		header("Recent"),
		rail_row("notes"),
	}
	local new_row = ui.div {
		relative = true,
		shrink = 0,
		h = function(): number
			return 24 * open()
		end,
		hidden = function(): boolean
			return open() <= 0
		end,
		overflow = "hidden",
		opacity = open,
		ui.div {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			ui.switch(function(): string
				return if made:get() then "made" else "naming"
			end, {
				made = folder("search", {
					rounded = K.r(6),
					bg = "$zeron.whats_new.wash_08",
					K.glyph("pin", 10, function(): string
						return if pinned() > 0 then "$accent" else "$text.tertiary"
					end),
				}),
				naming = ui.row {
					h = 22,
					px = K.sp(6),
					items = "center",
					gap = K.sp(6),
					rounded = K.r(6),
					border = 1,
					border_color = "$zeron.whats_new.accent_rule",
					K.glyph("lucide-folder-plus", 11, "$text.secondary"),
					K.text(11, "$text.primary", name),
					K.caret(clock),
				},
			}),
		},
	}
	local footer = ui.row {
		h = 30,
		px = K.sp(6),
		items = "center",
		gap = K.raw(5),
		border_sides = { 1, 0, 0, 0 },
		border_color = "$border",
		K.glyph("lucide-folder-plus", 10, "$text.secondary"),
		ui.div { grow = 1, min_w = 0, K.text(10, "$text.secondary", "New folder") },
		ui.row {
			shrink = 0,
			h = 20,
			px = K.sp(8),
			items = "center",
			rounded = K.r(6),
			bg = "$text.primary",
			K.text(10, "$background", function(): string
				return if made:get() then "Add search" else `Create and add {name()}`
			end),
		},
	}
	return ui.row(K.surface {
		w = 320,
		h = 128,
		shrink = 0,
		overflow = "hidden",
		rail,
		ui.column {
			grow = 1,
			min_w = 0,
			ui.column { grow = 1, p = K.raw(5), new_row, folder("docs"), folder("site") },
			footer,
		},
	})
end

return { period = 6.0, rest = 4.4, build = build }
