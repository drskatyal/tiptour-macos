// Connected apps tab — lists every bundled adapter (Spotify,
// WhatsApp, …) with install + enable controls.
//
// Adapter lifecycle on this page:
//   1. List arrives from `list_adapters`; each row carries
//      manifest + isInstalled + isEnabled.
//   2. "Install": ApiKey adapters show an inline credential row
//      (paste, save into the per-slug keychain entry). None
//      adapters skip straight to enabled. OAuth2 adapters open
//      the help URL (v2 will run a real PKCE redirect).
//   3. Enable toggle persists into adapters-enabled.json so the
//      orchestrator's dispatch_adapter_command rejects calls to
//      disabled adapters.

import { invoke } from "@tauri-apps/api/core";

interface AdapterAuth {
  kind: "none" | "api-key" | "o-auth2";
  label?: string;
  helpUrl?: string;
  scopes?: string[];
}
interface AdapterListing {
  slug: string;
  name: string;
  description: string;
  category: string;
  voiceTriggers: string[];
  capabilities: string[];
  auth: AdapterAuth;
  supportedPlatforms: string[];
  setupNotes: string;
  isInstalled: boolean;
  isEnabled: boolean;
}

const CATEGORY_DISPLAY: Record<string, { label: string; icon: string }> = {
  music: { label: "Music & Audio", icon: "♪" },
  communication: { label: "Communication", icon: "✉" },
  productivity: { label: "Productivity", icon: "▤" },
  dev: { label: "Development", icon: "⌘" },
  files: { label: "Files & Storage", icon: "▾" },
};

const isMacOsPlatform = navigator.platform.toLowerCase().includes("mac");
const isWindowsPlatform = navigator.platform.toLowerCase().includes("win");
const hostPlatform = isMacOsPlatform
  ? "macos"
  : isWindowsPlatform
    ? "windows"
    : "linux";

export async function renderAdaptersTab(paneElement: HTMLElement): Promise<void> {
  const all = await invoke<AdapterListing[]>("list_adapters");
  // Filter to adapters that support this OS so the user doesn't see
  // greyed-out rows for unreachable integrations.
  const compatible = all.filter((adapter) =>
    adapter.supportedPlatforms.includes(hostPlatform),
  );

  paneElement.innerHTML = `
    <h2>Connected apps</h2>
    <p class="lede">
      Adapters that let TipTour control your favourite apps by voice.
      Each one declares the capabilities it needs — clipboard, network,
      keychain — so you can decide what to grant.
    </p>
    <div id="adapters-status-banner" class="flag-banner success" hidden></div>
    <div id="adapters-categories"></div>
  `;

  const banner = paneElement.querySelector<HTMLDivElement>("#adapters-status-banner")!;
  const categoriesContainer = paneElement.querySelector<HTMLDivElement>(
    "#adapters-categories",
  )!;

  function flash(message: string): void {
    banner.textContent = message;
    banner.hidden = false;
    setTimeout(() => {
      banner.hidden = true;
    }, 1600);
  }

  if (compatible.length === 0) {
    categoriesContainer.innerHTML = `
      <div class="adapters-empty-state">
        <strong>No adapters available on this platform.</strong>
        <p>
          Adapters ship with the binary and target macOS, Windows, or Linux.
          If you're seeing this on a supported OS, the binary may have shipped
          without any bundled adapters — file a bug.
        </p>
      </div>
    `;
    return;
  }

  // Group adapters by category so the user scans music / messaging /
  // dev separately rather than reading a flat list.
  const byCategory = new Map<string, AdapterListing[]>();
  for (const adapter of compatible) {
    const list = byCategory.get(adapter.category) ?? [];
    list.push(adapter);
    byCategory.set(adapter.category, list);
  }

  for (const [categoryKey, adaptersInCategory] of byCategory) {
    const categoryMeta = CATEGORY_DISPLAY[categoryKey] ?? {
      label: categoryKey,
      icon: "•",
    };
    const section = document.createElement("section");
    section.className = "adapters-category";

    const header = document.createElement("h3");
    header.textContent = categoryMeta.label;
    section.appendChild(header);

    for (const adapter of adaptersInCategory) {
      section.appendChild(renderAdapterRow(adapter, flash));
    }
    categoriesContainer.appendChild(section);
  }
}

