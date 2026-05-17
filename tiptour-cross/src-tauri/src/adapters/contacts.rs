// Contacts adapter.
//
// macOS reads the local Contacts.app database via OSA. Windows reads
// the People store via PowerShell + the WinRT Contacts namespace.
// Linux returns an empty result (no standard system contacts store).
//
// The big win: "send Sara a message" / "email Sara" works without
// the user typing a phone number or email. Gemini calls
// control_app(contacts, lookup, {name}) before the actual messaging
// dispatch and substitutes the resolved address in the next call.
//
// Handlers:
//   lookup   { name } -> { matches: [{name, emails, phones}] }
//   suggest  { partial } -> same shape, prefix-matched, count<=8
//
// No auth — these are local OS-owned stores the user already
// populated. We never network the contacts anywhere.

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::helpers::parse_args;
use super::{AdapterAuth, AdapterCapability, AdapterManifest};

pub fn manifest() -> AdapterManifest {
    AdapterManifest {
        slug: "contacts".into(),
        name: "Contacts".into(),
        description: "Resolve names to emails and phone numbers using the OS Contacts store. Used by other adapters before they send messages.".into(),
        category: "productivity".into(),
        voice_triggers: vec![
            "find sara's email".into(),
            "look up contact".into(),
            "who is".into(),
        ],
        #[cfg(target_os = "macos")]
        capabilities: vec![AdapterCapability::Osascript],
        #[cfg(not(target_os = "macos"))]
        capabilities: vec![AdapterCapability::Spawn],
        auth: AdapterAuth::None,
        supported_platforms: vec!["macos".into(), "windows".into()],
        setup_notes: "Reads the OS Contacts database directly. macOS will prompt for Contacts permission on first lookup — grant it in System Settings → Privacy & Security → Contacts.".into(),
    }
}

