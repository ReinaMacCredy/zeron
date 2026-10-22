// Page world: React fiber expandos live on the page's DOM wrappers, invisible to the overlay's world.
import { isSourceFile } from "bippy/source";
import { getElementContext } from "react-grab/primitives";
import { wellFormed } from "./well-formed.ts";

const ELEMENT_ATTRIBUTE = "data-zeron-annotate";
const ELEMENT_CONTEXT_TIMEOUT_MS = 2_000;
const CONTEXT_TEXT_LIMIT = 4_000;

interface Frame {
  functionName?: string;
  fileName?: string;
  lineNumber?: number;
  columnNumber?: number;
}

const isReadableName = (name: string | null): name is string =>
  name !== null && /^[A-Z][\w$]{2,}$/.test(name);

const toStackFrame = (frame: Frame) => ({
  functionName: frame.functionName ?? null,
  fileName: frame.fileName ?? null,
  lineNumber: frame.lineNumber ?? null,
  columnNumber: frame.columnNumber ?? null,
});

function withTimeout<A>(promise: Promise<A>, millis: number): Promise<A | null> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return Promise.race([
    promise,
    new Promise<null>((resolve) => {
      timer = setTimeout(() => resolve(null), millis);
    }),
  ]).finally(() => clearTimeout(timer));
}

async function grab(id: string) {
  const element = document.querySelector(`[${ELEMENT_ATTRIBUTE}="${CSS.escape(id)}"]`);
  if (!element) return null;
  element.removeAttribute(ELEMENT_ATTRIBUTE);
  try {
    const context = await withTimeout(
      Promise.resolve(getElementContext(element)),
      ELEMENT_CONTEXT_TIMEOUT_MS,
    );
    if (!context) return null;
    const stack = (context.stack ?? []).map(toStackFrame);
    return {
      id,
      componentName: isReadableName(context.componentName) ? context.componentName : null,
      source:
        stack.find(
          (frame) =>
            isReadableName(frame.functionName) &&
            frame.fileName !== null &&
            isSourceFile(frame.fileName),
        ) ?? null,
      selector: context.selector,
      htmlPreview: (context.htmlPreview ?? "").slice(0, CONTEXT_TEXT_LIMIT),
      styles: (context.styles ?? "").slice(0, CONTEXT_TEXT_LIMIT),
    };
  } catch {
    return null;
  }
}

(globalThis as { __zeronAnnotateGrab?: unknown }).__zeronAnnotateGrab = async (
  ids: string[],
): Promise<string> =>
  JSON.stringify((await Promise.all(ids.map(grab))).filter(Boolean), wellFormed);
