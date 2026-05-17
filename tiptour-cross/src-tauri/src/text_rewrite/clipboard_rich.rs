// Rich-text clipboard helper.
//
// When a rewrite completes we want the user to be able to Cmd+V it
// into Word / Google Docs / Mail / Notion and have the formatting
// survive. Those targets read the `text/html` clipboard slot when
// available and fall back to plain text otherwise. macOS apps also
// honour the RTF flavor, but arboard's HTML support is sufficient
// for Word/Docs/Mail (they all accept HTML paste).
//
// On the macOS/Windows native API levels arboard maps:
//   - `set_text` -> CF_TEXT / NSStringPboardType
//   - `set_html` -> CF_HTML / NSHTMLPboardType + text/html
//
// We always set both so plain-text consumers still get something.

use arboard::Clipboard;

/// Put markdown (as plain text) AND its HTML rendering on the
/// system clipboard. Word, Docs, Mail, and Notion will pick the
/// HTML flavor; terminals and code editors get the plain markdown.
pub fn put_markdown_and_html(markdown: &str, html: &str) -> Result<(), String> {
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("clipboard open: {error}"))?;
    // arboard 3.x exposes `set` + `Set` for combined flavors; we
    // use the chained builder if available, otherwise fall back to
    // setting text first then html (the second call overwrites the
    // primary flavor on some platforms — accepting that trade-off
    // because the rich consumers we care about read HTML).
    clipboard
        .set_html(html, Some(markdown))
        .map_err(|error| format!("clipboard set_html: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn put_clipboard_rich(markdown: String, html: String) -> Result<(), String> {
    put_markdown_and_html(&markdown, &html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_rewrite::markdown_to_html;

    #[test]
    fn html_output_round_trips_basic_markdown() {
        // We don't actually exercise the clipboard from tests (it
        // requires a display server / mac WindowServer / Win
        // pasteboard daemon — none of which exist on Linux CI). But
        // we DO exercise the markdown -> html pipeline that the
        // clipboard helper depends on, so a regression in
        // pulldown-cmark configuration shows up as a test failure
        // rather than a silent paste of unrendered markdown.
        let markdown = "# Title\n\nParagraph with **bold** + *italic* + `code`.\n\n- item 1\n- item 2";
        let html = markdown_to_html(markdown);
        assert!(html.contains("<h1>Title</h1>"), "missing h1: {html}");
        assert!(html.contains("<strong>bold</strong>"), "missing strong: {html}");
        assert!(html.contains("<em>italic</em>"), "missing em: {html}");
        assert!(html.contains("<code>code</code>"), "missing code: {html}");
        assert!(html.contains("<li>item 1</li>"), "missing li: {html}");
    }

    #[test]
    fn tables_render_as_html_tables() {
        // Tables are critical for spreadsheet-style rewrites; if the
        // `ENABLE_TABLES` option ever drops off, this test catches it
        // before the user sees their meeting notes paste as plain text.
        let markdown = "| col a | col b |\n|-------|-------|\n| 1     | 2     |";
        let html = markdown_to_html(markdown);
        assert!(html.contains("<table>"), "tables disabled: {html}");
        assert!(html.contains("<th>col a</th>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn task_lists_render_as_checkbox_lis() {
        // Task lists are how rewrite-as-action-items output looks.
        let markdown = "- [ ] open thing\n- [x] done thing";
        let html = markdown_to_html(markdown);
        assert!(html.contains("type=\"checkbox\""), "task lists disabled: {html}");
    }

    #[test]
    fn html_is_safe_to_set_via_arboard_signature() {
        // We don't actually call put_markdown_and_html here because
        // arboard on Linux CI has no X11 display and would return
        // ClipboardNotSupported. But we verify the function exists
        // with the expected (markdown, html) signature so a future
        // refactor that drops the markdown plain-text fallback
        // shows up as a compile error.
        let _: fn(&str, &str) -> Result<(), String> = put_markdown_and_html;
    }
}
