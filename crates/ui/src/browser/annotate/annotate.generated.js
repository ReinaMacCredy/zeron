(()=>{var tn=`/*! tailwindcss v4.3.3 | MIT License | https://tailwindcss.com */
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
`;var kn=(n,r)=>typeof r==="string"?r.toWellFormed():r,Mn=(n)=>n.key==="Enter"&&!n.shiftKey&&!n.isComposing,E="data-zeron-annotation-ui",$t="data-zeron-annotate",gt="data-zeron-annotation-tool",An=2147483646,at="var(--t3-primary)",It="color-mix(in srgb, var(--t3-primary) 10%, transparent)",Ln=20,_t=1,nn=10,dt=null,en=0,Ft=null,Sn=(n,r)=>{if(!r)return;n.style.colorScheme=r.colorScheme;let i={"--t3-radius":r.radius,"--t3-background":r.background,"--t3-foreground":r.foreground,"--t3-popover":r.popover,"--t3-popover-foreground":r.popoverForeground,"--t3-primary":r.primary,"--t3-primary-foreground":r.primaryForeground,"--t3-muted":r.muted,"--t3-muted-foreground":r.mutedForeground,"--t3-accent":r.accent,"--t3-accent-foreground":r.accentForeground,"--t3-border":r.border,"--t3-input":r.input,"--t3-ring":r.ring,"--t3-font-sans":r.fontSans,"--t3-font-mono":r.fontMono};for(let[d,c]of Object.entries(i))n.style.setProperty(d,c)},ht=(n)=>(en+=1,`${n}_${en.toString(36)}`),lt=(n)=>({x:n.left,y:n.top,width:n.width,height:n.height}),on=(n,r,i,d)=>({x:Math.min(n,i),y:Math.min(r,d),width:Math.abs(i-n),height:Math.abs(d-r)}),rn=(n)=>n.width>=3&&n.height>=3;function an(n,r=20){if(n.length===0)return null;let i=Math.min(...n.map((v)=>v.x)),d=Math.min(...n.map((v)=>v.y)),c=Math.max(...n.map((v)=>v.x+v.width)),L=Math.max(...n.map((v)=>v.y+v.height)),z=Math.max(0,i-r),g=Math.max(0,d-r),N=Math.max(1,window.innerWidth-z),M=Math.max(1,window.innerHeight-g);return{x:z,y:g,width:Math.min(N,c-i+r*2),height:Math.min(M,L-d+r*2)}}function Q(n){return n instanceof Element&&n.closest(`[${E}]`)!==null}function sn(n,r){for(let i of document.elementsFromPoint(n,r)){if(!(i instanceof Element))continue;if(Q(i))continue;if(i===document.documentElement||i===document.body)continue;return i}return null}function Cn(n){let r=n.tagName.toLowerCase(),i=n.id?`#${n.id}`:"",d=n instanceof HTMLElement&&typeof n.className==="string"?n.className.trim().split(/\s+/).filter(Boolean).slice(0,2).map((c)=>`.${c}`).join(""):"";return`${r}${i}${d}`}function ft(n,r){let i=document.createElement("div");return i.setAttribute(E,""),i.style.cssText=["position:fixed","pointer-events:none",`border:2px solid ${n}`,`background:${r}`,"border-radius:3px","box-sizing:border-box","display:none",`z-index:${_t}`].join(";"),i}function vt(n,r){n.style.display="block",n.style.transform=`translate(${r.x}px, ${r.y}px)`,n.style.width=`${r.width}px`,n.style.height=`${r.height}px`}function Tn(){let n=document.createElement("div");return n.setAttribute(E,""),n.className="fixed z-1 max-w-70 overflow-hidden rounded-md bg-primary px-2 py-1 font-sans text-xs font-semibold text-primary-foreground shadow-md",n.style.cssText=["position:fixed","pointer-events:none","white-space:nowrap","text-overflow:ellipsis",`z-index:${_t}`].join(";"),n}function Ht(n){if(!n.element.isConnected){n.outline.style.display="none",n.label.style.display="none";return}let r=n.element.getBoundingClientRect();vt(n.outline,lt(r)),n.label.textContent=Cn(n.element),n.label.style.display="block";let i=Math.max(4,Math.min(r.left,window.innerWidth-n.label.offsetWidth-4));n.label.style.transform=`translate(${i}px, ${Math.max(4,r.top-22)}px)`}var Rn=500;function zn(n){return{pageUrl:location.href,pageTitle:document.title?.trim()||null,tagName:n.tagName.toLowerCase(),pickedAt:new Date().toISOString(),selector:null,htmlPreview:n.outerHTML.slice(0,Rn),componentName:null,source:null,stack:[],styles:""}}function wt(n,r){let i=document.createElement("button");return i.type="button",i.textContent=n,i.title=r,i.className="inline-flex h-7 cursor-pointer items-center justify-center rounded-md border border-transparent px-2 font-sans text-xs font-medium text-foreground outline-none hover:bg-accent disabled:pointer-events-none disabled:opacity-60",i}function Bt(n){n.setAttribute("aria-label",n.getAttribute("aria-label")??"Style value"),n.className="h-7 min-w-0 w-full appearance-none rounded-md border border-input bg-background px-2 font-mono text-xs text-foreground shadow-xs outline-none"}function Dt(n){let r=document.createElement("div");r.style.cssText="position:relative;min-width:0";let i=document.createElement("span");return i.textContent=n.dataset.unit??"",i.className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 font-mono text-xs text-muted-foreground",r.append(n,i),r}function H(n,r){let i=document.createElement("label");i.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let d=document.createElement("span");return d.textContent=n,Bt(r),i.append(d,r instanceof HTMLInputElement&&r.dataset.unit?Dt(r):r),i}function bt(){let n=document.createElement("section");return n.className="grid gap-1 border-t border-border py-2",n}function st(n,r="0"){let i=document.createElement("input");return i.type="number",i.placeholder=r,i.style.paddingRight="30px",i.dataset.unit=n,i}function Nn(n){if(n.length===0)return"";if(n.length===1)return`M ${n[0].x} ${n[0].y} l 0.01 0.01`;let r=`M ${n[0].x} ${n[0].y}`;for(let d=1;d<n.length-1;d+=1){let c=n[d],L=n[d+1];r+=` Q ${c.x} ${c.y} ${(c.x+L.x)/2} ${(c.y+L.y)/2}`}let i=n[n.length-1];return r+=` L ${i.x} ${i.y}`,r}function Pn(n,r){let i=n.map((M)=>M.x),d=n.map((M)=>M.y),c=r+3,L=Math.min(...i)-c,z=Math.min(...d)-c,g=Math.max(...i)+c,N=Math.max(...d)+c;return{x:L,y:z,width:g-L,height:N-z}}function $n(n){dt?.teardown();let r=!1,i=!1,d=(t)=>{if(i)return;i=!0,n(JSON.stringify(t,kn))},c=document.createElement("div");c.setAttribute(E,""),c.style.cssText=`position:fixed;inset:0;z-index:${An};pointer-events:none`,Sn(c,Ft);let L=c.attachShadow({mode:"closed"}),z=document.createElement("style");z.textContent=tn,L.appendChild(z);let g=document.createElement("div");g.setAttribute(E,""),g.className="fixed inset-0 font-sans text-foreground",g.style.cssText="pointer-events:none";let N=document.createElement("style");N.setAttribute(E,""),N.textContent=`html[${gt}] body, html[${gt}] body * { cursor: crosshair !important; } [${E}], [${E}] * { cursor: default !important; } [${E}] input[type=number]::-webkit-inner-spin-button, [${E}] input[type=number]::-webkit-outer-spin-button { appearance:none; margin:0; }`,document.documentElement.appendChild(N),L.appendChild(g);let M=ft(at,It),v=ft(at,It);g.append(M,v);let T=document.createElementNS("http://www.w3.org/2000/svg","svg");T.setAttribute(E,""),T.setAttribute("width","100%"),T.setAttribute("height","100%"),T.setAttribute("viewBox",`0 0 ${window.innerWidth} ${window.innerHeight}`),T.style.cssText="position:fixed;inset:0;overflow:visible;pointer-events:none",T.style.zIndex=String(_t),g.appendChild(T);let X=document.createElement("div");X.setAttribute(E,""),X.className="pointer-events-auto fixed top-2.5 left-1/2 flex -translate-x-1/2 gap-0.5 rounded-lg border border-border bg-popover/95 p-1 text-popover-foreground shadow-lg backdrop-blur-xl",X.style.zIndex=String(nn),g.appendChild(X);let h=document.createElement("div");h.setAttribute(E,""),h.className="pointer-events-auto fixed hidden max-h-[calc(100vh-16px)] w-[min(360px,calc(100vw-16px))] flex-col overflow-hidden rounded-xl border border-border bg-popover/96 text-popover-foreground shadow-2xl backdrop-blur-xl",h.style.zIndex=String(nn),g.appendChild(h);let q=document.createElement("div");q.className="flex items-start gap-2 p-2";let x=wt("","Expand annotation editor");x.setAttribute("aria-label","Expand annotation editor"),x.setAttribute("aria-expanded","false"),x.className+=" h-8 w-8 shrink-0 bg-muted p-0 text-muted-foreground hover:bg-accent hover:text-accent-foreground",x.innerHTML='<svg viewBox="0 0 20 20" width="15" height="15" aria-hidden="true"><path d="M4 5h12M4 10h12M4 15h12M7 3v4M13 8v4M9 13v4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>',q.appendChild(x);let xt=document.createElement("span");xt.textContent="Attach, then describe the change in the chat",xt.className="min-w-0 flex-1 self-center font-sans text-xs text-muted-foreground",q.appendChild(xt);let p=document.createElement("button");p.type="button",p.textContent="⠿",p.title="Drag annotation editor",p.className="hidden h-8 w-6 shrink-0 cursor-grab select-none border-0 bg-transparent p-0 font-sans text-lg font-bold leading-5 text-muted-foreground",q.appendChild(p);let F=wt("Attach","Attach annotation and screenshot (Enter)");F.className+=" h-8 shrink-0 border-primary bg-primary px-3 text-primary-foreground shadow-sm hover:bg-primary/90",q.appendChild(F),h.appendChild(q);let B=document.createElement("div");B.className="hidden max-h-[min(176px,calc(100vh-180px))] overflow-auto border-t border-border bg-muted/40 px-3",h.appendChild(B);let u=new Map,D=[],_=[],tt=new Map,Yt=new Map,f="select",k=null,y=null,yt=!1,R=!1,nt=null,P=null,O=null,j=()=>{let t=u.size>0||D.length>0||_.length>0;h.style.display=t?"flex":"none",F.disabled=!t,F.style.opacity=t?"1":"0.45",x.disabled=!t,B.style.display=R&&u.size>0?"grid":"none",Nt()},Et=()=>{for(let[t,e]of Yt){let o=t===f;e.classList.toggle("bg-primary/10",o),e.classList.toggle("text-primary",o),e.classList.toggle("text-foreground",!o)}if(f!=="select")M.style.display="none";if(f!=="marquee")v.style.display="none";document.documentElement.setAttribute(gt,f)},Ut=(t)=>{if(t.baselineStyle===void 0)return;if(t.baselineStyle===null)t.element.removeAttribute("style");else t.element.setAttribute("style",t.baselineStyle)},kt=(t)=>{Ut(t),u.delete(t.element),t.element.removeAttribute($t),t.outline.remove(),t.label.remove();for(let[e,o]of tt)if(o.targetId===t.id)tt.delete(e);j()},Mt=(t)=>{if(u.has(t))return;let e={id:ht("element"),element:t,outline:ft(at,It),label:Tn()};if(u.set(t,e),g.append(e.outline,e.label),Ht(e),j(),R)B.style.display="grid",Xt()},dn=(t,e)=>{let o=u.get(t);if(o){kt(o);return}if(!e)for(let a of Array.from(u.values()))kt(a);Mt(t)},w=(t,e)=>{for(let o of u.values()){if(!(o.element instanceof HTMLElement||o.element instanceof SVGElement))continue;if(o.baselineStyle===void 0)o.baselineStyle=o.element.getAttribute("style");let a=`${o.id}:${t}`,s=tt.get(a)?.previousValue??getComputedStyle(o.element).getPropertyValue(t).trim();o.element.style.setProperty(t,e,"important"),tt.set(a,{targetId:o.id,selector:null,property:t,previousValue:s,value:e}),Ht(o)}},et=bt(),ct=bt(),pt=bt(),At=bt();B.append(et,ct,pt,At);let V=document.createElement("select");for(let t of["inherit","system-ui","sans-serif","serif","monospace"]){let e=document.createElement("option");e.value=t,e.textContent=t,V.appendChild(e)}V.addEventListener("change",()=>w("font-family",V.value)),et.appendChild(H("Font",V));let I=st("px","16");I.min="1",I.max="300",I.addEventListener("input",()=>{if(I.value)w("font-size",`${I.value}px`)}),et.appendChild(H("Font size",I));let ot=document.createElement("select");for(let t of["300","400","500","600","700","800","900"]){let e=document.createElement("option");e.value=t,e.textContent=t,ot.appendChild(e)}ot.addEventListener("change",()=>w("font-weight",ot.value)),et.appendChild(H("Font weight",ot));let Y=document.createElement("input");Y.type="text",Y.placeholder="normal / 1.4",Y.addEventListener("change",()=>{if(Y.value.trim())w("line-height",Y.value.trim())}),et.appendChild(H("Line height",Y));let Lt=(t,e,o)=>{let a=document.createElement("label");a.className="grid min-h-7 grid-cols-[82px_minmax(0,1fr)] items-center gap-2 font-sans text-xs font-medium text-muted-foreground";let s=document.createElement("span");s.textContent=t;let b=document.createElement("div");b.className="grid h-7 grid-cols-[22px_minmax(0,1fr)] items-center gap-1 rounded-md border border-input bg-background px-1 shadow-xs";let m=document.createElement("input");m.type="color",m.setAttribute("aria-label",t),m.style.cssText="width:20px;height:20px;padding:0;border:0;border-radius:5px;overflow:hidden;background:transparent;cursor:pointer";let l=document.createElement("input");return l.type="text",l.setAttribute("aria-label",`${t} value`),l.className="min-w-0 w-full border-0 bg-transparent font-mono text-xs text-foreground outline-none",m.addEventListener("input",()=>{l.value=m.value,w(e,m.value)}),l.addEventListener("change",()=>{let J=l.value.trim();if(!J)return;if(w(e,J),/^#[0-9a-f]{6}$/i.test(J))m.value=J}),b.append(m,l),a.append(s,b),o.appendChild(a),{row:a,color:m,text:l}},cn=Lt("Text color","color",ct),pn=Lt("Background","background-color",ct),S=document.createElement("input");S.type="range",S.min="0",S.max="1",S.step="0.05",S.value="1",S.style.accentColor=at,S.addEventListener("input",()=>w("opacity",S.value)),ct.appendChild(H("Opacity",S));let U=st("px","0");U.min="0",U.max="300",U.addEventListener("input",()=>{if(U.value)w("border-radius",`${U.value}px`)}),pt.appendChild(H("Radius",U));let un=Lt("Border color","border-color",pt),W=st("px","0");W.min="0",W.max="100",W.addEventListener("input",()=>{if(W.value)w("border-style","solid"),w("border-width",`${W.value}px`)}),pt.appendChild(H("Border width",W));let St=document.createElement("div");St.style.cssText="display:grid;grid-template-columns:82px minmax(0,1fr);gap:8px;align-items:center";let Ct=document.createElement("div");Ct.className="grid gap-2 font-sans text-xs font-medium text-muted-foreground",Ct.innerHTML="<span>Width</span><span>Height</span>";let Tt=document.createElement("div");Tt.style.cssText="position:relative;display:grid;gap:3px;padding-left:22px";let K=st("px","auto"),G=st("px","auto");Bt(K),Bt(G);let A=wt("","Lock aspect ratio");A.setAttribute("aria-pressed","true"),A.style.cssText+=";position:absolute;left:0;top:50%;transform:translateY(-50%);width:18px;height:38px;padding:0",A.className+=" bg-primary/10 text-primary",Tt.append(Dt(K),Dt(G),A),St.append(Ct,Tt),At.appendChild(St);let C=!0,rt=1,Wt=()=>{A.innerHTML=C?'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="M8 6.5 9.5 5A3.5 3.5 0 0 1 14.5 10l-1.5 1.5M12 13.5 10.5 15A3.5 3.5 0 0 1 5.5 10L7 8.5M7.5 12.5l5-5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>':'<svg viewBox="0 0 20 20" width="14" height="14" aria-hidden="true"><path d="m6 6 8 8M8 6.5 9.5 5A3.5 3.5 0 0 1 14 9M12 13.5 10.5 15A3.5 3.5 0 0 1 6 11" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',A.setAttribute("aria-pressed",String(C)),A.classList.toggle("bg-primary/10",C),A.classList.toggle("text-primary",C),A.classList.toggle("bg-muted",!C),A.classList.toggle("text-muted-foreground",!C)};A.addEventListener("click",()=>{C=!C,Wt()}),K.addEventListener("input",()=>{let t=Number(K.value);if(!Number.isFinite(t)||t<=0)return;if(w("width",`${t}px`),C&&rt>0){let e=Math.max(1,Math.round(t/rt));G.value=String(e),w("height",`${e}px`)}}),G.addEventListener("input",()=>{let t=Number(G.value);if(!Number.isFinite(t)||t<=0)return;if(w("height",`${t}px`),C&&rt>0){let e=Math.max(1,Math.round(t*rt));K.value=String(e),w("width",`${e}px`)}}),Wt();let Rt=(t,e,o)=>{let a=document.createElement("input");return a.type="text",a.placeholder=o,a.addEventListener("change",()=>{if(a.value.trim())w(e,a.value.trim())}),At.appendChild(H(t,a)),a},mn=Rt("Padding","padding","0 0 0 0"),gn=Rt("Margin","margin","0 0 0 0"),hn=Rt("Gap","gap","0px"),Xt=()=>{let t=u.values().next().value;if(!t)return;let e=getComputedStyle(t.element),o=t.element.getBoundingClientRect();rt=o.height>0?o.width/o.height:1,K.value=String(Math.round(o.width)),G.value=String(Math.round(o.height)),I.value=String(Math.round(Number.parseFloat(e.fontSize)||16)),ot.value=e.fontWeight.match(/^[0-9]+$/)?e.fontWeight:"400",Y.value=e.lineHeight,V.value=Array.from(V.options).some((a)=>a.value===e.fontFamily)?e.fontFamily:"inherit",cn.text.value=e.color,pn.text.value=e.backgroundColor,un.text.value=e.borderColor,S.value=e.opacity,U.value=String(Math.round(Number.parseFloat(e.borderRadius)||0)),W.value=String(Math.round(Number.parseFloat(e.borderWidth)||0)),mn.value=e.padding,gn.value=e.margin,hn.value=e.gap==="normal"?"0px":e.gap},fn=[["select","Select","Select elements (V)"],["marquee","Region","Draw a region or marquee-select elements (R)"],["draw","Draw","Draw freehand (D)"],["erase","Erase","Remove an annotation target (E)"]];for(let[t,e,o]of fn){let a=wt(e,o);a.className+=" h-8 px-2.5 text-sm",a.addEventListener("click",()=>{f=t,Et()}),Yt.set(t,a),X.appendChild(a)}let wn=(t,e)=>{let a=h.getBoundingClientRect();return{left:Math.min(Math.max(8,t),Math.max(8,window.innerWidth-a.width-8)),top:Math.min(Math.max(8,e),Math.max(8,window.innerHeight-a.height-8))}},zt=(t)=>{let e=wn(t.left,t.top);if(h.style.left=`${e.left}px`,h.style.top=`${e.top}px`,h.style.right="auto",h.style.bottom="auto",R)nt=e},bn=()=>an([...Array.from(u.values(),(t)=>lt(t.element.getBoundingClientRect())),...D.map((t)=>t.rect),..._.map((t)=>t.bounds)],0),vn=()=>{let t=bn();if(!t)return;let e=h.getBoundingClientRect(),o=8,a=[{left:t.x+t.width+o,top:t.y},{left:t.x-e.width-o,top:t.y},{left:t.x+t.width-e.width,top:t.y+t.height+o},{left:t.x+t.width-e.width,top:t.y-e.height-o}],s=(m)=>Math.max(0,-m.left)+Math.max(0,-m.top)+Math.max(0,m.left+e.width-window.innerWidth)+Math.max(0,m.top+e.height-window.innerHeight),b=a.reduce((m,l)=>s(l)<s(m)?l:m);zt(b)};function Nt(){if(O!==null)window.cancelAnimationFrame(O);O=window.requestAnimationFrame(()=>{if(O=null,h.style.display==="none")return;if(R&&nt)zt(nt);else vn()})}x.addEventListener("click",()=>{if(u.size===0)return;if(!R){let t=h.getBoundingClientRect();if(R=!0,nt={left:t.left,top:t.top},B.style.display=u.size>0?"grid":"none",p.style.display="block",x.setAttribute("aria-expanded","true"),x.title="Collapse annotation editor",x.setAttribute("aria-label","Collapse annotation editor"),u.size>0)Xt()}else R=!1,nt=null,B.style.display="none",p.style.display="none",x.setAttribute("aria-expanded","false"),x.title="Expand annotation editor",x.setAttribute("aria-label","Expand annotation editor");Nt()});let qt=(t)=>{if(t.button!==0||!R)return;let e=h.getBoundingClientRect();P={pointerId:t.pointerId,offsetX:t.clientX-e.left,offsetY:t.clientY-e.top},p.setPointerCapture(t.pointerId),p.style.cursor="grabbing",t.preventDefault(),t.stopPropagation()},Ot=(t)=>{if(!P||P.pointerId!==t.pointerId)return;zt({left:t.clientX-P.offsetX,top:t.clientY-P.offsetY}),t.preventDefault(),t.stopPropagation()},ut=(t)=>{if(!P||P.pointerId!==t.pointerId)return;if(P=null,p.style.cursor="grab",p.hasPointerCapture(t.pointerId))p.releasePointerCapture(t.pointerId);t.preventDefault(),t.stopPropagation()};p.addEventListener("pointerdown",qt),p.addEventListener("pointermove",Ot),p.addEventListener("pointerup",ut),p.addEventListener("pointercancel",ut);let mt=()=>{for(let t of u.values())Ht(t);Nt()},xn=(t,e)=>{for(let s of Array.from(u.values()).toReversed()){let b=s.element.getBoundingClientRect();if(t>=b.left&&t<=b.right&&e>=b.top&&e<=b.bottom)return kt(s),!0}let o=D.findIndex((s)=>t>=s.rect.x&&t<=s.rect.x+s.rect.width&&e>=s.rect.y&&e<=s.rect.y+s.rect.height);if(o>=0){let[s]=D.splice(o,1);return g.querySelector(`[data-region-id="${s?.id}"]`)?.remove(),j(),!0}let a=_.findIndex((s)=>t>=s.bounds.x&&t<=s.bounds.x+s.bounds.width&&e>=s.bounds.y&&e<=s.bounds.y+s.bounds.height);if(a>=0){let[s]=_.splice(a,1);return T.querySelector(`[data-stroke-id="${s?.id}"]`)?.remove(),j(),!0}return!1},yn=(t)=>{let e=Array.from(document.querySelectorAll("body *")).filter((o)=>!Q(o)).map((o)=>({element:o,rect:o.getBoundingClientRect()})).filter(({rect:o})=>{if(o.width<2||o.height<2)return!1;return!(o.right<t.x||o.left>t.x+t.width||o.bottom<t.y||o.top>t.y+t.height)}).filter(({element:o,rect:a})=>{let s=a.left+a.width/2,b=a.top+a.height/2;return s>=t.x&&s<=t.x+t.width&&b>=t.y&&b<=t.y+t.height&&(o.children.length===0||o instanceof HTMLButtonElement||o instanceof HTMLAnchorElement||o.getAttribute("role")==="button")}).sort((o,a)=>o.rect.width*o.rect.height-a.rect.width*a.rect.height).slice(0,Ln);for(let o of e)Mt(o.element);return e.length},it=()=>{M.style.display="none"},jt=(t)=>{if(!t.isTrusted)return;if(Q(t.target)){it();return}if(f==="select"&&k===null){let e=sn(t.clientX,t.clientY);if(e)vt(M,lt(e.getBoundingClientRect()));else it();return}if(it(),f==="marquee"&&k){vt(v,on(k.x,k.y,t.clientX,t.clientY));return}if(f==="draw"&&y)y.target.points=[...y.target.points,{x:t.clientX,y:t.clientY}],y.target.bounds=Pn(y.target.points,y.target.width),y.path.setAttribute("d",Nn(y.target.points))},Vt=(t)=>{if(!t.isTrusted||t.button!==0||Q(t.target))return;if(t.preventDefault(),t.stopPropagation(),f==="select"){let e=sn(t.clientX,t.clientY);if(e)dn(e,t.shiftKey);return}if(f==="erase"){xn(t.clientX,t.clientY);return}if(k={x:t.clientX,y:t.clientY},f==="draw"){let e={id:ht("stroke"),color:Ft?.primary??"#2563eb",width:4,points:[k],bounds:{x:k.x,y:k.y,width:1,height:1}},o=document.createElementNS("http://www.w3.org/2000/svg","path");o.setAttribute(E,""),o.setAttribute("data-stroke-id",e.id),o.setAttribute("fill","none"),o.setAttribute("stroke",e.color),o.setAttribute("stroke-width",String(e.width)),o.setAttribute("stroke-linecap","round"),o.setAttribute("stroke-linejoin","round"),T.appendChild(o),y={target:e,path:o}}},Kt=(t)=>{if(!t.isTrusted||!k)return;if(t.preventDefault(),t.stopPropagation(),f==="marquee"){let e=on(k.x,k.y,t.clientX,t.clientY);if(v.style.display="none",rn(e)){if(yn(e)===0){let a={id:ht("region"),rect:e};D.push(a);let s=ft(at,"color-mix(in srgb, var(--t3-primary) 6%, transparent)");s.setAttribute("data-region-id",a.id),vt(s,e),g.appendChild(s)}}}else if(f==="draw"&&y){if(y.target.points.length>1)_.push(y.target);else y.path.remove();y=null}k=null,j()},Z=(t)=>{if(Q(t.target))return;t.preventDefault(),t.stopPropagation()},Gt=(t)=>{if(t.relatedTarget===null)it()},Zt=()=>{it()},En=()=>{for(let t of u.values())t.element.removeAttribute($t),Ut(t)},Jt=()=>{if(r)return;if(r=!0,En(),window.removeEventListener("pointermove",jt,!0),window.removeEventListener("pointerdown",Vt,!0),window.removeEventListener("pointerup",Kt,!0),window.removeEventListener("pointerout",Gt,!0),window.removeEventListener("click",Z,!0),window.removeEventListener("dblclick",Z,!0),window.removeEventListener("contextmenu",Z,!0),window.removeEventListener("blur",Zt),window.removeEventListener("keydown",Qt,!0),window.removeEventListener("scroll",mt,!0),window.removeEventListener("resize",mt),p.removeEventListener("pointerdown",qt),p.removeEventListener("pointermove",Ot),p.removeEventListener("pointerup",ut),p.removeEventListener("pointercancel",ut),O!==null)window.cancelAnimationFrame(O);document.documentElement.removeAttribute(gt),N.remove(),c.remove(),dt=null,d(null)},Qt=(t)=>{if(!t.isTrusted)return;if(Q(t.target)&&t.key!=="Escape")return;if(t.key==="Escape"){if(t.preventDefault(),t.stopPropagation(),!yt)Jt();return}if(Mn(t)){t.preventDefault(),t.stopPropagation(),Pt();return}if(t.metaKey||t.ctrlKey||t.altKey)return;if(t.key==="v")f="select";else if(t.key==="r")f="marquee";else if(t.key==="d")f="draw";else if(t.key==="e")f="erase";else return;t.preventDefault(),t.stopPropagation(),Et()},Pt=()=>{if(yt||u.size===0&&D.length===0&&_.length===0)return;yt=!0,F.disabled=!0,F.textContent="Capturing…";let t=[...D],e=[..._],o=Array.from(tt.values(),(l)=>({...l})),a=Array.from(u.values(),(l)=>{let J=zn(l.element);return l.element.setAttribute($t,l.id),{id:l.id,element:J,rect:lt(l.element.getBoundingClientRect())}}),s={id:ht("annotation"),pageUrl:location.href,pageTitle:document.title?.trim()||null,elements:a,regions:t,strokes:e,styleChanges:o,screenshot:null,createdAt:new Date().toISOString()};h.style.display="none",X.style.display="none",M.style.display="none";let b=Array.from(u.values(),(l)=>lt(l.label.getBoundingClientRect())).filter(rn),m=an([...a.map((l)=>l.rect),...b,...t.map((l)=>l.rect),...e.map((l)=>l.bounds)]);window.requestAnimationFrame(()=>d({annotation:s,cropRect:m}))};F.addEventListener("click",(t)=>{if(t.isTrusted)Pt()}),g.addEventListener("keydown",(t)=>t.stopImmediatePropagation()),window.addEventListener("pointermove",jt,{capture:!0,passive:!1}),window.addEventListener("pointerdown",Vt,{capture:!0,passive:!1}),window.addEventListener("pointerup",Kt,{capture:!0,passive:!1}),window.addEventListener("pointerout",Gt,{capture:!0,passive:!0}),window.addEventListener("click",Z,{capture:!0,passive:!1}),window.addEventListener("dblclick",Z,{capture:!0,passive:!1}),window.addEventListener("contextmenu",Z,{capture:!0,passive:!1}),window.addEventListener("blur",Zt),window.addEventListener("keydown",Qt,{capture:!0}),window.addEventListener("scroll",mt,{capture:!0,passive:!0}),window.addEventListener("resize",mt,{passive:!0}),document.documentElement.appendChild(c),Et(),j(),dt={teardown:Jt,demo:{select:Mt,fontSize:(t)=>{if(!R)x.click();I.value=String(t),w("font-size",`${t}px`)},submit:Pt}}}var ln=globalThis;ln.__zeronAnnotate?.teardown();ln.__zeronAnnotate={start(n){return Ft=n,new Promise((r)=>$n(r))},teardown(){dt?.teardown()},demo(){return dt?.demo}};})();
