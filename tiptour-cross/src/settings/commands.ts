// Voice Commands tab — built-in immutable list + user-defined custom
// commands wired to keyboard shortcuts or system commands.

import { invoke } from "@tauri-apps/api/core";

type CustomVoiceCommandActionKind = "keyboardShortcut" | "systemCommand";

interface CustomVoiceCommand {
  id: string;
  name: string;
  voicePhrase: string;
  actionKind: CustomVoiceCommandActionKind;
  actionPayload: string;
}

const IMMUTABLE_SYSTEM_COMMANDS: { phrase: string; description: string }[] = [
  { phrase: "stop", description: "Hang up the Gemini session." },
  { phrase: "cancel", description: "Cancel the active workflow." },
  { phrase: "pause", description: "Pause the in-flight replay." },
  { phrase: "do <flow>", description: "Run a saved flow by name." },
  { phrase: "run <flow>", description: "Run a saved flow by name." },
  { phrase: "open <app>", description: "Locally launch a discovered app." },
  { phrase: "launch <app>", description: "Locally launch a discovered app." },
  { phrase: "start <app>", description: "Locally launch a discovered app." },
  { phrase: "go to <app>", description: "Locally launch a discovered app." },
];

interface DiscoveredApp {
  canonicalId: string;
  displayName: string;
  launchTarget: { kind: "bundleId" | "executablePath"; value: string };
  aliases: string[];
  isUserEnabled: boolean;
}

