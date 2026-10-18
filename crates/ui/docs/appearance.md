[Settings > Appearance](zeron://settings/appearance) controls how Zeron looks: light or dark, themes, fonts, glass, and background art.

## Color scheme and themes

Pick **System**, **Light**, or **Dark** at the top of the page. **System** follows macOS. The same choice is in the menu bar under **View > Appearance**, and in the command palette.

**Light theme** and **Dark theme** choose the theme used for each mode, so you can pair, say, GitHub Light with Tokyo Night. Zeron ships many themes, including Catppuccin, Dracula, Nord, Gruvbox, One Dark Pro, and Rosé Pine.

**Accent color** offers **Theme default** or a preset: Zeron, Orange, Amber, Green, Cyan, Blue, or Pink. Turn on **Match wallpaper colors** to take accents and subtle tints from your background image instead.

To use your own theme, click **Add theme** under **Theme library**. Zeron reads VS Code themes from a theme file, `package.json`, or extension folder. **Import a copy** keeps a snapshot; **Link to source** reloads it when the file changes.

## Glass and layout

- **Glass**: **Frosted** for translucent surfaces, **Opaque** for solid ones, or **Theme default**. In dark mode with frosted glass you also get **Glass tint** and **Dim bright backdrops**, which keeps the glass dark over light windows behind Zeron.
- **Inset card**: hold the sidebar and content in one rounded card above the window background.
- **Reveal sidebar on hover**: show the collapsed sidebar when the pointer reaches the left edge.

## Background art

**New thread background** sets an image behind the new chat canvas. Click **Choose image**; once set, use **Adjust** to drag and zoom it, **Replace image**, or **Remove**.

**Wallpaper folder** points at a folder of images. **Shuffle**, or Cmd+U (**Random wallpaper**), picks a new background from it.

With a background set, more rows appear:

- **Background effect**: **None**, **Dither**, **ASCII**, **Halftone**, or **Scanlines**.
- **Chat background**: show the artwork behind open conversations, with **Placement** (**Hero** or **Full**), **Opacity**, **Blur**, and **Scrim** to keep text readable.
- **Terminal background**: **Same as chat**, **Custom**, or **Off**.

## Motion

**Reduce motion** follows the system by default, or set it **On** or **Off**. **Pause animations in background** holds animations still while Zeron is not the focused window.

## Fonts and width

Set the family and size for **Interface font**, **Terminal font**, and **Code & diff font**. Geist is the interface default, Geist Mono the default for terminal and code. **Conversation width** sets the maximum width for messages and the composer.

> **Compact mode** is in [Settings > General](zeron://settings/general), not here. It collapses thinking and tool calls so only the replies stay in view. For one-line sidebar rows, use **Compact** in the sidebar's **…** menu.
