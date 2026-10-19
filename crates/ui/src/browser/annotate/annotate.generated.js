(()=>{var Qt=`/*! tailwindcss v4.3.3 | MIT License | https://tailwindcss.com */
@layer properties;
:root, :host {
  --spacing: 0.25rem;
  --text-xs: 0.75rem;
  --text-xs--line-height: calc(1 / 0.75);
  --text-sm: 0.875rem;
  --text-sm--line-height: calc(1.25 / 0.875);
  --text-lg: 1.125rem;
  --text-lg--line-height: calc(1.75 / 1.125);
  --font-weight-medium: 500;
  --font-weight-semibold: 600;
  --font-weight-bold: 700;
  --blur-xl: 24px;
  --default-font-family: var(--t3-font-sans);
  --default-mono-font-family: var(--t3-font-mono);
}
*, ::after, ::before, ::backdrop, ::file-selector-button {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
  border: 0 solid;
}
html, :host {
  line-height: 1.5;
  -webkit-text-size-adjust: 100%;
  tab-size: 4;
  font-family: var(--default-font-family, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', 'Noto Sans', Arial, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji', 'Segoe UI Symbol', 'Noto Color Emoji');
  font-feature-settings: var(--default-font-feature-settings, normal);
  font-variation-settings: var(--default-font-variation-settings, normal);
  -webkit-tap-highlight-color: transparent;
}
hr {
  height: 0;
  color: inherit;
  border-top-width: 1px;
}
abbr:where([title]) {
  -webkit-text-decoration: underline dotted;
  text-decoration: underline dotted;
}
h1, h2, h3, h4, h5, h6 {
  font-size: inherit;
  font-weight: inherit;
}
a {
  color: inherit;
  -webkit-text-decoration: inherit;
  text-decoration: inherit;
}
b, strong {
  font-weight: bolder;
}
code, kbd, samp, pre {
  font-family: var(--default-mono-font-family, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace);
  font-feature-settings: var(--default-mono-font-feature-settings, normal);
  font-variation-settings: var(--default-mono-font-variation-settings, normal);
  font-size: 1em;
}
small {
  font-size: 80%;
}
sub, sup {
  font-size: 75%;
  line-height: 0;
  position: relative;
  vertical-align: baseline;
}
sub {
  bottom: -0.25em;
}
sup {
  top: -0.5em;
}
table {
  text-indent: 0;
  border-color: inherit;
  border-collapse: collapse;
}
:-moz-focusring:where(:not(iframe)) {
  outline: auto;
}
progress {
  vertical-align: baseline;
}
summary {
  display: list-item;
}
ol, ul, menu {
  list-style: none;
}
img, svg, video, canvas, audio, iframe, embed, object {
  display: block;
  vertical-align: middle;
}
img, video {
  max-width: 100%;
  height: auto;
}
button, input, select, optgroup, textarea, ::file-selector-button {
  font: inherit;
  font-feature-settings: inherit;
  font-variation-settings: inherit;
  letter-spacing: inherit;
  color: inherit;
  border-radius: 0;
  background-color: transparent;
  opacity: 1;
}
:where(select:is([multiple], [size])) optgroup {
  font-weight: bolder;
}
:where(select:is([multiple], [size])) optgroup option {
  padding-inline-start: 20px;
}
::file-selector-button {
  margin-inline-end: 4px;
}
::placeholder {
  opacity: 1;
}
@supports (not (-webkit-appearance: -apple-pay-button))  or (contain-intrinsic-size: 1px) {
  ::placeholder {
    color: currentcolor;
    @supports (color: color-mix(in lab, red, red)) {
      color: color-mix(in oklab, currentcolor 50%, transparent);
    }
  }
}
textarea {
  resize: vertical;
}
::-webkit-search-decoration {
  -webkit-appearance: none;
}
::-webkit-date-and-time-value {
  min-height: 1lh;
  text-align: inherit;
}
::-webkit-datetime-edit {
  display: inline-flex;
}
::-webkit-datetime-edit-fields-wrapper {
  padding: 0;
}
::-webkit-datetime-edit, ::-webkit-datetime-edit-year-field, ::-webkit-datetime-edit-month-field, ::-webkit-datetime-edit-day-field, ::-webkit-datetime-edit-hour-field, ::-webkit-datetime-edit-minute-field, ::-webkit-datetime-edit-second-field, ::-webkit-datetime-edit-millisecond-field, ::-webkit-datetime-edit-meridiem-field {
  padding-block: 0;
}
::-webkit-calendar-picker-indicator {
  line-height: 1;
}
:-moz-ui-invalid {
  box-shadow: none;
}
button, input:where([type='button'], [type='reset'], [type='submit']), ::file-selector-button {
  appearance: button;
}
::-webkit-inner-spin-button, ::-webkit-outer-spin-button {
  height: auto;
}
[hidden]:where(:not([hidden='until-found'])) {
  display: none !important;
}
.pointer-events-auto {
  pointer-events: auto;
}
.pointer-events-none {
  pointer-events: none;
}
.absolute {
  position: absolute;
}
.fixed {
  position: fixed;
}
.inset-0 {
  inset: 0px;
}
.top-1\\/2 {
  top: calc(1 / 2 * 100%);
}
.top-2\\.5 {
  top: calc(var(--spacing) * 2.5);
}
.right-2 {
  right: calc(var(--spacing) * 2);
}
.left-1\\/2 {
  left: calc(1 / 2 * 100%);
}
.z-1 {
  z-index: 1;
}
.block {
  display: block;
}
.flex {
  display: flex;
}
.grid {
  display: grid;
}
.hidden {
  display: none;
}
.inline-flex {
  display: inline-flex;
}
.h-7 {
  height: calc(var(--spacing) * 7);
}
.h-8 {
  height: calc(var(--spacing) * 8);
}
.max-h-24 {
  max-height: calc(var(--spacing) * 24);
}
.max-h-\\[calc\\(100vh-16px\\)\\] {
  max-height: calc(100vh - 16px);
}
.max-h-\\[min\\(176px\\,calc\\(100vh-180px\\)\\)\\] {
  max-height: min(176px, calc(100vh - 180px));
}
.min-h-7 {
  min-height: calc(var(--spacing) * 7);
}
.min-h-8 {
  min-height: calc(var(--spacing) * 8);
}
.w-6 {
  width: calc(var(--spacing) * 6);
}
.w-8 {
  width: calc(var(--spacing) * 8);
}
.w-\\[min\\(360px\\,calc\\(100vw-16px\\)\\)\\] {
  width: min(360px, calc(100vw - 16px));
}
.w-full {
  width: 100%;
}
.max-w-70 {
  max-width: calc(var(--spacing) * 70);
}
.min-w-0 {
  min-width: 0px;
}
.flex-1 {
  flex: 1;
}
.shrink-0 {
  flex-shrink: 0;
}
.-translate-x-1\\/2 {
  --tw-translate-x: calc(calc(1 / 2 * 100%) * -1);
  translate: var(--tw-translate-x) var(--tw-translate-y);
}
.-translate-y-1\\/2 {
  --tw-translate-y: calc(calc(1 / 2 * 100%) * -1);
  translate: var(--tw-translate-x) var(--tw-translate-y);
}
.cursor-grab {
  cursor: grab;
}
.cursor-pointer {
  cursor: pointer;
}
.resize {
  resize: both;
}
.resize-none {
  resize: none;
}
.appearance-none {
  appearance: none;
}
.grid-cols-\\[22px_minmax\\(0\\,1fr\\)\\] {
  grid-template-columns: 22px minmax(0,1fr);
}
.grid-cols-\\[82px_minmax\\(0\\,1fr\\)\\] {
  grid-template-columns: 82px minmax(0,1fr);
}
.flex-col {
  flex-direction: column;
}
.items-center {
  align-items: center;
}
.items-start {
  align-items: flex-start;
}
.justify-center {
  justify-content: center;
}
.gap-0\\.5 {
  gap: calc(var(--spacing) * 0.5);
}
.gap-1 {
  gap: var(--spacing);
}
.gap-2 {
  gap: calc(var(--spacing) * 2);
}
.overflow-auto {
  overflow: auto;
}
.overflow-hidden {
  overflow: hidden;
}
.overflow-y-hidden {
  overflow-y: hidden;
}
.rounded-lg {
  border-radius: var(--t3-radius);
}
.rounded-md {
  border-radius: calc(var(--t3-radius) - 2px);
}
.rounded-xl {
  border-radius: calc(var(--t3-radius) + 4px);
}
.border {
  border-style: var(--tw-border-style);
  border-width: 1px;
}
.border-0 {
  border-style: var(--tw-border-style);
  border-width: 0px;
}
.border-t {
  border-top-style: var(--tw-border-style);
  border-top-width: 1px;
}
.border-b {
  border-bottom-style: var(--tw-border-style);
  border-bottom-width: 1px;
}
.border-border {
  border-color: var(--t3-border);
}
.border-input {
  border-color: var(--t3-input);
}
.border-primary {
  border-color: var(--t3-primary);
}
.border-transparent {
  border-color: transparent;
}
.border-b-transparent {
  border-bottom-color: transparent;
}
.bg-background {
  background-color: var(--t3-background);
}
.bg-muted {
  background-color: var(--t3-muted);
}
.bg-muted\\/40 {
  background-color: var(--t3-muted);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-muted) 40%, transparent);
  }
}
.bg-popover\\/95 {
  background-color: var(--t3-popover);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-popover) 95%, transparent);
  }
}
.bg-popover\\/96 {
  background-color: var(--t3-popover);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-popover) 96%, transparent);
  }
}
.bg-primary {
  background-color: var(--t3-primary);
}
.bg-primary\\/10 {
  background-color: var(--t3-primary);
  @supports (color: color-mix(in lab, red, red)) {
    background-color: color-mix(in oklab, var(--t3-primary) 10%, transparent);
  }
}
.bg-transparent {
  background-color: transparent;
}
.p-0 {
  padding: 0px;
}
.p-1 {
  padding: var(--spacing);
}
.p-2 {
  padding: calc(var(--spacing) * 2);
}
.px-0 {
  padding-inline: 0px;
}
.px-1 {
  padding-inline: var(--spacing);
}
.px-2 {
  padding-inline: calc(var(--spacing) * 2);
}
.px-2\\.5 {
  padding-inline: calc(var(--spacing) * 2.5);
}
.px-3 {
  padding-inline: calc(var(--spacing) * 3);
}
.py-1 {
  padding-block: var(--spacing);
}
.py-1\\.5 {
  padding-block: calc(var(--spacing) * 1.5);
}
.py-2 {
  padding-block: calc(var(--spacing) * 2);
}
.font-mono {
  font-family: var(--t3-font-mono);
}
.font-sans {
  font-family: var(--t3-font-sans);
}
.text-lg {
  font-size: var(--text-lg);
  line-height: var(--tw-leading, var(--text-lg--line-height));
}
.text-sm {
  font-size: var(--text-sm);
  line-height: var(--tw-leading, var(--text-sm--line-height));
}
.text-xs {
  font-size: var(--text-xs);
  line-height: var(--tw-leading, var(--text-xs--line-height));
}
.leading-5 {
  --tw-leading: calc(var(--spacing) * 5);
  line-height: calc(var(--spacing) * 5);
}
.font-bold {
  --tw-font-weight: var(--font-weight-bold);
  font-weight: var(--font-weight-bold);
}
.font-medium {
  --tw-font-weight: var(--font-weight-medium);
  font-weight: var(--font-weight-medium);
}
.font-semibold {
  --tw-font-weight: var(--font-weight-semibold);
  font-weight: var(--font-weight-semibold);
}
.text-foreground {
  color: var(--t3-foreground);
}
.text-muted-foreground {
  color: var(--t3-muted-foreground);
}
.text-popover-foreground {
  color: var(--t3-popover-foreground);
}
.text-primary {
  color: var(--t3-primary);
}
.text-primary-foreground {
  color: var(--t3-primary-foreground);
}
.shadow-2xl {
  --tw-shadow: 0 25px 50px -12px var(--tw-shadow-color, rgb(0 0 0 / 0.25));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-lg {
  --tw-shadow: 0 10px 15px -3px var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 4px 6px -4px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-md {
  --tw-shadow: 0 4px 6px -1px var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 2px 4px -2px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-sm {
  --tw-shadow: 0 1px 3px 0 var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 1px 2px -1px var(--tw-shadow-color, rgb(0 0 0 / 0.1));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.shadow-xs {
  --tw-shadow: 0 1px 2px 0 var(--tw-shadow-color, rgb(0 0 0 / 0.05));
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.ring-0 {
  --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(0px + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor);
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.blur {
  --tw-blur: blur(8px);
  filter: var(--tw-blur,) var(--tw-brightness,) var(--tw-contrast,) var(--tw-grayscale,) var(--tw-hue-rotate,) var(--tw-invert,) var(--tw-saturate,) var(--tw-sepia,) var(--tw-drop-shadow,);
}
.backdrop-blur-xl {
  --tw-backdrop-blur: blur(var(--blur-xl));
  -webkit-backdrop-filter: var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,);
  backdrop-filter: var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,);
}
.outline-none {
  --tw-outline-style: none;
  outline-style: none;
}
.select-none {
  -webkit-user-select: none;
  user-select: none;
}
.placeholder\\:text-muted-foreground::placeholder {
  color: var(--t3-muted-foreground);
}
@media (hover: hover) {
  .hover\\:bg-accent:hover {
    background-color: var(--t3-accent);
  }
  .hover\\:bg-primary\\/90:hover {
    background-color: var(--t3-primary);
  }
  @supports (color: color-mix(in lab, red, red)) {
    .hover\\:bg-primary\\/90:hover {
      background-color: color-mix(in oklab, var(--t3-primary) 90%, transparent);
    }
  }
  .hover\\:text-accent-foreground:hover {
    color: var(--t3-accent-foreground);
  }
}
.focus\\:border-b-primary:focus {
  border-bottom-color: var(--t3-primary);
}
.focus\\:ring-0:focus {
  --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(0px + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor);
  box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow);
}
.focus\\:outline-none:focus {
  --tw-outline-style: none;
  outline-style: none;
}
.disabled\\:pointer-events-none:disabled {
  pointer-events: none;
}
.disabled\\:opacity-60:disabled {
  opacity: 60%;
}
:host {
  --t3-font-sans: "DM Sans Variable", "DM Sans", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui,
    sans-serif;
  --t3-font-mono: "SF Mono", "SFMono-Regular", "JetBrains Mono", Consolas, "Liberation Mono", Menlo, monospace;
  --t3-radius: 0.625rem;
  --t3-background: white;
  --t3-foreground: oklch(0.269 0 0);
  --t3-popover: white;
  --t3-popover-foreground: oklch(0.269 0 0);
  --t3-primary: oklch(0.488 0.217 264);
  --t3-primary-foreground: white;
  --t3-muted: rgb(0 0 0 / 4%);
  --t3-muted-foreground: oklch(0.556 0 0);
  --t3-accent: rgb(0 0 0 / 4%);
  --t3-accent-foreground: oklch(0.269 0 0);
  --t3-border: rgb(0 0 0 / 8%);
  --t3-input: rgb(0 0 0 / 10%);
  --t3-ring: oklch(0.488 0.217 264);
  color: var(--t3-foreground);
  font-family: var(--t3-font-sans);
}
* {
  box-sizing: border-box;
  border-color: var(--t3-border);
}
button, input, select, textarea {
  font: inherit;
}
button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible {
  outline: 2px solid var(--t3-ring);
  @supports (color: color-mix(in lab, red, red)) {
    outline: 2px solid color-mix(in srgb, var(--t3-ring) 72%, transparent);
  }
  outline-offset: 1px;
}
@property --tw-translate-x {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-translate-y {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-translate-z {
  syntax: "*";
  inherits: false;
  initial-value: 0;
}
@property --tw-border-style {
  syntax: "*";
  inherits: false;
  initial-value: solid;
}
@property --tw-leading {
  syntax: "*";
  inherits: false;
}
@property --tw-font-weight {
  syntax: "*";
  inherits: false;
}
@property --tw-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-inset-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-inset-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-inset-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-ring-color {
  syntax: "*";
  inherits: false;
}
@property --tw-ring-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-inset-ring-color {
  syntax: "*";
  inherits: false;
}
@property --tw-inset-ring-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-ring-inset {
  syntax: "*";
  inherits: false;
}
@property --tw-ring-offset-width {
  syntax: "<length>";
  inherits: false;
  initial-value: 0px;
}
@property --tw-ring-offset-color {
  syntax: "*";
  inherits: false;
  initial-value: #fff;
}
@property --tw-ring-offset-shadow {
  syntax: "*";
  inherits: false;
  initial-value: 0 0 #0000;
}
@property --tw-blur {
  syntax: "*";
  inherits: false;
}
@property --tw-brightness {
  syntax: "*";
  inherits: false;
}
@property --tw-contrast {
  syntax: "*";
  inherits: false;
}
@property --tw-grayscale {
  syntax: "*";
  inherits: false;
}
@property --tw-hue-rotate {
  syntax: "*";
  inherits: false;
}
@property --tw-invert {
  syntax: "*";
  inherits: false;
}
@property --tw-opacity {
  syntax: "*";
  inherits: false;
}
@property --tw-saturate {
  syntax: "*";
  inherits: false;
}
@property --tw-sepia {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow-color {
  syntax: "*";
  inherits: false;
}
@property --tw-drop-shadow-alpha {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 100%;
}
@property --tw-drop-shadow-size {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-blur {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-brightness {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-contrast {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-grayscale {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-hue-rotate {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-invert {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-opacity {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-saturate {
  syntax: "*";
  inherits: false;
}
@property --tw-backdrop-sepia {
  syntax: "*";
  inherits: false;
}
@layer properties {
  @supports ((-webkit-hyphens: none) and (not (margin-trim: inline))) or ((-moz-orient: inline) and (not (color:rgb(from red r g b)))) {
    *, ::before, ::after, ::backdrop {
      --tw-translate-x: 0;
      --tw-translate-y: 0;
      --tw-translate-z: 0;
      --tw-border-style: solid;
      --tw-leading: initial;
      --tw-font-weight: initial;
      --tw-shadow: 0 0 #0000;
      --tw-shadow-color: initial;
      --tw-shadow-alpha: 100%;
      --tw-inset-shadow: 0 0 #0000;
      --tw-inset-shadow-color: initial;
      --tw-inset-shadow-alpha: 100%;
      --tw-ring-color: initial;
      --tw-ring-shadow: 0 0 #0000;
      --tw-inset-ring-color: initial;
      --tw-inset-ring-shadow: 0 0 #0000;
      --tw-ring-inset: initial;
      --tw-ring-offset-width: 0px;
      --tw-ring-offset-color: #fff;
      --tw-ring-offset-shadow: 0 0 #0000;
      --tw-blur: initial;
      --tw-brightness: initial;
      --tw-contrast: initial;
      --tw-grayscale: initial;
      --tw-hue-rotate: initial;
      --tw-invert: initial;
      --tw-opacity: initial;
      --tw-saturate: initial;
      --tw-sepia: initial;
      --tw-drop-shadow: initial;
      --tw-drop-shadow-color: initial;
      --tw-drop-shadow-alpha: 100%;
      --tw-drop-shadow-size: initial;
      --tw-backdrop-blur: initial;
      --tw-backdrop-brightness: initial;
      --tw-backdrop-contrast: initial;
      --tw-backdrop-grayscale: initial;
      --tw-backdrop-hue-rotate: initial;
      --tw-backdrop-invert: initial;
      --tw-backdrop-opacity: initial;
      --tw-backdrop-saturate: initial;
      --tw-backdrop-sepia: initial;
    }
  }
}
`;var En=(e)=>e.key==="Enter"&&!e.shiftKey&&!e.isComposing,E="data-zeron-annotation-ui",Pt="data-zeron-annotate",mt="data-zeron-annotation-tool",kn=2147483646,it="var(--t3-primary)",Nt="color-mix(in srgb, var(--t3-primary) 10%, transparent)",Mn=20,Ft=1,tn=10,lt=null,nn=0,It=null,An=(e,r)=>{if(!r)return;e.style.colorScheme=r.colorScheme;let i={"--t3-radius":r.radius,"--t3-background":r.background,"--t3-foreground":r.foreground,"--t3-popover":r.popover,"--t3-popover-foreground":r.popoverForeground,"--t3-primary":r.primary,"--t3-primary-foreground":r.primaryForeground,"--t3-muted":r.muted,"--t3-muted-foreground":r.mutedForeground,"--t3-accent":r.accent,"--t3-accent-foreground":r.accentForeground,"--t3-border":r.border,"--t3-input":r.input,"--t3-ring":r.ring,"--t3-font-sans":r.fontSans,"--t3-font-mono":r.fontMono};for(let[d,c]of Object.entries(i))e.style.setProperty(d,c)},gt=(e)=>(nn+=1,`${e}_${nn.toString(36)}`),st=(e)=>({x:e.left,y:e.top,width:e.width,height:e.height}),en=(e,r,i,d)=>({x:Math.min(e,i),y:Math.min(r,d),width:Math.abs(i-e),height:Math.abs(d-r)}),on=(e)=>e.width>=3&&e.height>=3;function rn(e,r=20){if(e.length===0)return null;let i=Math.min(...e.map((v)=>v.x)),d=Math.min(...e.map((v)=>v.y)),c=Math.max(...e.map((v)=>v.x+v.width)),L=Math.max(...e.map((v)=>v.y+v.height)),z=Math.max(0,i-r),g=Math.max(0,d-r),P=Math.max(1,window.innerWidth-z),M=Math.max(1,window.innerHeight-g);return{x:z,y:g,width:Math.min(P,c-i+r*2),height:Math.min(M,L-d+r*2)}}function J(e){return e instanceof Element&&e.closest(`[${E}]`)!==null}function an(e,r){for(let i of document.elementsFromPoint(e,r)){if(!(i instanceof Element))continue;if(J(i))continue;if(i===document.documentElement||i===document.body)continue;return i}return null}function Ln(e){let r=e.tagName.toLowerCase(),i=e.id?`#${e.id}`:"",d=e instanceof HTMLElement&&typeof e.className==="string"?e.className.trim().split(/\s+/).filter(Boolean).slice(0,2).map((c)=>`.${c}`).join(""):"";return`${r}${i}${d}`}function ht(e,r){let i=document.createElement("div");return i.setAttribute(E,""),i.style.cssText=["position:fixed","pointer-events:none",`border:2px solid ${e}`,`background:${r}`,"border-radius:3px","box-sizing:border-box","display:none",`z-index:${Ft}`].join(";"),i}function bt(e,r){e.style.display="block",e.style.transform=`translate(${r.x}px, ${r.y}px)`,e.style.width=`${r.width}px`,e.style.height=`${r.height}px`}function Sn(){let e=document.createElement("div");return e.setAttribute(E,""),e.className="fixed z-1 max-w-70 overflow-hidden rounded-md bg-primary px-2 py-1 font-sans text-xs font-semibold text-primary-foreground shadow-md",e.style.cssText=["position:fixed","pointer-events:none","white-space:nowrap","text-overflow:ellipsis",`z-index:${Ft}`].join(";"),e}function $t(e){if(!e.element.isConnected){e.outline.style.display="none",e.label.style.display="none";return}let r=e.element.getBoundingClientRect();bt(e.outline,st(r)),e.label.textContent=Ln(e.element),e.label.style.display="block";let i=Math.max(4,Math.min(r.left,window.innerWidth-e.label.offsetWidth-4));e.label.style.transform=`translate(${i}px, ${Math.max(4,r.top-22)}px)`}var Cn=500;function Tn(e){return{pageUrl:location.href,pageTitle:document.title?.trim()||null,tagName:e.tagName.toLowerCase(),pickedAt:new Date().toISOString(),selector:null,htmlPreview:e.outerHTML.slice(0,Cn),componentName:null,source:null,stack:[],styles:""}}function ft(e,r){let i=document.createElement("button");return i.type="button",i.textContent=e,i.title=r,i.className="inline-flex h-7 cursor-pointer items-center justify-center rounded-md border border-transparent px-2 font-sans text-xs font-medium text-foreground outline-none hover:bg-accent disabled:pointer-events-none disabled:opacity-60",i}function Ht(e){e.setAttribute("aria-label",e.getAttribute("aria-label")??"Style value"),e.className="h-7 min-w-0 w-full appearance-none rounded-md border border-input bg-background px-2 font-mono text-xs text-foreground shadow-xs outline-none"}function Bt(e){let r=document.createElement("div");r.style.cssText="position:relative;min-width:0";let i=document.createElement("span");return i.textContent=e.dataset.unit??"",i.className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 font-mono text-xs text-muted-foreground",r.append(e,i),r}function H(e,r){let i=document.createElement("label");i.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let d=document.createElement("span");return d.textContent=e,Ht(r),i.append(d,r instanceof HTMLInputElement&&r.dataset.unit?Bt(r):r),i}function wt(){let e=document.createElement("section");return e.className="grid gap-1 border-t border-border py-2",e}function at(e,r="0"){let i=document.createElement("input");return i.type="number",i.placeholder=r,i.style.paddingRight="30px",i.dataset.unit=e,i}function Rn(e){if(e.length===0)return"";if(e.length===1)return`M ${e[0].x} ${e[0].y} l 0.01 0.01`;let r=`M ${e[0].x} ${e[0].y}`;for(let d=1;d<e.length-1;d+=1){let c=e[d],L=e[d+1];r+=` Q ${c.x} ${c.y} ${(c.x+L.x)/2} ${(c.y+L.y)/2}`}let i=e[e.length-1];return r+=` L ${i.x} ${i.y}`,r}function zn(e,r){let i=e.map((M)=>M.x),d=e.map((M)=>M.y),c=r+3,L=Math.min(...i)-c,z=Math.min(...d)-c,g=Math.max(...i)+c,P=Math.max(...d)+c;return{x:L,y:z,width:g-L,height:P-z}}function Pn(e){lt?.teardown();let r=!1,i=!1,d=(t)=>{if(i)return;i=!0,e(JSON.stringify(t))},c=document.createElement("div");c.setAttribute(E,""),c.style.cssText=`position:fixed;inset:0;z-index:${kn};pointer-events:none`,An(c,It);let L=c.attachShadow({mode:"closed"}),z=document.createElement("style");z.textContent=Qt,L.appendChild(z);let g=document.createElement("div");g.setAttribute(E,""),g.className="fixed inset-0 font-sans text-foreground",g.style.cssText="pointer-events:none";let P=document.createElement("style");P.setAttribute(E,""),P.textContent=`html[${mt}] body, html[${mt}] body * { cursor: crosshair !important; } [${E}], [${E}] * { cursor: default !important; } [${E}] input[type=number]::-webkit-inner-spin-button, [${E}] input[type=number]::-webkit-outer-spin-button { appearance:none; margin:0; }`,document.documentElement.appendChild(P),L.appendChild(g);let M=ht(it,Nt),v=ht(it,Nt);g.append(M,v);let T=document.createElementNS("http://www.w3.org/2000/svg","svg");T.setAttribute(E,""),T.setAttribute("width","100%"),T.setAttribute("height","100%"),T.setAttribute("viewBox",`0 0 ${window.innerWidth} ${window.innerHeight}`),T.style.cssText="position:fixed;inset:0;overflow:visible;pointer-events:none",T.style.zIndex=String(Ft),g.appendChild(T);let q=document.createElement("div");q.setAttribute(E,""),q.className="pointer-events-auto fixed top-2.5 left-1/2 flex -translate-x-1/2 gap-0.5 rounded-lg border border-border bg-popover/95 p-1 text-popover-foreground shadow-lg backdrop-blur-xl",q.style.zIndex=String(tn),g.appendChild(q);let h=document.createElement("div");h.setAttribute(E,""),h.className="pointer-events-auto fixed hidden max-h-[calc(100vh-16px)] w-[min(360px,calc(100vw-16px))] flex-col overflow-hidden rounded-xl border border-border bg-popover/96 text-popover-foreground shadow-2xl backdrop-blur-xl",h.style.zIndex=String(tn),g.appendChild(h);let O=document.createElement("div");O.className="flex items-start gap-2 p-2";let x=ft("","Expand annotation editor");x.setAttribute("aria-label","Expand annotation editor"),x.setAttribute("aria-expanded","false"),x.className+=" h-8 w-8 shrink-0 bg-muted p-0 text-muted-foreground hover:bg-accent hover:text-accent-foreground",x.innerHTML='<svg viewBox="0 0 20 20" width="15" height="15" aria-hidden="true"><path d="M4 5h12M4 10h12M4 15h12M7 3v4M13 8v4M9 13v4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>',O.appendChild(x);let vt=document.createElement("span");vt.textContent="Attach, then describe the change in the chat",vt.className="min-w-0 flex-1 self-center font-sans text-xs text-muted-foreground",O.appendChild(vt);let p=document.createElement("button");p.type="button",p.textContent="⠿",p.title="Drag annotation editor",p.className="hidden h-8 w-6 shrink-0 cursor-grab select-none border-0 bg-transparent p-0 font-sans text-lg font-bold leading-5 text-muted-foreground",O.appendChild(p);let B=ft("Attach","Attach annotation and screenshot (Enter)");B.className+=" h-8 shrink-0 border-primary bg-primary px-3 text-primary-foreground shadow-sm hover:bg-primary/90",O.appendChild(B),h.appendChild(O);let F=document.createElement("div");F.className="hidden max-h-[min(176px,calc(100vh-180px))] overflow-auto border-t border-border bg-muted/40 px-3",h.appendChild(F);let u=new Map,D=[],_=[],Q=new Map,Dt=new Map,f="select",k=null,y=null,_t=!1,R=!1,tt=null,N=null,V=null,W=()=>{let t=u.size>0||D.length>0||_.length>0;h.style.display=t?"flex":"none",B.disabled=!t,B.style.opacity=t?"1":"0.45",x.disabled=!t,F.style.display=R&&u.size>0?"grid":"none",Rt()},xt=()=>{for(let[t,n]of Dt){let o=t===f;n.classList.toggle("bg-primary/10",o),n.classList.toggle("text-primary",o),n.classList.toggle("text-foreground",!o)}if(f!=="select")M.style.display="none";if(f!=="marquee")v.style.display="none";document.documentElement.setAttribute(mt,f)},yt=(t)=>{if(t.element instanceof HTMLElement||t.element instanceof SVGElement)for(let[n,o]of t.baselineStyles)if(o)t.element.style.setProperty(n,o);else t.element.style.removeProperty(n);u.delete(t.element),t.element.removeAttribute(Pt),t.outline.remove(),t.label.remove();for(let[n,o]of Q)if(o.targetId===t.id)Q.delete(n);W()},Et=(t)=>{if(u.has(t))return;let n={id:gt("element"),element:t,outline:ht(it,Nt),label:Sn(),baselineStyles:new Map};if(u.set(t,n),g.append(n.outline,n.label),$t(n),W(),R)F.style.display="grid",Ut()},ln=(t,n)=>{let o=u.get(t);if(o){yt(o);return}if(!n)for(let a of Array.from(u.values()))yt(a);Et(t)},w=(t,n)=>{for(let o of u.values()){if(!(o.element instanceof HTMLElement||o.element instanceof SVGElement))continue;if(!o.baselineStyles.has(t))o.baselineStyles.set(t,o.element.style.getPropertyValue(t));let a=`${o.id}:${t}`,s=Q.get(a)?.previousValue??getComputedStyle(o.element).getPropertyValue(t).trim();o.element.style.setProperty(t,n,"important"),Q.set(a,{targetId:o.id,selector:null,property:t,previousValue:s,value:n}),$t(o)}},nt=wt(),dt=wt(),ct=wt(),kt=wt();F.append(nt,dt,ct,kt);let j=document.createElement("select");for(let t of["inherit","system-ui","sans-serif","serif","monospace"]){let n=document.createElement("option");n.value=t,n.textContent=t,j.appendChild(n)}j.addEventListener("change",()=>w("font-family",j.value)),nt.appendChild(H("Font",j));let I=at("px","16");I.min="1",I.max="300",I.addEventListener("input",()=>{if(I.value)w("font-size",`${I.value}px`)}),nt.appendChild(H("Font size",I));let et=document.createElement("select");for(let t of["300","400","500","600","700","800","900"]){let n=document.createElement("option");n.value=t,n.textContent=t,et.appendChild(n)}et.addEventListener("change",()=>w("font-weight",et.value)),nt.appendChild(H("Font weight",et));let Y=document.createElement("input");Y.type="text",Y.placeholder="normal / 1.4",Y.addEventListener("change",()=>{if(Y.value.trim())w("line-height",Y.value.trim())}),nt.appendChild(H("Line height",Y));let Mt=(t,n,o)=>{let a=document.createElement("label");a.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let s=document.createElement("span");s.textContent=t;let b=document.createElement("div");b.className="grid h-7 grid-cols-[22px_minmax(0,1fr)] items-center gap-1 rounded-md border border-input bg-background px-1 shadow-xs";let m=document.createElement("input");m.type="color",m.setAttribute("aria-label",t),m.style.cssText="width:20px;height:20px;padding:0;border:0;border-radius:5px;overflow:hidden;background:transparent;cursor:pointer";let l=document.createElement("input");return l.type="text",l.setAttribute("aria-label",`${t} value`),l.className="min-w-0 w-full border-0 bg-transparent font-mono text-xs text-foreground outline-none",m.addEventListener("input",()=>{l.value=m.value,w(n,m.value)}),l.addEventListener("change",()=>{let Z=l.value.trim();if(!Z)return;if(w(n,Z),/^#[0-9a-f]{6}$/i.test(Z))m.value=Z}),b.append(m,l),a.append(s,b),o.appendChild(a),{row:a,color:m,text:l}},dn=Mt("Text color","color",dt),cn=Mt("Background","background-color",dt),S=document.createElement("input");S.type="range",S.min="0",S.max="1",S.step="0.05",S.value="1",S.style.accentColor=it,S.addEventListener("input",()=>w("opacity",S.value)),dt.appendChild(H("Opacity",S));let U=at("px","0");U.min="0",U.max="300",U.addEventListener("input",()=>{if(U.value)w("border-radius",`${U.value}px`)}),ct.appendChild(H("Radius",U));let pn=Mt("Border color","border-color",ct),X=at("px","0");X.min="0",X.max="100",X.addEventListener("input",()=>{if(X.value)w("border-style","solid"),w("border-width",`${X.value}px`)}),ct.appendChild(H("Border width",X));let At=document.createElement("div");At.style.cssText="display:grid;grid-template-columns:82px minmax(0,1fr);gap:8px;align-items:center";let Lt=document.createElement("div");Lt.className="grid gap-2 font-sans text-xs font-medium text-muted-foreground",Lt.innerHTML="<span>Width</span><span>Height</span>";let St=document.createElement("div");St.style.cssText="position:relative;display:grid;gap:3px;padding-left:22px";let K=at("px","auto"),G=at("px","auto");Ht(K),Ht(G);let A=ft("","Lock aspect ratio");A.setAttribute("aria-pressed","true"),A.style.cssText+=";position:absolute;left:0;top:50%;transform:translateY(-50%);width:18px;height:38px;padding:0",A.className+=" bg-primary/10 text-primary",St.append(Bt(K),Bt(G),A),At.append(Lt,St),kt.appendChild(At);let C=!0,ot=1,Yt=()=>{A.innerHTML=C?'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="M8 6.5 9.5 5A3.5 3.5 0 0 1 14.5 10l-1.5 1.5M12 13.5 10.5 15A3.5 3.5 0 0 1 5.5 10L7 8.5M7.5 12.5l5-5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>':'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="m6 6 8 8M8 6.5 9.5 5A3.5 3.5 0 0 1 14 9M12 13.5 10.5 15A3.5 3.5 0 0 1 6 11" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',A.setAttribute("aria-pressed",String(C)),A.classList.toggle("bg-primary/10",C),A.classList.toggle("text-primary",C),A.classList.toggle("bg-muted",!C),A.classList.toggle("text-muted-foreground",!C)};A.addEventListener("click",()=>{C=!C,Yt()}),K.addEventListener("input",()=>{let t=Number(K.value);if(!Number.isFinite(t)||t<=0)return;if(w("width",`${t}px`),C&&ot>0){let n=Math.max(1,Math.round(t/ot));G.value=String(n),w("height",`${n}px`)}}),G.addEventListener("input",()=>{let t=Number(G.value);if(!Number.isFinite(t)||t<=0)return;if(w("height",`${t}px`),C&&ot>0){let n=Math.max(1,Math.round(t*ot));K.value=String(n),w("width",`${n}px`)}}),Yt();let Ct=(t,n,o)=>{let a=document.createElement("input");return a.type="text",a.placeholder=o,a.addEventListener("change",()=>{if(a.value.trim())w(n,a.value.trim())}),kt.appendChild(H(t,a)),a},un=Ct("Padding","padding","0 0 0 0"),mn=Ct("Margin","margin","0 0 0 0"),gn=Ct("Gap","gap","0px"),Ut=()=>{let t=u.values().next().value;if(!t)return;let n=getComputedStyle(t.element),o=t.element.getBoundingClientRect();ot=o.height>0?o.width/o.height:1,K.value=String(Math.round(o.width)),G.value=String(Math.round(o.height)),I.value=String(Math.round(Number.parseFloat(n.fontSize)||16)),et.value=n.fontWeight.match(/^[0-9]+$/)?n.fontWeight:"400",Y.value=n.lineHeight,j.value=Array.from(j.options).some((a)=>a.value===n.fontFamily)?n.fontFamily:"inherit",dn.text.value=n.color,cn.text.value=n.backgroundColor,pn.text.value=n.borderColor,S.value=n.opacity,U.value=String(Math.round(Number.parseFloat(n.borderRadius)||0)),X.value=String(Math.round(Number.parseFloat(n.borderWidth)||0)),un.value=n.padding,mn.value=n.margin,gn.value=n.gap==="normal"?"0px":n.gap},hn=[["select","Select","Select elements (V)"],["marquee","Region","Draw a region or marquee-select elements (R)"],["draw","Draw","Draw freehand (D)"],["erase","Erase","Remove an annotation target (E)"]];for(let[t,n,o]of hn){let a=ft(n,o);a.className+=" h-8 px-2.5 text-sm",a.addEventListener("click",()=>{f=t,xt()}),Dt.set(t,a),q.appendChild(a)}let fn=(t,n)=>{let a=h.getBoundingClientRect();return{left:Math.min(Math.max(8,t),Math.max(8,window.innerWidth-a.width-8)),top:Math.min(Math.max(8,n),Math.max(8,window.innerHeight-a.height-8))}},Tt=(t)=>{let n=fn(t.left,t.top);if(h.style.left=`${n.left}px`,h.style.top=`${n.top}px`,h.style.right="auto",h.style.bottom="auto",R)tt=n},wn=()=>rn([...Array.from(u.values(),(t)=>st(t.element.getBoundingClientRect())),...D.map((t)=>t.rect),..._.map((t)=>t.bounds)],0),bn=()=>{let t=wn();if(!t)return;let n=h.getBoundingClientRect(),o=8,a=[{left:t.x+t.width+o,top:t.y},{left:t.x-n.width-o,top:t.y},{left:t.x+t.width-n.width,top:t.y+t.height+o},{left:t.x+t.width-n.width,top:t.y-n.height-o}],s=(m)=>Math.max(0,-m.left)+Math.max(0,-m.top)+Math.max(0,m.left+n.width-window.innerWidth)+Math.max(0,m.top+n.height-window.innerHeight),b=a.reduce((m,l)=>s(l)<s(m)?l:m);Tt(b)};function Rt(){if(V!==null)window.cancelAnimationFrame(V);V=window.requestAnimationFrame(()=>{if(V=null,h.style.display==="none")return;if(R&&tt)Tt(tt);else bn()})}x.addEventListener("click",()=>{if(u.size===0)return;if(!R){let t=h.getBoundingClientRect();if(R=!0,tt={left:t.left,top:t.top},F.style.display=u.size>0?"grid":"none",p.style.display="block",x.setAttribute("aria-expanded","true"),x.title="Collapse annotation editor",x.setAttribute("aria-label","Collapse annotation editor"),u.size>0)Ut()}else R=!1,tt=null,F.style.display="none",p.style.display="none",x.setAttribute("aria-expanded","false"),x.title="Expand annotation editor",x.setAttribute("aria-label","Expand annotation editor");Rt()});let Xt=(t)=>{if(t.button!==0||!R)return;let n=h.getBoundingClientRect();N={pointerId:t.pointerId,offsetX:t.clientX-n.left,offsetY:t.clientY-n.top},p.setPointerCapture(t.pointerId),p.style.cursor="grabbing",t.preventDefault(),t.stopPropagation()},qt=(t)=>{if(!N||N.pointerId!==t.pointerId)return;Tt({left:t.clientX-N.offsetX,top:t.clientY-N.offsetY}),t.preventDefault(),t.stopPropagation()},pt=(t)=>{if(!N||N.pointerId!==t.pointerId)return;if(N=null,p.style.cursor="grab",p.hasPointerCapture(t.pointerId))p.releasePointerCapture(t.pointerId);t.preventDefault(),t.stopPropagation()};p.addEventListener("pointerdown",Xt),p.addEventListener("pointermove",qt),p.addEventListener("pointerup",pt),p.addEventListener("pointercancel",pt);let ut=()=>{for(let t of u.values())$t(t);Rt()},vn=(t,n)=>{for(let s of Array.from(u.values()).toReversed()){let b=s.element.getBoundingClientRect();if(t>=b.left&&t<=b.right&&n>=b.top&&n<=b.bottom)return yt(s),!0}let o=D.findIndex((s)=>t>=s.rect.x&&t<=s.rect.x+s.rect.width&&n>=s.rect.y&&n<=s.rect.y+s.rect.height);if(o>=0){let[s]=D.splice(o,1);return g.querySelector(`[data-region-id="${s?.id}"]`)?.remove(),W(),!0}let a=_.findIndex((s)=>t>=s.bounds.x&&t<=s.bounds.x+s.bounds.width&&n>=s.bounds.y&&n<=s.bounds.y+s.bounds.height);if(a>=0){let[s]=_.splice(a,1);return T.querySelector(`[data-stroke-id="${s?.id}"]`)?.remove(),W(),!0}return!1},xn=(t)=>{let n=Array.from(document.querySelectorAll("body *")).filter((o)=>!J(o)).map((o)=>({element:o,rect:o.getBoundingClientRect()})).filter(({rect:o})=>{if(o.width<2||o.height<2)return!1;return!(o.right<t.x||o.left>t.x+t.width||o.bottom<t.y||o.top>t.y+t.height)}).filter(({element:o,rect:a})=>{let s=a.left+a.width/2,b=a.top+a.height/2;return s>=t.x&&s<=t.x+t.width&&b>=t.y&&b<=t.y+t.height&&(o.children.length===0||o instanceof HTMLButtonElement||o instanceof HTMLAnchorElement||o.getAttribute("role")==="button")}).sort((o,a)=>o.rect.width*o.rect.height-a.rect.width*a.rect.height).slice(0,Mn);for(let o of n)Et(o.element);return n.length},rt=()=>{M.style.display="none"},Ot=(t)=>{if(J(t.target)){rt();return}if(f==="select"&&k===null){let n=an(t.clientX,t.clientY);if(n)bt(M,st(n.getBoundingClientRect()));else rt();return}if(rt(),f==="marquee"&&k){bt(v,en(k.x,k.y,t.clientX,t.clientY));return}if(f==="draw"&&y)y.target.points=[...y.target.points,{x:t.clientX,y:t.clientY}],y.target.bounds=zn(y.target.points,y.target.width),y.path.setAttribute("d",Rn(y.target.points))},Vt=(t)=>{if(!t.isTrusted||t.button!==0||J(t.target))return;if(t.preventDefault(),t.stopPropagation(),f==="select"){let n=an(t.clientX,t.clientY);if(n)ln(n,t.shiftKey);return}if(f==="erase"){vn(t.clientX,t.clientY);return}if(k={x:t.clientX,y:t.clientY},f==="draw"){let n={id:gt("stroke"),color:It?.primary??"#2563eb",width:4,points:[k],bounds:{x:k.x,y:k.y,width:1,height:1}},o=document.createElementNS("http://www.w3.org/2000/svg","path");o.setAttribute(E,""),o.setAttribute("data-stroke-id",n.id),o.setAttribute("fill","none"),o.setAttribute("stroke",n.color),o.setAttribute("stroke-width",String(n.width)),o.setAttribute("stroke-linecap","round"),o.setAttribute("stroke-linejoin","round"),T.appendChild(o),y={target:n,path:o}}},Wt=(t)=>{if(!k)return;if(t.preventDefault(),t.stopPropagation(),f==="marquee"){let n=en(k.x,k.y,t.clientX,t.clientY);if(v.style.display="none",on(n)){if(xn(n)===0){let a={id:gt("region"),rect:n};D.push(a);let s=ht(it,"color-mix(in srgb, var(--t3-primary) 6%, transparent)");s.setAttribute("data-region-id",a.id),bt(s,n),g.appendChild(s)}}}else if(f==="draw"&&y){if(y.target.points.length>1)_.push(y.target);else y.path.remove();y=null}k=null,W()},jt=(t)=>{if(J(t.target))return;t.preventDefault(),t.stopPropagation()},Kt=(t)=>{if(t.relatedTarget===null)rt()},Gt=()=>{rt()},yn=()=>{for(let t of u.values()){if(t.element.removeAttribute(Pt),!(t.element instanceof HTMLElement||t.element instanceof SVGElement))continue;for(let[n,o]of t.baselineStyles)if(o)t.element.style.setProperty(n,o);else t.element.style.removeProperty(n)}},Zt=()=>{if(r)return;if(r=!0,yn(),window.removeEventListener("pointermove",Ot,!0),window.removeEventListener("pointerdown",Vt,!0),window.removeEventListener("pointerup",Wt,!0),window.removeEventListener("pointerout",Kt,!0),window.removeEventListener("click",jt,!0),window.removeEventListener("blur",Gt),window.removeEventListener("keydown",Jt,!0),window.removeEventListener("scroll",ut,!0),window.removeEventListener("resize",ut),p.removeEventListener("pointerdown",Xt),p.removeEventListener("pointermove",qt),p.removeEventListener("pointerup",pt),p.removeEventListener("pointercancel",pt),V!==null)window.cancelAnimationFrame(V);document.documentElement.removeAttribute(mt),P.remove(),c.remove(),lt=null,d(null)},Jt=(t)=>{if(!t.isTrusted)return;if(J(t.target)&&t.key!=="Escape")return;if(t.key==="Escape"){t.preventDefault(),t.stopPropagation(),Zt();return}if(En(t)){t.preventDefault(),t.stopPropagation(),zt();return}if(t.key==="v")f="select";else if(t.key==="r")f="marquee";else if(t.key==="d")f="draw";else if(t.key==="e")f="erase";else return;xt()},zt=()=>{if(_t||u.size===0&&D.length===0&&_.length===0)return;_t=!0,B.disabled=!0,B.textContent="Capturing…";let t=[...D],n=[..._],o=Array.from(Q.values(),(l)=>({...l})),a=Array.from(u.values(),(l)=>{let Z=Tn(l.element);return l.element.setAttribute(Pt,l.id),{id:l.id,element:Z,rect:st(l.element.getBoundingClientRect())}}),s={id:gt("annotation"),pageUrl:location.href,pageTitle:document.title?.trim()||null,elements:a,regions:t,strokes:n,styleChanges:o,screenshot:null,createdAt:new Date().toISOString()};h.style.display="none",q.style.display="none",M.style.display="none";let b=Array.from(u.values(),(l)=>st(l.label.getBoundingClientRect())).filter(on),m=rn([...a.map((l)=>l.rect),...b,...t.map((l)=>l.rect),...n.map((l)=>l.bounds)]);window.requestAnimationFrame(()=>d({annotation:s,cropRect:m}))};B.addEventListener("click",(t)=>{if(t.isTrusted)zt()}),g.addEventListener("keydown",(t)=>t.stopImmediatePropagation()),window.addEventListener("pointermove",Ot,{capture:!0,passive:!1}),window.addEventListener("pointerdown",Vt,{capture:!0,passive:!1}),window.addEventListener("pointerup",Wt,{capture:!0,passive:!1}),window.addEventListener("pointerout",Kt,{capture:!0,passive:!0}),window.addEventListener("click",jt,{capture:!0,passive:!1}),window.addEventListener("blur",Gt),window.addEventListener("keydown",Jt,{capture:!0}),window.addEventListener("scroll",ut,{capture:!0,passive:!0}),window.addEventListener("resize",ut,{passive:!0}),document.documentElement.appendChild(c),xt(),W(),lt={teardown:Zt,demo:{select:Et,fontSize:(t)=>{if(!R)x.click();I.value=String(t),w("font-size",`${t}px`)},submit:zt}}}var sn=globalThis;sn.__zeronAnnotate?.teardown();sn.__zeronAnnotate={start(e){return It=e,new Promise((r)=>Pn(r))},teardown(){lt?.teardown()},demo(){return lt?.demo}};})();