export async function renderCommandsTab(paneElement: HTMLElement): Promise<void> {
  const customCommands = await invoke<CustomVoiceCommand[]>("list_custom_voice_commands").catch(
    () => [] as CustomVoiceCommand[],
  );

  paneElement.innerHTML = `
    <h2>Voice Commands</h2>
    <p class="lede">
      When always-on listening is enabled on the General tab, TipTour's local
      Vosk model runs on-device and matches what you say after the wake word
      ("hey tiptour") against this command list. Built-in commands cover the
      core voice surface; add custom ones below for app-specific shortcuts
      ("open the pull-request inbox", "queue today's standup notes").
    </p>

    <h3>Built-in (read-only)</h3>
    <p class="section-helper">Shipped with the app and always live. These can't be removed.</p>
    <ul class="list-rows" id="commands-builtin"></ul>

    <h3>Auto-detected app commands</h3>
    <p class="lede" id="discovered-apps-status">Scanning installed applications…</p>
    <div class="filter-bar">
      <input type="search" id="discovered-apps-search" class="filter-bar-input" placeholder="Filter by name or alias" />
      <button id="discovered-apps-rescan">Re-scan installed apps</button>
    </div>
    <ul class="list-rows" id="discovered-apps-list"></ul>

    <h3>Custom</h3>
    <p class="section-helper">
      Define your own voice phrases that fire a keyboard shortcut, run a CLI
      command, or call an adapter handler. Each command takes a trigger
      phrase + action; add as many aliases as you'd say in conversation.
    </p>
    <div id="commands-custom-editors"></div>
    <div class="button-stack">
      <button id="commands-add" class="primary">Add custom command</button>
      <button id="commands-reload">Reload commands</button>
    </div>
    <div id="commands-status-banner" class="flag-banner success" hidden></div>
  `;

  const builtinListElement = paneElement.querySelector<HTMLUListElement>("#commands-builtin")!;
  for (const builtinCommand of IMMUTABLE_SYSTEM_COMMANDS) {
    const rowElement = document.createElement("li");
    rowElement.className = "list-row disabled";
    rowElement.innerHTML = `
      <div class="row-main">
        <span class="row-title">${escapeHtml(builtinCommand.phrase)}</span>
        <span class="row-sub">${escapeHtml(builtinCommand.description)}</span>
      </div>
    `;
    builtinListElement.appendChild(rowElement);
  }

  const discoveredAppsListElement = paneElement.querySelector<HTMLUListElement>(
    "#discovered-apps-list",
  )!;
  const discoveredAppsStatusElement = paneElement.querySelector<HTMLParagraphElement>(
    "#discovered-apps-status",
  )!;
  const discoveredAppsSearchElement = paneElement.querySelector<HTMLInputElement>(
    "#discovered-apps-search",
  )!;
  const discoveredAppsRescanButtonElement = paneElement.querySelector<HTMLButtonElement>(
    "#discovered-apps-rescan",
  )!;

  let currentDiscoveredApps: DiscoveredApp[] = [];

  function renderDiscoveredApps(filterSubstring: string): void {
    discoveredAppsListElement.innerHTML = "";
    const lowerFilter = filterSubstring.trim().toLowerCase();
    const filtered = currentDiscoveredApps.filter((app) => {
      if (lowerFilter === "") return true;
      if (app.displayName.toLowerCase().includes(lowerFilter)) return true;
      return app.aliases.some((alias) => alias.toLowerCase().includes(lowerFilter));
    });
    const enabledCount = currentDiscoveredApps.filter((app) => app.isUserEnabled).length;
    discoveredAppsStatusElement.textContent = `${currentDiscoveredApps.length} apps detected — ${enabledCount} enabled.`;

    for (const app of filtered) {
      const rowElement = document.createElement("li");
      rowElement.className = "list-row";
      const samplePhrases = app.aliases
        .slice(0, 2)
        .map((alias) => `"open ${alias}"`)
        .join(", ");
      rowElement.innerHTML = `
        <div class="row-main">
          <span class="row-title">${escapeHtml(app.displayName)}</span>
          <span class="row-sub">say: ${escapeHtml(samplePhrases || "(no aliases)")}</span>
        </div>
        <div class="row-actions">
          <label class="toggle">
            <input type="checkbox" data-canonical-id="${escapeHtml(app.canonicalId)}" ${
              app.isUserEnabled ? "checked" : ""
            } />
            <span>Enabled</span>
          </label>
        </div>
      `;
      const toggleInputElement = rowElement.querySelector<HTMLInputElement>(
        "input[type='checkbox']",
      )!;
      toggleInputElement.addEventListener("change", async () => {
        try {
          await invoke("set_app_command_enabled", {
            canonicalId: app.canonicalId,
            enabled: toggleInputElement.checked,
          });
          app.isUserEnabled = toggleInputElement.checked;
          const newEnabledCount = currentDiscoveredApps.filter((a) => a.isUserEnabled).length;
          discoveredAppsStatusElement.textContent = `${currentDiscoveredApps.length} apps detected — ${newEnabledCount} enabled.`;
        } catch (toggleError) {
          flashBanner(`Toggle failed: ${errorMessageOf(toggleError)}`);
          toggleInputElement.checked = app.isUserEnabled;
        }
      });
      discoveredAppsListElement.appendChild(rowElement);
    }
  }

  async function loadDiscoveredApps(): Promise<void> {
    try {
      currentDiscoveredApps = await invoke<DiscoveredApp[]>("list_discovered_apps");
    } catch (loadError) {
      discoveredAppsStatusElement.textContent = `Discovery failed: ${errorMessageOf(loadError)}`;
      currentDiscoveredApps = [];
    }
    renderDiscoveredApps(discoveredAppsSearchElement.value);
  }

  discoveredAppsSearchElement.addEventListener("input", () => {
    renderDiscoveredApps(discoveredAppsSearchElement.value);
  });

  discoveredAppsRescanButtonElement.addEventListener("click", async () => {
    discoveredAppsRescanButtonElement.disabled = true;
    discoveredAppsStatusElement.textContent = "Re-scanning…";
    try {
      currentDiscoveredApps = await invoke<DiscoveredApp[]>("rescan_installed_apps");
      renderDiscoveredApps(discoveredAppsSearchElement.value);
      flashBanner("Re-scan complete.");
    } catch (rescanError) {
      flashBanner(`Re-scan failed: ${errorMessageOf(rescanError)}`);
    } finally {
      discoveredAppsRescanButtonElement.disabled = false;
    }
  });

  void loadDiscoveredApps();

  const customEditorsContainer = paneElement.querySelector<HTMLDivElement>(
    "#commands-custom-editors",
  )!;
  const addButtonElement = paneElement.querySelector<HTMLButtonElement>("#commands-add")!;
  const reloadButtonElement = paneElement.querySelector<HTMLButtonElement>("#commands-reload")!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#commands-status-banner",
  )!;

  function flashBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1800);
  }

  function renderCustomEditors(commandList: CustomVoiceCommand[]): void {
    customEditorsContainer.innerHTML = "";
    if (commandList.length === 0) {
      const emptyHint = document.createElement("p");
      emptyHint.className = "lede";
      emptyHint.textContent = "No custom commands yet. Add one to bind a phrase to a keyboard shortcut or system command.";
      customEditorsContainer.appendChild(emptyHint);
      return;
    }
    for (const command of commandList) {
      customEditorsContainer.appendChild(renderSingleCommandEditor(command));
    }
  }

  function renderSingleCommandEditor(command: CustomVoiceCommand): HTMLElement {
    const editorRow = document.createElement("div");
    editorRow.className = "command-editor-grid";
    editorRow.innerHTML = `
      <input type="text" placeholder="name" value="${escapeHtml(command.name)}" data-field="name" />
      <input type="text" placeholder="voice phrase" value="${escapeHtml(
        command.voicePhrase,
      )}" data-field="voicePhrase" />
      <select data-field="actionKind">
        <option value="keyboardShortcut">Keyboard shortcut</option>
        <option value="systemCommand">System command</option>
      </select>
      <input type="text" placeholder="payload (e.g. Ctrl+Shift+P)" value="${escapeHtml(
        command.actionPayload,
      )}" data-field="actionPayload" />
      <div class="row-actions">
        <button data-action="save">Save</button>
        <button data-action="delete" class="danger">Delete</button>
      </div>
    `;
    const actionKindSelectElement = editorRow.querySelector<HTMLSelectElement>(
      "select[data-field='actionKind']",
    )!;
    actionKindSelectElement.value = command.actionKind;

    editorRow
      .querySelector<HTMLButtonElement>("button[data-action='save']")!
      .addEventListener("click", async () => {
        const updated: CustomVoiceCommand = {
          id: command.id,
          name:
            editorRow.querySelector<HTMLInputElement>("input[data-field='name']")!.value ||
            "(unnamed)",
          voicePhrase:
            editorRow.querySelector<HTMLInputElement>("input[data-field='voicePhrase']")!.value,
          actionKind: actionKindSelectElement.value as CustomVoiceCommandActionKind,
          actionPayload:
            editorRow.querySelector<HTMLInputElement>("input[data-field='actionPayload']")!.value,
        };
        try {
          await invoke("upsert_custom_voice_command", { command: updated });
          flashBanner("Saved.");
        } catch (saveError) {
          flashBanner(`Save failed: ${errorMessageOf(saveError)}`);
        }
      });

    editorRow
      .querySelector<HTMLButtonElement>("button[data-action='delete']")!
      .addEventListener("click", async () => {
        try {
          await invoke("delete_custom_voice_command", { id: command.id });
          flashBanner("Deleted.");
          editorRow.remove();
        } catch (deleteError) {
          flashBanner(`Delete failed: ${errorMessageOf(deleteError)}`);
        }
      });
    return editorRow;
  }

  addButtonElement.addEventListener("click", () => {
    const draftCommand: CustomVoiceCommand = {
      id: `cmd_${Date.now()}_${Math.floor(Math.random() * 1000)}`,
      name: "",
      voicePhrase: "",
      actionKind: "keyboardShortcut",
      actionPayload: "",
    };
    customEditorsContainer.appendChild(renderSingleCommandEditor(draftCommand));
  });

  reloadButtonElement.addEventListener("click", async () => {
    try {
      // Cycle the listener off + on so it rebuilds its grammar with the
      // updated custom-command list. Stubbed: the Vosk side doesn't yet
      // read the custom_voice_commands file when building grammars, so
      // this reload only re-applies built-ins. Tracked for follow-up.
      const isListenerCurrentlyEnabled = await invoke<boolean>("is_listener_enabled");
      if (isListenerCurrentlyEnabled) {
        await invoke("set_listener_enabled", { enabled: false });
        await invoke("set_listener_enabled", { enabled: true });
      }
      flashBanner("Reloaded. Custom-command grammar integration is on the roadmap.");
    } catch (reloadError) {
      flashBanner(`Reload failed: ${errorMessageOf(reloadError)}`);
    }
  });

  renderCustomEditors(customCommands);
}

function escapeHtml(rawString: string): string {
  return rawString
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
