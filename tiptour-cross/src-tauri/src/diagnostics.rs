// Diagnostics: runs lightweight probes so the user can sanity-check
// "is the thing wired up" without a debugger. Every probe returns the
// same shape so the UI can render a uniform pass/fail table, and a
// "Copy bundle" button can dump the full result as JSON for bug reports.

use serde::Serialize;

use crate::adapters;
use crate::keychain;

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub name: String,
    pub category: String,
    pub ok: bool,
    pub detail: String,
}

fn probe_provider_key(slug: &str, label: &str) -> DiagnosticReport {
    match keychain::read_provider_key(slug) {
        Ok(Some(value)) => DiagnosticReport {
            name: label.into(),
            category: "credentials".into(),
            ok: true,
            // Don't leak the key — just confirm presence + length so
            // the user can spot a truncated paste.
            detail: format!("present ({} chars)", value.len()),
        },
        Ok(None) => DiagnosticReport {
            name: label.into(),
            category: "credentials".into(),
            ok: false,
            detail: "no key set".into(),
        },
        Err(error) => DiagnosticReport {
            name: label.into(),
            category: "credentials".into(),
            ok: false,
            detail: error,
        },
    }
}

fn probe_data_dir() -> DiagnosticReport {
    match dirs::data_local_dir() {
        Some(mut path) => {
            path.push("TipTour");
            let exists = path.exists();
            let writable = std::fs::create_dir_all(&path).is_ok();
            DiagnosticReport {
                name: "Local data directory".into(),
                category: "storage".into(),
                ok: writable,
                detail: format!(
                    "{} ({})",
                    path.display(),
                    if exists { "exists" } else { "created" }
                ),
            }
        }
        None => DiagnosticReport {
            name: "Local data directory".into(),
            category: "storage".into(),
            ok: false,
            detail: "no OS data_local_dir available".into(),
        },
    }
}

fn probe_enabled_adapters() -> DiagnosticReport {
    let enabled = adapters::enabled_adapter_slugs_with_capability_hints();
    DiagnosticReport {
        name: "Enabled adapters".into(),
        category: "adapters".into(),
        ok: !enabled.is_empty(),
        detail: if enabled.is_empty() {
            "none — enable some in Connected apps".into()
        } else {
            enabled
                .iter()
                .map(|(slug, _)| slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
    }
}

async fn probe_brave_search() -> DiagnosticReport {
    let key = match keychain::read_provider_key("brave-search") {
        Ok(Some(value)) => value,
        _ => {
            return DiagnosticReport {
                name: "Brave Search reachability".into(),
                category: "network".into(),
                ok: false,
                detail: "no Brave Search key set".into(),
            };
        }
    };
    // One-token query — Brave's cheapest probe shape. We only check
    // that the auth handshake completes; result content is irrelevant.
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return DiagnosticReport {
                name: "Brave Search reachability".into(),
                category: "network".into(),
                ok: false,
                detail: format!("build client: {error}"),
            };
        }
    };
    let response = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("Accept", "application/json")
        .header("X-Subscription-Token", &key)
        .query(&[("q", "ping"), ("count", "1")])
        .send()
        .await;
    match response {
        Ok(resp) if resp.status().is_success() => DiagnosticReport {
            name: "Brave Search reachability".into(),
            category: "network".into(),
            ok: true,
            detail: format!("HTTP {}", resp.status()),
        },
        Ok(resp) => DiagnosticReport {
            name: "Brave Search reachability".into(),
            category: "network".into(),
            ok: false,
            detail: format!("HTTP {}", resp.status()),
        },
        Err(error) => DiagnosticReport {
            name: "Brave Search reachability".into(),
            category: "network".into(),
            ok: false,
            detail: format!("{error}"),
        },
    }
}

async fn probe_gemini_reachability() -> DiagnosticReport {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return DiagnosticReport {
                name: "Gemini API reachability".into(),
                category: "network".into(),
                ok: false,
                detail: format!("build client: {error}"),
            };
        }
    };
    // We don't need a key for a connectivity probe: any GET on the
    // models endpoint returns a structured 4xx without a key, which
    // proves the network/DNS path is intact.
    let response = client
        .get("https://generativelanguage.googleapis.com/v1beta/models")
        .send()
        .await;
    match response {
        Ok(resp) => DiagnosticReport {
            name: "Gemini API reachability".into(),
            category: "network".into(),
            // 4xx is fine here — we're checking the TLS+DNS path, not auth.
            ok: resp.status().is_success() || resp.status().is_client_error(),
            detail: format!("HTTP {}", resp.status()),
        },
        Err(error) => DiagnosticReport {
            name: "Gemini API reachability".into(),
            category: "network".into(),
            ok: false,
            detail: format!("{error}"),
        },
    }
}

#[tauri::command]
pub async fn run_diagnostics() -> Result<Vec<DiagnosticReport>, String> {
    let mut reports: Vec<DiagnosticReport> = Vec::new();

    // Credentials — only check the providers we ship UI for. Missing
    // keys for unused providers shouldn't dirty the report.
    reports.push(probe_provider_key("gemini", "Gemini API key"));
    reports.push(probe_provider_key("soniox", "Soniox API key"));
    reports.push(probe_provider_key("brave-search", "Brave Search API key"));

    reports.push(probe_data_dir());
    reports.push(probe_enabled_adapters());

    // Network probes can run concurrently — each is independent.
    let (gemini_report, brave_report) =
        tokio::join!(probe_gemini_reachability(), probe_brave_search());
    reports.push(gemini_report);
    reports.push(brave_report);

    Ok(reports)
}
