// Drafting window — the rich-text surface where LLM rewrites land.
//
// Lifecycle:
//   1. Boots empty, listens for `drafting_seed_text` (Tauri event)
//      and `rewrite_complete` (the result envelope from
//      text_rewrite::rewrite_selection).
//   2. User can edit the markdown via contenteditable + a tiny
//      formatting toolbar (Bold / Italic / H2 / List / Code).
//   3. "Copy" writes both markdown and the rendered HTML to the
//      clipboard so paste targets pick the best flavor.
//   4. "Replace selection" sends the rich payload back to the
//      foreground app via the ActionExecutor clipboard-paste path.
//
// We deliberately use contenteditable + execCommand rather than
// pulling in a heavy editor framework (TipTap/ProseMirror) because
// the surface stays simple: bold/italic/headings/lists/code is the
// whole formatting surface for LLM drafts.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { installThemeBridge } from "../theme";

interface RewriteResponse {
  markdown: string;
  html: string;
  provider: string;
  model: string;
  elapsedMs: number;
}

const editorElement = document.getElementById("drafting-editor") as HTMLDivElement;
const statusElement = document.getElementById("drafting-status") as HTMLSpanElement;
const statsElement = document.getElementById("drafting-stats") as HTMLSpanElement;
const provenanceElement = document.getElementById("drafting-provenance") as HTMLSpanElement;

function setStatus(text: string): void {
  statusElement.textContent = text;
}
function updateStats(): void {
  const plain = editorElement.innerText.trim();
  const wordCount = plain.length === 0 ? 0 : plain.split(/\s+/).length;
  statsElement.textContent = `${wordCount} word${wordCount === 1 ? "" : "s"}`;
}

// Minimal Markdown -> HTML for the seed text. The Rust side also
// produces HTML, but seeding from the panel "Open in editor" path
// gives us only markdown, so we render it here too.
function markdownToInlineHtml(markdown: string): string {
  // Block-level: headings, paragraphs, lists.
  const lines = markdown.split("\n");
  const out: string[] = [];
  let inList = false;
  for (const line of lines) {
    if (/^#{1,6}\s/.test(line)) {
      if (inList) {
        out.push("</ul>");
        inList = false;
      }
      const level = (line.match(/^#+/) ?? [""])[0].length;
      const text = line.replace(/^#+\s+/, "");
      out.push(`<h${level}>${inline(text)}</h${level}>`);
    } else if (/^[-*]\s/.test(line)) {
      if (!inList) {
        out.push("<ul>");
        inList = true;
      }
      out.push(`<li>${inline(line.replace(/^[-*]\s+/, ""))}</li>`);
    } else if (line.trim().length === 0) {
      if (inList) {
        out.push("</ul>");
        inList = false;
      }
    } else {
      if (inList) {
        out.push("</ul>");
        inList = false;
      }
      out.push(`<p>${inline(line)}</p>`);
    }
  }
  if (inList) out.push("</ul>");
  return out.join("\n");
}
function inline(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
    .replace(/\*(.+?)\*/g, "<em>$1</em>")
    .replace(/`([^`]+)`/g, "<code>$1</code>");
}

function seedFromMarkdown(markdown: string): void {
  editorElement.innerHTML = markdownToInlineHtml(markdown);
  updateStats();
  setStatus("Draft ready");
}

function applyProvenance(rewrite: RewriteResponse): void {
  provenanceElement.textContent = `via ${rewrite.provider} · ${rewrite.model} · ${rewrite.elapsedMs}ms`;
}

// ---------- Toolbar bindings ----------

function bindFormatButton(id: string, command: string, value?: string): void {
  const button = document.getElementById(id);
  if (!button) return;
  button.addEventListener("click", (event) => {
    event.preventDefault();
    editorElement.focus();
    // execCommand is deprecated but still the simplest way to
    // mutate a contenteditable on every browser in use. The
    // replacement (Selection + Range manipulation) would triple
    // this file's size for no real win on our limited surface.
    document.execCommand(command, false, value);
    updateStats();
  });
}
bindFormatButton("drafting-format-bold", "bold");
bindFormatButton("drafting-format-italic", "italic");
bindFormatButton("drafting-format-h2", "formatBlock", "h2");
bindFormatButton("drafting-format-list", "insertUnorderedList");
bindFormatButton("drafting-format-code", "formatBlock", "pre");

// Keyboard shortcuts for the common pair so user muscle memory works.
editorElement.addEventListener("keydown", (event) => {
  if (!(event.ctrlKey || event.metaKey)) return;
  if (event.key === "b" || event.key === "B") {
    event.preventDefault();
    document.execCommand("bold");
  } else if (event.key === "i" || event.key === "I") {
    event.preventDefault();
    document.execCommand("italic");
  }
});
editorElement.addEventListener("input", updateStats);

// ---------- Copy + Replace ----------

document.getElementById("drafting-copy-rich")?.addEventListener("click", async () => {
  try {
    const markdown = editorElement.innerText;
    const html = editorElement.innerHTML;
    await invoke("put_clipboard_rich", { markdown, html });
    setStatus("Copied (rich + plain)");
    setTimeout(() => setStatus("Draft ready"), 1400);
  } catch (copyError) {
    setStatus(`Copy failed: ${errorMessageOf(copyError)}`);
  }
});

document
  .getElementById("drafting-replace-selection")
  ?.addEventListener("click", async () => {
    try {
      const markdown = editorElement.innerText;
      const html = editorElement.innerHTML;
      await invoke("put_clipboard_rich", { markdown, html });
      // The replace path mirrors the ActionExecutor clipboard paste:
      // synthesize Cmd/Ctrl+V into whatever the foreground field is.
      // Tiny delay first so the source app has a chance to refocus
      // after the user clicks our button.
      await new Promise((resolve) => setTimeout(resolve, 120));
      await invoke("paste_from_drafting_window");
      setStatus("Pasted into selection");
      setTimeout(() => setStatus("Draft ready"), 1400);
    } catch (replaceError) {
      // The paste command may not be wired yet on every build —
      // surface a useful message and leave the rich clipboard
      // payload so the user can still Cmd+V manually.
      setStatus(
        `Replace failed: ${errorMessageOf(replaceError)} — clipboard still holds the draft`,
      );
    }
  });

// ---------- Boot ----------

await installThemeBridge();

await listen<string>("drafting_seed_text", (event) => {
  if (typeof event.payload === "string") {
    seedFromMarkdown(event.payload);
  }
});

await listen<RewriteResponse>("rewrite_complete", (event) => {
  if (event.payload?.markdown !== undefined) {
    seedFromMarkdown(event.payload.markdown);
    applyProvenance(event.payload);
  }
});

updateStats();

function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
