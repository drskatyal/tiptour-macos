// Microsoft Office for Windows — Word, Excel, PowerPoint, Outlook
// desktop. Each app exposes a COM automation surface that we drive
// via `powershell -Command "..."` invocations of `New-Object
// -ComObject Word.Application` (and equivalents). PowerShell is on
// every Windows 10/11 box by default so no extra setup.
//
// We keep all four apps in one file because they share the same
// PowerShell+COM shape — distinct manifests + handler tables, but
// the runner is one helper.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::parse_args;
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

fn office_manifest(slug: &str, name: &str, description: &str, triggers: Vec<String>) -> AdapterManifest {
    AdapterManifest {
        slug: slug.into(),
        name: name.into(),
        description: description.into(),
        category: "productivity".into(),
        voice_triggers: triggers,
        capabilities: vec![AdapterCapability::Spawn],
        auth: AdapterAuth::None,
        supported_platforms: vec!["windows".into()],
        setup_notes: "Drives the desktop app via PowerShell + COM. Requires the Office app installed.".into(),
    }
}

pub fn word_manifest() -> AdapterManifest {
    office_manifest(
        "word",
        "Microsoft Word",
        "Open documents, create blank docs, save as PDF.",
        vec!["open word".into(), "new word document".into(), "save as pdf in word".into()],
    )
}
pub fn excel_manifest() -> AdapterManifest {
    office_manifest(
        "excel",
        "Microsoft Excel",
        "Open workbooks, create blank spreadsheets.",
        vec!["open excel".into(), "new excel".into(), "new spreadsheet".into()],
    )
}
pub fn powerpoint_manifest() -> AdapterManifest {
    office_manifest(
        "powerpoint",
        "Microsoft PowerPoint",
        "Open decks, create blank presentations, start slideshow.",
        vec!["open powerpoint".into(), "new presentation".into(), "start slideshow".into()],
    )
}
pub fn outlook_desktop_manifest() -> AdapterManifest {
    office_manifest(
        "outlook-desktop",
        "Outlook (desktop)",
        "Compose a new email through the desktop Outlook client.",
        vec!["compose outlook email".into(), "new outlook mail".into()],
    )
}

#[cfg(target_os = "windows")]
fn run_powershell(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|error| format!("powershell spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "powershell: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(not(target_os = "windows"))]
fn run_powershell(_script: &str) -> Result<String, String> {
    // Soft-fail on non-Windows so the workspace compiles. The
    // manifests above declare windows-only support so the settings
    // UI hides these rows on macOS/Linux and the dispatch never
    // actually reaches this branch.
    Err("Office adapters are Windows-only".to_string())
}

// ---------- Word ----------

pub async fn dispatch_word(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open" => {
            let p: PathArgs = parse_args(args)?;
            run_powershell(&format!(
                "$w = New-Object -ComObject Word.Application; $w.Visible = $true; \
                 $w.Documents.Open('{}')",
                ps_escape(&p.path)
            ))?;
            Ok(json!({ "opened": p.path }))
        }
        "new_document" => {
            run_powershell(
                "$w = New-Object -ComObject Word.Application; $w.Visible = $true; \
                 [void]$w.Documents.Add()",
            )?;
            Ok(json!({ "created": true }))
        }
        "save_as_pdf" => {
            let p: SaveAsPdfArgs = parse_args(args)?;
            // wdFormatPDF = 17. Save the currently active document
            // to the requested output path.
            run_powershell(&format!(
                "$w = New-Object -ComObject Word.Application; $w.Visible = $true; \
                 $doc = $w.Documents.Open('{}'); \
                 $doc.SaveAs2('{}', 17); $doc.Close()",
                ps_escape(&p.input_path),
                ps_escape(&p.output_path)
            ))?;
            Ok(json!({ "saved": p.output_path }))
        }
        other => Err(format!("word: no handler '{other}'")),
    }
}

// ---------- Excel ----------

pub async fn dispatch_excel(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open" => {
            let p: PathArgs = parse_args(args)?;
            run_powershell(&format!(
                "$x = New-Object -ComObject Excel.Application; $x.Visible = $true; \
                 [void]$x.Workbooks.Open('{}')",
                ps_escape(&p.path)
            ))?;
            Ok(json!({ "opened": p.path }))
        }
        "new_workbook" => {
            run_powershell(
                "$x = New-Object -ComObject Excel.Application; $x.Visible = $true; \
                 [void]$x.Workbooks.Add()",
            )?;
            Ok(json!({ "created": true }))
        }
        other => Err(format!("excel: no handler '{other}'")),
    }
}

// ---------- PowerPoint ----------

pub async fn dispatch_powerpoint(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "open" => {
            let p: PathArgs = parse_args(args)?;
            run_powershell(&format!(
                "$p = New-Object -ComObject PowerPoint.Application; $p.Visible = $true; \
                 [void]$p.Presentations.Open('{}')",
                ps_escape(&p.path)
            ))?;
            Ok(json!({ "opened": p.path }))
        }
        "new_presentation" => {
            run_powershell(
                "$p = New-Object -ComObject PowerPoint.Application; $p.Visible = $true; \
                 [void]$p.Presentations.Add()",
            )?;
            Ok(json!({ "created": true }))
        }
        "start_slideshow" => {
            // Slide.SlideShowSettings.Run starts the show on the
            // currently active presentation.
            run_powershell(
                "$p = New-Object -ComObject PowerPoint.Application; $p.Visible = $true; \
                 [void]$p.ActivePresentation.SlideShowSettings.Run()",
            )?;
            Ok(json!({ "started": true }))
        }
        other => Err(format!("powerpoint: no handler '{other}'")),
    }
}

// ---------- Outlook desktop ----------

pub async fn dispatch_outlook_desktop(
    _app: AppHandle,
    handler: &str,
    args: Value,
) -> Result<Value, String> {
    match handler {
        "compose" => {
            let p: ComposeArgs = parse_args(args)?;
            // 0 = olMailItem. We open the draft visible and DON'T
            // auto-send — the user reviews + clicks send themselves
            // since auto-send is a footgun for desktop email.
            run_powershell(&format!(
                "$o = New-Object -ComObject Outlook.Application; \
                 $mail = $o.CreateItem(0); \
                 $mail.To = '{}'; \
                 $mail.Subject = '{}'; \
                 $mail.Body = '{}'; \
                 $mail.Display($true)",
                ps_escape(&p.to),
                ps_escape(&p.subject),
                ps_escape(&p.body),
            ))?;
            Ok(json!({ "drafted": true }))
        }
        other => Err(format!("outlook-desktop: no handler '{other}'")),
    }
}

// ---------- shared argument shapes ----------

#[derive(Deserialize)]
struct PathArgs { path: String }
#[derive(Deserialize)]
struct SaveAsPdfArgs { input_path: String, output_path: String }
#[derive(Deserialize)]
struct ComposeArgs { to: String, subject: String, body: String }

/// PowerShell single-quoted string escape: single-quote becomes
/// two single-quotes. Backslashes and double-quotes pass through.
/// We rely on '...' string semantics for variable-non-expansion.
fn ps_escape(input: &str) -> String {
    input.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_escape_doubles_quotes() {
        assert_eq!(ps_escape("hello"), "hello");
        assert_eq!(ps_escape("it's mine"), "it''s mine");
        assert_eq!(ps_escape("a\\b"), "a\\b");
    }
}