function renderAdapterRow(
  adapter: AdapterListing,
  flash: (message: string) => void,
): HTMLElement {
  const row = document.createElement("div");
  row.className = "adapter-row";
  row.dataset.slug = adapter.slug;

  // Header line: name + capability chips + enable toggle.
  const header = document.createElement("div");
  header.className = "adapter-row-header";

  const nameWrap = document.createElement("div");
  nameWrap.className = "adapter-row-name";
  const nameElement = document.createElement("strong");
  nameElement.textContent = adapter.name;
  nameWrap.appendChild(nameElement);
  if (adapter.isInstalled) {
    const installedBadge = document.createElement("span");
    installedBadge.className = "adapter-badge installed";
    installedBadge.textContent = adapter.auth.kind === "none" ? "Ready" : "Authorized";
    nameWrap.appendChild(installedBadge);
  }

  const toggleLabel = document.createElement("label");
  toggleLabel.className = "adapter-toggle";
  const toggleInput = document.createElement("input");
  toggleInput.type = "checkbox";
  toggleInput.checked = adapter.isEnabled;
  toggleInput.disabled = !adapter.isInstalled;
  toggleInput.addEventListener("change", async () => {
    try {
      await invoke("set_adapter_enabled", {
        slug: adapter.slug,
        enabled: toggleInput.checked,
      });
      flash(
        toggleInput.checked
          ? `Enabled ${adapter.name}.`
          : `Disabled ${adapter.name}.`,
      );
    } catch (error) {
      flash(`Couldn't toggle ${adapter.name}: ${errorMessageOf(error)}`);
      toggleInput.checked = !toggleInput.checked;
    }
  });
  toggleLabel.appendChild(toggleInput);

  header.appendChild(nameWrap);
  header.appendChild(toggleLabel);
  row.appendChild(header);

  // Description + capability chips.
  const description = document.createElement("p");
  description.className = "adapter-row-description";
  description.textContent = adapter.description;
  row.appendChild(description);

  const chipsRow = document.createElement("div");
  chipsRow.className = "adapter-chips";
  for (const cap of adapter.capabilities) {
    const chip = document.createElement("span");
    chip.className = "adapter-chip";
    chip.textContent = humanizeCapability(cap);
    chipsRow.appendChild(chip);
  }
  row.appendChild(chipsRow);

  // Install flow.
  if (!adapter.isInstalled) {
    row.appendChild(renderInstallSection(adapter, flash));
  } else if (adapter.setupNotes) {
    const notes = document.createElement("p");
    notes.className = "adapter-setup-notes";
    notes.textContent = adapter.setupNotes;
    row.appendChild(notes);
  }

  return row;
}

function renderInstallSection(
  adapter: AdapterListing,
  flash: (message: string) => void,
): HTMLElement {
  const section = document.createElement("div");
  section.className = "adapter-install";

  if (adapter.setupNotes) {
    const notes = document.createElement("p");
    notes.className = "adapter-setup-notes";
    notes.textContent = adapter.setupNotes;
    section.appendChild(notes);
  }

  if (adapter.auth.kind === "none") {
    // No auth required — surface a single Install button that
    // marks the adapter as enabled. The "installed" state for a
    // None adapter is "user has clicked install", which we model
    // by enabling it on click.
    const installButton = document.createElement("button");
    installButton.textContent = "Install";
    installButton.className = "primary";
    installButton.addEventListener("click", async () => {
      try {
        await invoke("set_adapter_enabled", { slug: adapter.slug, enabled: true });
        flash(`Installed ${adapter.name}.`);
        const paneRoot = document.getElementById("settings-pane");
        if (paneRoot) await renderAdaptersTab(paneRoot);
      } catch (error) {
        flash(`Install failed: ${errorMessageOf(error)}`);
      }
    });
    section.appendChild(installButton);
    return section;
  }

  if (adapter.auth.kind === "api-key") {
    const inputRow = document.createElement("div");
    inputRow.className = "adapter-install-input-row";
    const labelText = adapter.auth.label ?? "API key";
    const label = document.createElement("label");
    label.textContent = labelText;
    inputRow.appendChild(label);

    const input = document.createElement("input");
    input.type = "password";
    input.autocomplete = "off";
    input.spellcheck = false;
    input.placeholder = `Paste ${labelText}`;
    inputRow.appendChild(input);

    const saveButton = document.createElement("button");
    saveButton.textContent = "Save & enable";
    saveButton.className = "primary";
    saveButton.addEventListener("click", async () => {
      const value = input.value.trim();
      if (!value) {
        flash("Token was empty.");
        return;
      }
      try {
        await invoke("set_provider_api_key", {
          providerId: adapter.slug,
          key: value,
        });
        await invoke("set_adapter_enabled", { slug: adapter.slug, enabled: true });
        flash(`Saved ${adapter.name} token and enabled.`);
        const paneRoot = document.getElementById("settings-pane");
        if (paneRoot) await renderAdaptersTab(paneRoot);
      } catch (error) {
        flash(`Save failed: ${errorMessageOf(error)}`);
      }
    });
    inputRow.appendChild(saveButton);
    section.appendChild(inputRow);

    if (adapter.auth.helpUrl) {
      const helpLink = document.createElement("a");
      helpLink.href = adapter.auth.helpUrl;
      helpLink.target = "_blank";
      helpLink.rel = "noreferrer noopener";
      helpLink.textContent = "Get a token →";
      helpLink.className = "adapter-help-link";
      section.appendChild(helpLink);
    }
    return section;
  }

  // OAuth2 placeholder — v1 just sends the user to the help URL.
  const oauthButton = document.createElement("button");
  oauthButton.textContent = "Open authorization page";
  oauthButton.addEventListener("click", () => {
    if (adapter.auth.helpUrl) window.open(adapter.auth.helpUrl, "_blank");
  });
  section.appendChild(oauthButton);
  return section;
}

function humanizeCapability(cap: string): string {
  switch (cap) {
    case "clipboard": return "Clipboard";
    case "network": return "Network";
    case "keychain": return "Keychain";
    case "spawn": return "Run programs";
    case "open-uri": return "Open URLs";
    case "osascript": return "AppleScript";
    case "keystrokes": return "Send keys";
    default: return cap;
  }
}

function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