pub async fn dispatch(_app: AppHandle, handler: &str, args: Value) -> Result<Value, String> {
    match handler {
        "lookup" => lookup(args),
        "suggest" => suggest(args),
        other => Err(format!("contacts: no handler '{other}'")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupArgs {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SuggestArgs {
    partial: String,
}

fn lookup(args: Value) -> Result<Value, String> {
    let parsed: LookupArgs = parse_args(args)?;
    let matches = query_contacts(&parsed.name, 8)?;
    Ok(json!({ "matches": matches }))
}

fn suggest(args: Value) -> Result<Value, String> {
    let parsed: SuggestArgs = parse_args(args)?;
    let matches = query_contacts(&parsed.partial, 8)?;
    Ok(json!({ "matches": matches }))
}

// ---------- platform-specific contact queries ----------

#[cfg(target_os = "macos")]
fn query_contacts(needle: &str, limit: usize) -> Result<Vec<Value>, String> {
    use super::helpers::{osa_escape, run_osascript};
    // Build a script that finds people whose first/last/full name
    // contains the needle case-insensitively. Returns a |-separated
    // record per match: name|email1,email2|phone1,phone2
    let escaped = osa_escape(needle);
    let script = format!(
        "tell application \"Contacts\"\n\
            set foundPeople to (people whose (name contains \"{escaped}\") or (first name contains \"{escaped}\") or (last name contains \"{escaped}\"))\n\
            set outputLines to {{}}\n\
            repeat with personEntry in foundPeople\n\
                set personName to name of personEntry\n\
                set personEmails to value of every email of personEntry\n\
                set personPhones to value of every phone of personEntry\n\
                set emailsString to my joinList(personEmails, \",\")\n\
                set phonesString to my joinList(personPhones, \",\")\n\
                set end of outputLines to personName & \"||\" & emailsString & \"||\" & phonesString\n\
            end repeat\n\
            return my joinList(outputLines, \"<<<\")\n\
        end tell\n\
        on joinList(theList, theDelimiter)\n\
            set savedDelimiters to AppleScript's text item delimiters\n\
            set AppleScript's text item delimiters to theDelimiter\n\
            set theResult to theList as string\n\
            set AppleScript's text item delimiters to savedDelimiters\n\
            return theResult\n\
        end joinList"
    );

    let output = match run_osascript(&script) {
        Ok(o) => o,
        Err(error) => {
            // The most common error is "Not authorised to send Apple
            // events to Contacts" — surface the specific remedy.
            if error.contains("not authoriz") || error.contains("not allowed") || error.contains("-1743") {
                return Err(
                    "Contacts access denied. Grant TipTour Contacts access in System Settings → Privacy & Security → Contacts.".into(),
                );
            }
            return Err(error);
        }
    };

    let mut results: Vec<Value> = Vec::new();
    for record in output.split("<<<").take(limit) {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let parts: Vec<&str> = record.splitn(3, "||").collect();
        if parts.is_empty() {
            continue;
        }
        let name = parts.first().map(|s| s.trim()).unwrap_or("").to_string();
        let emails: Vec<&str> = parts
            .get(1)
            .map(|s| s.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect())
            .unwrap_or_default();
        let phones: Vec<&str> = parts
            .get(2)
            .map(|s| s.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect())
            .unwrap_or_default();
        if name.is_empty() && emails.is_empty() && phones.is_empty() {
            continue;
        }
        results.push(json!({ "name": name, "emails": emails, "phones": phones }));
    }
    Ok(results)
}

#[cfg(target_os = "windows")]
fn query_contacts(needle: &str, limit: usize) -> Result<Vec<Value>, String> {
    use super::helpers::spawn_cli;
    // PowerShell + WinRT bridge. Querying the People store via
    // Windows.ApplicationModel.Contacts.ContactStore. The store
    // requires the "contacts" capability declared in the app
    // manifest, which Tauri's installer flow handles.
    let ps_script = format!(
        r#"$key = '{needle_escaped}'
        Add-Type -AssemblyName System.Runtime.WindowsRuntime
        $taskMethod = ([System.WindowsRuntimeSystemExtensions].GetMethods() | ? {{ $_.Name -eq 'AsTask' -and $_.GetParameters().Length -eq 1 }}) | Select-Object -First 1
        function Await($a, $t) {{
          $i = $taskMethod.MakeGenericMethod($t)
          $task = $i.Invoke($null, @($a))
          $task.Wait() | Out-Null
          $task.Result
        }}
        $storeType = [Windows.ApplicationModel.Contacts.ContactManager,Windows.ApplicationModel.Contacts,ContentType=WindowsRuntime]
        $store = Await ($storeType::RequestStoreAsync()) ([Windows.ApplicationModel.Contacts.ContactStore])
        if ($store -eq $null) {{ Write-Error 'Contacts store unavailable'; exit 1 }}
        $contacts = Await ($store.FindContactsAsync($key)) ([System.Collections.Generic.IReadOnlyList[Windows.ApplicationModel.Contacts.Contact]])
        $top = $contacts | Select-Object -First {limit}
        $top | ForEach-Object {{
          $emails = ($_.Emails | ForEach-Object {{ $_.Address }}) -join ','
          $phones = ($_.Phones | ForEach-Object {{ $_.Number }}) -join ','
          "$($_.DisplayName)||$emails||$phones"
        }}"#,
        needle_escaped = needle.replace('\'', "''"),
        limit = limit
    );
    let output = spawn_cli("powershell", &["-NoProfile", "-Command", &ps_script])?;
    let mut results: Vec<Value> = Vec::new();
    for line in output.lines().take(limit) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, "||").collect();
        let name = parts.first().map(|s| s.trim()).unwrap_or("").to_string();
        let emails: Vec<&str> = parts
            .get(1)
            .map(|s| s.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect())
            .unwrap_or_default();
        let phones: Vec<&str> = parts
            .get(2)
            .map(|s| s.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect())
            .unwrap_or_default();
        results.push(json!({ "name": name, "emails": emails, "phones": phones }));
    }
    Ok(results)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn query_contacts(_needle: &str, _limit: usize) -> Result<Vec<Value>, String> {
    Err("Contacts lookup is not supported on this OS yet.".into())
}
