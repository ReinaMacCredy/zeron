(()=>{var nn=`/*! tailwindcss v4.3.3 | MIT License | https://tailwindcss.com */
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
`;var Mn=(e)=>{if(e.key!=="Enter"||e.shiftKey||e.isComposing)return null;return e.metaKey||e.ctrlKey?"send":"attach"},k="data-zeron-annotation-ui",Pt="data-zeron-annotate",gt="data-zeron-annotation-tool",Sn=2147483646,at="var(--t3-primary)",Nt="color-mix(in srgb, var(--t3-primary) 10%, transparent)",Ln=20,Bt=1,en=10,lt=null,on=0,Ht=null,An=(e,r)=>{if(!r)return;e.style.colorScheme=r.colorScheme;let i={"--t3-radius":r.radius,"--t3-background":r.background,"--t3-foreground":r.foreground,"--t3-popover":r.popover,"--t3-popover-foreground":r.popoverForeground,"--t3-primary":r.primary,"--t3-primary-foreground":r.primaryForeground,"--t3-muted":r.muted,"--t3-muted-foreground":r.mutedForeground,"--t3-accent":r.accent,"--t3-accent-foreground":r.accentForeground,"--t3-border":r.border,"--t3-input":r.input,"--t3-ring":r.ring,"--t3-font-sans":r.fontSans,"--t3-font-mono":r.fontMono};for(let[l,d]of Object.entries(i))e.style.setProperty(l,d)},ht=(e)=>(on+=1,`${e}_${on.toString(36)}`),vt=(e)=>({x:e.left,y:e.top,width:e.width,height:e.height}),rn=(e,r,i,l)=>({x:Math.min(e,i),y:Math.min(r,l),width:Math.abs(i-e),height:Math.abs(l-r)}),Cn=(e)=>e.width>=3&&e.height>=3;function an(e,r=20){if(e.length===0)return null;let i=Math.min(...e.map((v)=>v.x)),l=Math.min(...e.map((v)=>v.y)),d=Math.max(...e.map((v)=>v.x+v.width)),C=Math.max(...e.map((v)=>v.y+v.height)),N=Math.max(0,i-r),g=Math.max(0,l-r),H=Math.max(1,window.innerWidth-N),L=Math.max(1,window.innerHeight-g);return{x:N,y:g,width:Math.min(H,d-i+r*2),height:Math.min(L,C-l+r*2)}}function Q(e){return e instanceof Element&&e.closest(`[${k}]`)!==null}function sn(e,r){for(let i of document.elementsFromPoint(e,r)){if(!(i instanceof Element))continue;if(Q(i))continue;if(i===document.documentElement||i===document.body)continue;return i}return null}function Tn(e){let r=e.tagName.toLowerCase(),i=e.id?`#${e.id}`:"",l=e instanceof HTMLElement&&typeof e.className==="string"?e.className.trim().split(/\s+/).filter(Boolean).slice(0,2).map((d)=>`.${d}`).join(""):"";return`${r}${i}${l}`}function ft(e,r){let i=document.createElement("div");return i.setAttribute(k,""),i.style.cssText=["position:fixed","pointer-events:none",`border:2px solid ${e}`,`background:${r}`,"border-radius:3px","box-sizing:border-box","display:none",`z-index:${Bt}`].join(";"),i}function xt(e,r){e.style.display="block",e.style.transform=`translate(${r.x}px, ${r.y}px)`,e.style.width=`${r.width}px`,e.style.height=`${r.height}px`}function Rn(){let e=document.createElement("div");return e.setAttribute(k,""),e.className="fixed z-1 max-w-70 overflow-hidden rounded-md bg-primary px-2 py-1 font-sans text-xs font-semibold text-primary-foreground shadow-md",e.style.cssText=["position:fixed","pointer-events:none","white-space:nowrap","text-overflow:ellipsis",`z-index:${Bt}`].join(";"),e}function $t(e){if(!e.element.isConnected){e.outline.style.display="none",e.label.style.display="none";return}let r=e.element.getBoundingClientRect();xt(e.outline,vt(r)),e.label.textContent=Tn(e.element),e.label.style.display="block",e.label.style.transform=`translate(${Math.max(4,r.left)}px, ${Math.max(4,r.top-22)}px)`}var zn=500;function Pn(e){return{pageUrl:location.href,pageTitle:document.title?.trim()||null,tagName:e.tagName.toLowerCase(),pickedAt:new Date().toISOString(),selector:null,htmlPreview:e.outerHTML.slice(0,zn),componentName:null,source:null,stack:[],styles:""}}function wt(e,r){let i=document.createElement("button");return i.type="button",i.textContent=e,i.title=r,i.className="inline-flex h-7 cursor-pointer items-center justify-center rounded-md border border-transparent px-2 font-sans text-xs font-medium text-foreground outline-none hover:bg-accent disabled:pointer-events-none disabled:opacity-60",i}function It(e){e.setAttribute("aria-label",e.getAttribute("aria-label")??"Style value"),e.className="h-7 min-w-0 w-full appearance-none rounded-md border border-input bg-background px-2 font-mono text-xs text-foreground shadow-xs outline-none"}function Ft(e){let r=document.createElement("div");r.style.cssText="position:relative;min-width:0";let i=document.createElement("span");return i.textContent=e.dataset.unit??"",i.className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 font-mono text-xs text-muted-foreground",r.append(e,i),r}function B(e,r){let i=document.createElement("label");i.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let l=document.createElement("span");return l.textContent=e,It(r),i.append(l,r instanceof HTMLInputElement&&r.dataset.unit?Ft(r):r),i}function bt(){let e=document.createElement("section");return e.className="grid gap-1 border-t border-border py-2",e}function st(e,r="0"){let i=document.createElement("input");return i.type="number",i.placeholder=r,i.style.paddingRight="30px",i.dataset.unit=e,i}function Nn(e){if(e.length===0)return"";if(e.length===1)return`M ${e[0].x} ${e[0].y} l 0.01 0.01`;let r=`M ${e[0].x} ${e[0].y}`;for(let l=1;l<e.length-1;l+=1){let d=e[l],C=e[l+1];r+=` Q ${d.x} ${d.y} ${(d.x+C.x)/2} ${(d.y+C.y)/2}`}let i=e[e.length-1];return r+=` L ${i.x} ${i.y}`,r}function $n(e,r){let i=e.map((L)=>L.x),l=e.map((L)=>L.y),d=r+3,C=Math.min(...i)-d,N=Math.min(...l)-d,g=Math.max(...i)+d,H=Math.max(...l)+d;return{x:C,y:N,width:g-C,height:H-N}}function Hn(e){lt?.teardown();let r=!1,i=!1,l=(t)=>{if(i)return;i=!0,e(JSON.stringify(t))},d=document.createElement("div");d.setAttribute(k,""),d.style.cssText=`position:fixed;inset:0;z-index:${Sn};pointer-events:none`,An(d,Ht);let C=d.attachShadow({mode:"closed"}),N=document.createElement("style");N.textContent=nn,C.appendChild(N);let g=document.createElement("div");g.setAttribute(k,""),g.className="fixed inset-0 font-sans text-foreground",g.style.cssText="pointer-events:none";let H=document.createElement("style");H.setAttribute(k,""),H.textContent=`html[${gt}] body, html[${gt}] body * { cursor: crosshair !important; } [${k}], [${k}] * { cursor: default !important; } [${k}] input[type=number]::-webkit-inner-spin-button, [${k}] input[type=number]::-webkit-outer-spin-button { appearance:none; margin:0; }`,document.documentElement.appendChild(H),C.appendChild(g);let L=ft(at,Nt),v=ft(at,Nt);g.append(L,v);let z=document.createElementNS("http://www.w3.org/2000/svg","svg");z.setAttribute(k,""),z.setAttribute("width","100%"),z.setAttribute("height","100%"),z.setAttribute("viewBox",`0 0 ${window.innerWidth} ${window.innerHeight}`),z.style.cssText="position:fixed;inset:0;overflow:visible;pointer-events:none",z.style.zIndex=String(Bt),g.appendChild(z);let V=document.createElement("div");V.setAttribute(k,""),V.className="pointer-events-auto fixed top-2.5 left-1/2 flex -translate-x-1/2 gap-0.5 rounded-lg border border-border bg-popover/95 p-1 text-popover-foreground shadow-lg backdrop-blur-xl",V.style.zIndex=String(en),g.appendChild(V);let h=document.createElement("div");h.setAttribute(k,""),h.className="pointer-events-auto fixed hidden max-h-[calc(100vh-16px)] w-[min(360px,calc(100vw-16px))] flex-col overflow-hidden rounded-xl border border-border bg-popover/96 text-popover-foreground shadow-2xl backdrop-blur-xl",h.style.zIndex=String(en),g.appendChild(h);let j=document.createElement("div");j.className="flex items-start gap-2 p-2";let x=wt("","Expand annotation editor");x.setAttribute("aria-label","Expand annotation editor"),x.setAttribute("aria-expanded","false"),x.className+=" h-8 w-8 shrink-0 bg-muted p-0 text-muted-foreground hover:bg-accent hover:text-accent-foreground",x.innerHTML='<svg viewBox="0 0 20 20" width="15" height="15" aria-hidden="true"><path d="M4 5h12M4 10h12M4 15h12M7 3v4M13 8v4M9 13v4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>',j.appendChild(x);let y=document.createElement("textarea");y.placeholder="Describe the change…",y.rows=1,y.className="min-h-8 max-h-24 min-w-0 flex-1 resize-none overflow-y-hidden border-0 border-b border-b-transparent bg-transparent px-0 py-1.5 font-sans text-sm leading-5 text-foreground outline-none ring-0 placeholder:text-muted-foreground focus:border-b-primary focus:outline-none focus:ring-0",j.appendChild(y);let c=document.createElement("button");c.type="button",c.textContent="⠿",c.title="Drag annotation editor",c.className="hidden h-8 w-6 shrink-0 cursor-grab select-none border-0 bg-transparent p-0 font-sans text-lg font-bold leading-5 text-muted-foreground",j.appendChild(c);let D=wt("Attach","Attach annotation and screenshot (Enter)");D.className+=" h-8 shrink-0 border-primary bg-primary px-3 text-primary-foreground shadow-sm hover:bg-primary/90",j.appendChild(D),h.appendChild(j);let _=document.createElement("div");_.className="hidden max-h-[min(176px,calc(100vh-180px))] overflow-auto border-t border-border bg-muted/40 px-3",h.appendChild(_);let u=new Map,Y=[],U=[],tt=new Map,Dt=new Map,f="select",S=null,E=null,_t=!1,P=!1,Yt=!1,nt=null,I=null,W=null,Ut=()=>{y.style.height="auto";let n=Math.min(y.scrollHeight,96);y.style.height=`${n}px`,y.style.overflowY=y.scrollHeight>96?"auto":"hidden",pt()};y.addEventListener("input",Ut);let K=()=>{let t=u.size>0||Y.length>0||U.length>0;if(h.style.display=t?"flex":"none",D.disabled=!t,D.style.opacity=t?"1":"0.45",x.disabled=!t,_.style.display=P&&u.size>0?"grid":"none",pt(),t&&!Yt)Yt=!0,window.setTimeout(()=>y.focus({preventScroll:!0}),0)},yt=()=>{for(let[t,n]of Dt){let o=t===f;n.classList.toggle("bg-primary/10",o),n.classList.toggle("text-primary",o),n.classList.toggle("text-foreground",!o)}if(f!=="select")L.style.display="none";if(f!=="marquee")v.style.display="none";document.documentElement.setAttribute(gt,f)},Et=(t)=>{if(t.element instanceof HTMLElement||t.element instanceof SVGElement)for(let[n,o]of t.baselineStyles)if(o)t.element.style.setProperty(n,o);else t.element.style.removeProperty(n);u.delete(t.element),t.element.removeAttribute(Pt),t.outline.remove(),t.label.remove();for(let[n,o]of tt)if(o.targetId===t.id)tt.delete(n);K()},kt=(t)=>{if(u.has(t))return;let n={id:ht("element"),element:t,outline:ft(at,Nt),label:Rn(),baselineStyles:new Map};if(u.set(t,n),g.append(n.outline,n.label),$t(n),K(),P)_.style.display="grid",qt()},dn=(t,n)=>{let o=u.get(t);if(o){Et(o);return}if(!n)for(let a of Array.from(u.values()))Et(a);kt(t)},w=(t,n)=>{for(let o of u.values()){if(!(o.element instanceof HTMLElement||o.element instanceof SVGElement))continue;if(!o.baselineStyles.has(t))o.baselineStyles.set(t,o.element.style.getPropertyValue(t));let a=`${o.id}:${t}`,s=tt.get(a)?.previousValue??getComputedStyle(o.element).getPropertyValue(t).trim();o.element.style.setProperty(t,n,"important"),tt.set(a,{targetId:o.id,selector:null,property:t,previousValue:s,value:n}),$t(o)}},et=bt(),dt=bt(),ct=bt(),Mt=bt();_.append(et,dt,ct,Mt);let G=document.createElement("select");for(let t of["inherit","system-ui","sans-serif","serif","monospace"]){let n=document.createElement("option");n.value=t,n.textContent=t,G.appendChild(n)}G.addEventListener("change",()=>w("font-family",G.value)),et.appendChild(B("Font",G));let F=st("px","16");F.min="1",F.max="300",F.addEventListener("input",()=>{if(F.value)w("font-size",`${F.value}px`)}),et.appendChild(B("Font size",F));let ot=document.createElement("select");for(let t of["300","400","500","600","700","800","900"]){let n=document.createElement("option");n.value=t,n.textContent=t,ot.appendChild(n)}ot.addEventListener("change",()=>w("font-weight",ot.value)),et.appendChild(B("Font weight",ot));let X=document.createElement("input");X.type="text",X.placeholder="normal / 1.4",X.addEventListener("change",()=>{if(X.value.trim())w("line-height",X.value.trim())}),et.appendChild(B("Line height",X));let St=(t,n,o)=>{let a=document.createElement("label");a.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let s=document.createElement("span");s.textContent=t;let b=document.createElement("div");b.className="grid h-7 grid-cols-[22px_minmax(0,1fr)] items-center gap-1 rounded-md border border-input bg-background px-1 shadow-xs";let p=document.createElement("input");p.type="color",p.setAttribute("aria-label",t),p.style.cssText="width:20px;height:20px;padding:0;border:0;border-radius:5px;overflow:hidden;background:transparent;cursor:pointer";let M=document.createElement("input");return M.type="text",M.setAttribute("aria-label",`${t} value`),M.className="min-w-0 w-full border-0 bg-transparent font-mono text-xs text-foreground outline-none",p.addEventListener("input",()=>{M.value=p.value,w(n,p.value)}),M.addEventListener("change",()=>{let m=M.value.trim();if(!m)return;if(w(n,m),/^#[0-9a-f]{6}$/i.test(m))p.value=m}),b.append(p,M),a.append(s,b),o.appendChild(a),{row:a,color:p,text:M}},cn=St("Text color","color",dt),pn=St("Background","background-color",dt),T=document.createElement("input");T.type="range",T.min="0",T.max="1",T.step="0.05",T.value="1",T.style.accentColor=at,T.addEventListener("input",()=>w("opacity",T.value)),dt.appendChild(B("Opacity",T));let q=st("px","0");q.min="0",q.max="300",q.addEventListener("input",()=>{if(q.value)w("border-radius",`${q.value}px`)}),ct.appendChild(B("Radius",q));let un=St("Border color","border-color",ct),O=st("px","0");O.min="0",O.max="100",O.addEventListener("input",()=>{if(O.value)w("border-style","solid"),w("border-width",`${O.value}px`)}),ct.appendChild(B("Border width",O));let Lt=document.createElement("div");Lt.style.cssText="display:grid;grid-template-columns:82px minmax(0,1fr);gap:8px;align-items:center";let At=document.createElement("div");At.className="grid gap-2 font-sans text-xs font-medium text-muted-foreground",At.innerHTML="<span>Width</span><span>Height</span>";let Ct=document.createElement("div");Ct.style.cssText="position:relative;display:grid;gap:3px;padding-left:22px";let Z=st("px","auto"),J=st("px","auto");It(Z),It(J);let A=wt("","Lock aspect ratio");A.setAttribute("aria-pressed","true"),A.style.cssText+=";position:absolute;left:0;top:50%;transform:translateY(-50%);width:18px;height:38px;padding:0",A.className+=" bg-primary/10 text-primary",Ct.append(Ft(Z),Ft(J),A),Lt.append(At,Ct),Mt.appendChild(Lt);let R=!0,rt=1,Xt=()=>{A.innerHTML=R?'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="M8 6.5 9.5 5A3.5 3.5 0 0 1 14.5 10l-1.5 1.5M12 13.5 10.5 15A3.5 3.5 0 0 1 5.5 10L7 8.5M7.5 12.5l5-5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>':'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="m6 6 8 8M8 6.5 9.5 5A3.5 3.5 0 0 1 14 9M12 13.5 10.5 15A3.5 3.5 0 0 1 6 11" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',A.setAttribute("aria-pressed",String(R)),A.classList.toggle("bg-primary/10",R),A.classList.toggle("text-primary",R),A.classList.toggle("bg-muted",!R),A.classList.toggle("text-muted-foreground",!R)};A.addEventListener("click",()=>{R=!R,Xt()}),Z.addEventListener("input",()=>{let t=Number(Z.value);if(!Number.isFinite(t)||t<=0)return;if(w("width",`${t}px`),R&&rt>0){let n=Math.max(1,Math.round(t/rt));J.value=String(n),w("height",`${n}px`)}}),J.addEventListener("input",()=>{let t=Number(J.value);if(!Number.isFinite(t)||t<=0)return;if(w("height",`${t}px`),R&&rt>0){let n=Math.max(1,Math.round(t*rt));Z.value=String(n),w("width",`${n}px`)}}),Xt();let Tt=(t,n,o)=>{let a=document.createElement("input");return a.type="text",a.placeholder=o,a.addEventListener("change",()=>{if(a.value.trim())w(n,a.value.trim())}),Mt.appendChild(B(t,a)),a},mn=Tt("Padding","padding","0 0 0 0"),gn=Tt("Margin","margin","0 0 0 0"),hn=Tt("Gap","gap","0px"),qt=()=>{let t=u.values().next().value;if(!t)return;let n=getComputedStyle(t.element),o=t.element.getBoundingClientRect();rt=o.height>0?o.width/o.height:1,Z.value=String(Math.round(o.width)),J.value=String(Math.round(o.height)),F.value=String(Math.round(Number.parseFloat(n.fontSize)||16)),ot.value=n.fontWeight.match(/^[0-9]+$/)?n.fontWeight:"400",X.value=n.lineHeight,G.value=Array.from(G.options).some((a)=>a.value===n.fontFamily)?n.fontFamily:"inherit",cn.text.value=n.color,pn.text.value=n.backgroundColor,un.text.value=n.borderColor,T.value=n.opacity,q.value=String(Math.round(Number.parseFloat(n.borderRadius)||0)),O.value=String(Math.round(Number.parseFloat(n.borderWidth)||0)),mn.value=n.padding,gn.value=n.margin,hn.value=n.gap==="normal"?"0px":n.gap},fn=[["select","Select","Select elements (V)"],["marquee","Region","Draw a region or marquee-select elements (R)"],["draw","Draw","Draw freehand (D)"],["erase","Erase","Remove an annotation target (E)"]];for(let[t,n,o]of fn){let a=wt(n,o);a.className+=" h-8 px-2.5 text-sm",a.addEventListener("click",()=>{f=t,yt()}),Dt.set(t,a),V.appendChild(a)}let wn=(t,n)=>{let a=h.getBoundingClientRect();return{left:Math.min(Math.max(8,t),Math.max(8,window.innerWidth-a.width-8)),top:Math.min(Math.max(8,n),Math.max(8,window.innerHeight-a.height-8))}},Rt=(t)=>{let n=wn(t.left,t.top);if(h.style.left=`${n.left}px`,h.style.top=`${n.top}px`,h.style.right="auto",h.style.bottom="auto",P)nt=n},bn=()=>an([...Array.from(u.values(),(t)=>vt(t.element.getBoundingClientRect())),...Y.map((t)=>t.rect),...U.map((t)=>t.bounds)],0),vn=()=>{let t=bn();if(!t)return;let n=h.getBoundingClientRect(),o=8,a=[{left:t.x+t.width+o,top:t.y},{left:t.x-n.width-o,top:t.y},{left:t.x+t.width-n.width,top:t.y+t.height+o},{left:t.x+t.width-n.width,top:t.y-n.height-o}],s=(p)=>Math.max(0,-p.left)+Math.max(0,-p.top)+Math.max(0,p.left+n.width-window.innerWidth)+Math.max(0,p.top+n.height-window.innerHeight),b=a.reduce((p,M)=>s(M)<s(p)?M:p);Rt(b)};function pt(){if(W!==null)window.cancelAnimationFrame(W);W=window.requestAnimationFrame(()=>{if(W=null,h.style.display==="none")return;if(P&&nt)Rt(nt);else vn()})}x.addEventListener("click",()=>{if(u.size===0)return;if(!P){let t=h.getBoundingClientRect();if(P=!0,nt={left:t.left,top:t.top},_.style.display=u.size>0?"grid":"none",c.style.display="block",x.setAttribute("aria-expanded","true"),x.title="Collapse annotation editor",x.setAttribute("aria-label","Collapse annotation editor"),u.size>0)qt()}else P=!1,nt=null,_.style.display="none",c.style.display="none",x.setAttribute("aria-expanded","false"),x.title="Expand annotation editor",x.setAttribute("aria-label","Expand annotation editor");pt()});let Ot=(t)=>{if(t.button!==0||!P)return;let n=h.getBoundingClientRect();I={pointerId:t.pointerId,offsetX:t.clientX-n.left,offsetY:t.clientY-n.top},c.setPointerCapture(t.pointerId),c.style.cursor="grabbing",t.preventDefault(),t.stopPropagation()},Vt=(t)=>{if(!I||I.pointerId!==t.pointerId)return;Rt({left:t.clientX-I.offsetX,top:t.clientY-I.offsetY}),t.preventDefault(),t.stopPropagation()},ut=(t)=>{if(!I||I.pointerId!==t.pointerId)return;if(I=null,c.style.cursor="grab",c.hasPointerCapture(t.pointerId))c.releasePointerCapture(t.pointerId);t.preventDefault(),t.stopPropagation()};c.addEventListener("pointerdown",Ot),c.addEventListener("pointermove",Vt),c.addEventListener("pointerup",ut),c.addEventListener("pointercancel",ut);let mt=()=>{for(let t of u.values())$t(t);pt()},xn=(t,n)=>{for(let s of Array.from(u.values()).toReversed()){let b=s.element.getBoundingClientRect();if(t>=b.left&&t<=b.right&&n>=b.top&&n<=b.bottom)return Et(s),!0}let o=Y.findIndex((s)=>t>=s.rect.x&&t<=s.rect.x+s.rect.width&&n>=s.rect.y&&n<=s.rect.y+s.rect.height);if(o>=0){let[s]=Y.splice(o,1);return g.querySelector(`[data-region-id="${s?.id}"]`)?.remove(),K(),!0}let a=U.findIndex((s)=>t>=s.bounds.x&&t<=s.bounds.x+s.bounds.width&&n>=s.bounds.y&&n<=s.bounds.y+s.bounds.height);if(a>=0){let[s]=U.splice(a,1);return z.querySelector(`[data-stroke-id="${s?.id}"]`)?.remove(),K(),!0}return!1},yn=(t)=>{let n=Array.from(document.querySelectorAll("body *")).filter((o)=>!Q(o)).map((o)=>({element:o,rect:o.getBoundingClientRect()})).filter(({rect:o})=>{if(o.width<2||o.height<2)return!1;return!(o.right<t.x||o.left>t.x+t.width||o.bottom<t.y||o.top>t.y+t.height)}).filter(({element:o,rect:a})=>{let s=a.left+a.width/2,b=a.top+a.height/2;return s>=t.x&&s<=t.x+t.width&&b>=t.y&&b<=t.y+t.height&&(o.children.length===0||o instanceof HTMLButtonElement||o instanceof HTMLAnchorElement||o.getAttribute("role")==="button")}).sort((o,a)=>o.rect.width*o.rect.height-a.rect.width*a.rect.height).slice(0,Ln);for(let o of n)kt(o.element);return n.length},it=()=>{L.style.display="none"},jt=(t)=>{if(Q(t.target)){it();return}if(f==="select"&&S===null){let n=sn(t.clientX,t.clientY);if(n)xt(L,vt(n.getBoundingClientRect()));else it();return}if(it(),f==="marquee"&&S){xt(v,rn(S.x,S.y,t.clientX,t.clientY));return}if(f==="draw"&&E)E.target.points=[...E.target.points,{x:t.clientX,y:t.clientY}],E.target.bounds=$n(E.target.points,E.target.width),E.path.setAttribute("d",Nn(E.target.points))},Wt=(t)=>{if(!t.isTrusted||t.button!==0||Q(t.target))return;if(t.preventDefault(),t.stopPropagation(),f==="select"){let n=sn(t.clientX,t.clientY);if(n)dn(n,t.shiftKey);return}if(f==="erase"){xn(t.clientX,t.clientY);return}if(S={x:t.clientX,y:t.clientY},f==="draw"){let n={id:ht("stroke"),color:Ht?.primary??"#2563eb",width:4,points:[S],bounds:{x:S.x,y:S.y,width:1,height:1}},o=document.createElementNS("http://www.w3.org/2000/svg","path");o.setAttribute(k,""),o.setAttribute("data-stroke-id",n.id),o.setAttribute("fill","none"),o.setAttribute("stroke",n.color),o.setAttribute("stroke-width",String(n.width)),o.setAttribute("stroke-linecap","round"),o.setAttribute("stroke-linejoin","round"),z.appendChild(o),E={target:n,path:o}}},Kt=(t)=>{if(!S)return;if(t.preventDefault(),t.stopPropagation(),f==="marquee"){let n=rn(S.x,S.y,t.clientX,t.clientY);if(v.style.display="none",Cn(n)){if(yn(n)===0){let a={id:ht("region"),rect:n};Y.push(a);let s=ft(at,"color-mix(in srgb, var(--t3-primary) 6%, transparent)");s.setAttribute("data-region-id",a.id),xt(s,n),g.appendChild(s)}}}else if(f==="draw"&&E){if(E.target.points.length>1)U.push(E.target);else E.path.remove();E=null}S=null,K()},Gt=(t)=>{if(Q(t.target))return;t.preventDefault(),t.stopPropagation()},Zt=(t)=>{if(t.relatedTarget===null)it()},Jt=()=>{it()},En=()=>{for(let t of u.values()){if(t.element.removeAttribute(Pt),!(t.element instanceof HTMLElement||t.element instanceof SVGElement))continue;for(let[n,o]of t.baselineStyles)if(o)t.element.style.setProperty(n,o);else t.element.style.removeProperty(n)}},Qt=()=>{if(r)return;if(r=!0,En(),window.removeEventListener("pointermove",jt,!0),window.removeEventListener("pointerdown",Wt,!0),window.removeEventListener("pointerup",Kt,!0),window.removeEventListener("pointerout",Zt,!0),window.removeEventListener("click",Gt,!0),window.removeEventListener("blur",Jt),window.removeEventListener("keydown",tn,!0),window.removeEventListener("scroll",mt,!0),window.removeEventListener("resize",mt),c.removeEventListener("pointerdown",Ot),c.removeEventListener("pointermove",Vt),c.removeEventListener("pointerup",ut),c.removeEventListener("pointercancel",ut),W!==null)window.cancelAnimationFrame(W);document.documentElement.removeAttribute(gt),H.remove(),d.remove(),lt=null,l(null)},tn=(t)=>{if(!t.isTrusted)return;if(Q(t.target)&&t.key!=="Escape")return;if(t.key==="Escape"){t.preventDefault(),t.stopPropagation(),Qt();return}if(t.key==="v")f="select";else if(t.key==="r")f="marquee";else if(t.key==="d")f="draw";else if(t.key==="e")f="erase";else return;yt()},zt=(t)=>{if(_t||u.size===0&&Y.length===0&&U.length===0)return;_t=!0,D.disabled=!0,D.textContent="Capturing…";let n=y.value.trim(),o=[...Y],a=[...U],s=Array.from(tt.values(),(m)=>({...m})),b=Array.from(u.values(),(m)=>{let kn=Pn(m.element);return m.element.setAttribute(Pt,m.id),{id:m.id,element:kn,rect:vt(m.element.getBoundingClientRect())}}),p={id:ht("annotation"),pageUrl:location.href,pageTitle:document.title?.trim()||null,comment:n,elements:b,regions:o,strokes:a,styleChanges:s,screenshot:null,createdAt:new Date().toISOString()};h.style.display="none",V.style.display="none",L.style.display="none";let M=an([...b.map((m)=>m.rect),...o.map((m)=>m.rect),...a.map((m)=>m.bounds)]);window.requestAnimationFrame(()=>l({annotation:p,cropRect:M,submission:t}))};D.addEventListener("click",(t)=>{if(t.isTrusted)zt("attach")}),g.addEventListener("keydown",(t)=>{let n=t.isTrusted&&t.target===y?Mn(t):null;if(t.stopImmediatePropagation(),!n)return;t.preventDefault(),zt(n)}),window.addEventListener("pointermove",jt,{capture:!0,passive:!1}),window.addEventListener("pointerdown",Wt,{capture:!0,passive:!1}),window.addEventListener("pointerup",Kt,{capture:!0,passive:!1}),window.addEventListener("pointerout",Zt,{capture:!0,passive:!0}),window.addEventListener("click",Gt,{capture:!0,passive:!1}),window.addEventListener("blur",Jt),window.addEventListener("keydown",tn,{capture:!0}),window.addEventListener("scroll",mt,{capture:!0,passive:!0}),window.addEventListener("resize",mt,{passive:!0}),document.documentElement.appendChild(d),yt(),K(),lt={teardown:Qt,demo:{select:kt,comment:(t)=>{y.value=t,Ut()},fontSize:(t)=>{if(!P)x.click();F.value=String(t),w("font-size",`${t}px`)},submit:zt}}}var ln=globalThis;ln.__zeronAnnotate?.teardown();ln.__zeronAnnotate={start(e){return Ht=e,new Promise((r)=>Hn(r))},teardown(){lt?.teardown()},demo(){return lt?.demo}};})();
