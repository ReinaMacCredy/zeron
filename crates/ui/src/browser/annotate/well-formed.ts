// A cut emoji or a page title like "\uD800" leaves a lone surrogate, which serde_json refuses.
// String.prototype.toWellFormed needs Safari 16.4; the app runs from macOS 12.
export const wellFormed = (_key: string, value: unknown): unknown =>
  typeof value === "string"
    ? value.replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]|[\uD800-\uDFFF]/g, (unit) =>
        unit.length === 2 ? unit : "\uFFFD",
      )
    : value;
