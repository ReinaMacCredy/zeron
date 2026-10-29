[Settings > Appearance](zeron://settings/appearance) controls how Zeron looks: light or dark, themes, fonts, glass, and background art.

## Color scheme and themes

Pick **System**, **Light**, or **Dark** at the top of the page. **System** follows macOS. The same choice is in the menu bar under **View** (**Appearance: System**, **Appearance: Light**, **Appearance: Dark**), and in the command palette.

![Theme and accent settings](appearance-themes.jpg)

**Light theme** and **Dark theme** choose the theme used for each mode, so you can pair, say, GitHub Light with Tokyo Night. The defaults are Zeron Light and Zeron Dark. Zeron ships many themes, including Catppuccin, Dracula, Nord, Gruvbox, One Dark Pro, and Rosé Pine.

**Accent color** offers **Theme default** or a preset: Zeron, Orange, Amber, Green, Cyan, Blue, or Pink. Turn on **Match wallpaper colors** to take accents and subtle tints for surfaces and borders from your background image instead. It only has an effect while a **New thread background** is set; your accent color comes back when it is off.

## Material and background

- **Glass**: **Frosted** for translucent surfaces, **Opaque** for solid ones, or **Theme default**.
- **Glass tint** and **Dim bright backdrops**: in dark mode with frosted glass, on macOS and Windows only. **Glass tint** runs from 50% to 100% (80% by default). **Dim bright backdrops**, on by default, keeps the glass dark over light windows behind Zeron; while it is on, **Backdrop brightness** (10% to 100%, 30% by default) sets how much of those windows still shows through.
- **Inset card**: hold the sidebar and content in one rounded card above the window background. On by default.
- **Reveal sidebar on hover**: show the collapsed sidebar when the pointer reaches the left edge. On by default.

### Background art

**New thread background** sets an image behind the new chat canvas. Click **Choose image**; once set, use **Adjust** to drag and zoom it, **Replace image**, or **Remove**.

![A new chat background with an effect](appearance-background.jpg)

**Wallpaper folder** points at a folder of images. **Shuffle**, or Cmd+U (**Random wallpaper**), picks a new background from it.

With a background set, more rows appear:

- **Background effect**: **None**, **Dither**, **ASCII**, **Halftone**, or **Scanlines**.
- **Chat background**: show the artwork behind open conversations (off by default), with **Placement** (**Hero** or **Full**), **Opacity** (20% by default), **Blur** (24 px by default), and **Scrim** to keep text readable. The artwork also shows through the titlebar and the selected tab.
- **Terminal background**: **Same as chat** (the default), **Custom**, or **Off**. **Custom** gives the terminal its own **Placement**, **Opacity**, **Blur**, and **Scrim**, starting from your chat settings.

## Motion

**Reduce motion** follows the system by default, or set it **On** or **Off**. **Pause animations in background** holds animations still while Zeron is not the focused window.

## Theme library

To use your own theme, click **Add theme** under **Theme library**. Zeron reads VS Code themes from a theme file, `package.json`, or extension folder. **Import a copy** keeps a snapshot; **Link to source** reloads it when the file changes.

Added themes are listed under **Imported** and **Linked**. Each one has **Reveal**, **Review**, **Duplicate as editable**, and **Remove**; linked themes also have **Reload** and **Unlink**. **Review** opens **Theme mapping**, which previews each variant and shows how its VS Code colors were mapped.

## Fonts and layout

Set the family and size for **Interface font**, **Terminal font**, and **Code & diff font**. Geist is the interface default, Geist Mono the default for terminal and code. **Conversation width** sets the maximum width for messages and the composer.

### Math

Zeron typesets LaTeX math in messages, Markdown file previews, and pull request descriptions and comments. Three rows at the end of the fonts card control it:

- **Render math**: on by default. Turn it off to show each formula's source as code instead: inline code for a formula in a line, a `math` code block for one on its own line.
- **Math font**: **Fira Math** (the default), **STIX Two**, **Computer Modern**, **Libertinus**, **Palatino**, or **Euler**. All six are built into Zeron, so they look the same on every Mac.
- **Math size**: 90%, 100% (the default), 110%, 120%, 135%, or 150% of the text around the formula.

These read as math: `$…$` or `\(…\)` for a formula inside a line, and `$$…$$` or `\[…\]` for one on its own line. Dollar signs in prices and shell variables stay text, so `$5`, `costs $5 and $10 total`, `$HOME/$USER`, and `echo $PATH and $HOME` show as written, and so does a closing `$` followed by a digit, as in `$x$5`. Nothing inside code spans or code blocks is math. A formula Zeron cannot typeset shows its source as code, as with **Render math** off. Copying a formula copies its source, with its `$` or `$$` delimiters.

> **Compact mode** is in [Settings > Chat](zeron://settings/chat), not here. It collapses thinking and tool calls so only the replies stay in view. For one-line sidebar rows, use **Compact** in the sidebar's **…** menu.
