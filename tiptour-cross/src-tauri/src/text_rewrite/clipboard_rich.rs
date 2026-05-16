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
