// Personas tab — switch active persona, edit system prompt + model
// + provider + reasoning toggle, add custom personas. Built-ins are
// protected from delete; their fields are still editable.

import { invoke } from "@tauri-apps/api/core";

interface PersonaShape {
  id: string;
  name: string;
  systemPrompt: string;
  voiceTriggerPhrases: string[];
  isBuiltIn: boolean;
  modelProvider: string;
  modelId: string;
  reasoningEnabled: boolean;
  temperature: number;
}

interface ProviderCatalogEntry {
  id: string;
  displayName: string;
  supportsReasoning: boolean;
  suggestedModels: string[];
  keychainAccount: string;
}

export async function renderPersonasTab(paneElement: HTMLElement): Promise<void> {
  const allPersonas = await invoke<PersonaShape[]>("list_personas");
  const activePersona = await invoke<PersonaShape>("get_active_persona");
  const providers = await invoke<ProviderCatalogEntry[]>("list_providers");
  const activePersonaId = activePersona.id;

  paneElement.innerHTML = `
    <h2>Personas</h2>
    <p class="lede">
      Voice-switchable system-prompt presets. Each persona picks its own
      LLM provider, model, and reasoning mode — Gemini for vision-heavy
      flows, Cerebras/Groq/Fireworks for fast text rewrites, Anthropic
      or OpenAI when you want their specific behaviour.
    </p>

    <h3>API keys</h3>
    <p class="lede">
      Stored in the OS keychain. The active persona's provider must have
      a key configured for rewrite/draft features to work.
    </p>
    <div id="personas-provider-keys"></div>

    <h3>Personas</h3>
    <div class="button-stack">
      <button id="personas-add-new" class="primary">+ New persona</button>
    </div>

    <ul id="personas-list" class="personas-list" style="list-style:none;padding:0;margin:12px 0"></ul>

    <div id="personas-status-banner" class="flag-banner success" hidden></div>
  `;

  const personasListElement = paneElement.querySelector<HTMLUListElement>("#personas-list")!;
  const addNewButton = paneElement.querySelector<HTMLButtonElement>("#personas-add-new")!;
  const statusBannerElement = paneElement.querySelector<HTMLDivElement>(
    "#personas-status-banner",
  )!;
  const providerKeysContainer = paneElement.querySelector<HTMLDivElement>(
    "#personas-provider-keys",
  )!;

  function flashSavedBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1500);
  }

  // -------- Provider API key rows --------

  async function renderProviderKeyRows(): Promise<void> {
    providerKeysContainer.innerHTML = "";
    for (const provider of providers) {
      const existingKey = await invoke<string | null>("get_provider_api_key", {
        providerId: provider.id,
      });
      const row = document.createElement("div");
      row.className = "settings-row";
      row.innerHTML = `
        <label>${escapeHtml(provider.displayName)}</label>
        <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">
          <input
            type="password"
            data-provider-key-input="${escapeHtml(provider.id)}"
            placeholder="${existingKey ? "•••••••• stored" : "Paste API key"}"
            autocomplete="off"
            spellcheck="false"
            style="flex:1;min-width:240px"
          />
          <button data-provider-save="${escapeHtml(provider.id)}">Save</button>
          ${
            existingKey
              ? `<button class="danger" data-provider-clear="${escapeHtml(provider.id)}">Clear</button>`
              : ""
          }
          <span class="row-hint" style="flex-basis:100%">
            ${
              provider.supportsReasoning
                ? "Supports reasoning toggle."
                : "Non-reasoning fast-inference provider."
            }
            Stored as <code>${escapeHtml(provider.keychainAccount)}</code> in the OS keychain.
          </span>
        </div>
      `;
      providerKeysContainer.appendChild(row);
    }

    providerKeysContainer
      .querySelectorAll<HTMLButtonElement>("[data-provider-save]")
      .forEach((button) => {
        button.addEventListener("click", async () => {
          const providerId = button.dataset.providerSave!;
          const input = providerKeysContainer.querySelector<HTMLInputElement>(
            `[data-provider-key-input="${providerId}"]`,
          )!;
          const value = input.value.trim();
          if (!value) {
            flashSavedBanner("Key was empty.");
            return;
          }
          try {
            await invoke("set_provider_api_key", { providerId, key: value });
            flashSavedBanner(`Saved ${providerId} key.`);
            input.value = "";
            await renderProviderKeyRows();
          } catch (saveError) {
            flashSavedBanner(`Save failed: ${errorMessageOf(saveError)}`);
          }
        });
      });

    providerKeysContainer
      .querySelectorAll<HTMLButtonElement>("[data-provider-clear]")
      .forEach((button) => {
        button.addEventListener("click", async () => {
          const providerId = button.dataset.providerClear!;
          if (!window.confirm(`Clear ${providerId} API key from keychain?`)) return;
          try {
            await invoke("clear_provider_api_key", { providerId });
            flashSavedBanner(`Cleared ${providerId} key.`);
            await renderProviderKeyRows();
          } catch (clearError) {
            flashSavedBanner(`Clear failed: ${errorMessageOf(clearError)}`);
          }
        });
      });
  }
  await renderProviderKeyRows();

  // -------- Persona rows --------

  function renderPersonaRow(persona: PersonaShape): HTMLLIElement {
    const rowElement = document.createElement("li");
    rowElement.className = "persona-row";
    rowElement.style.border = "1px solid var(--color-border)";
    rowElement.style.borderRadius = "var(--radius-md)";
    rowElement.style.padding = "12px";
    rowElement.style.marginBottom = "8px";
    rowElement.style.background =
      persona.id === activePersonaId
        ? "var(--surface-raised-hover)"
        : "var(--surface-raised)";

    const headerElement = document.createElement("div");
    headerElement.style.display = "flex";
    headerElement.style.justifyContent = "space-between";
    headerElement.style.alignItems = "center";
    headerElement.style.marginBottom = "8px";

    const nameElement = document.createElement("strong");
    nameElement.textContent = persona.name;
    if (persona.id === activePersonaId) {
      const activeBadge = document.createElement("span");
      activeBadge.textContent = " · active";
      activeBadge.style.color = "var(--accent-400)";
      activeBadge.style.fontWeight = "normal";
      nameElement.appendChild(activeBadge);
    }

    const buttonsElement = document.createElement("div");
    if (persona.id !== activePersonaId) {
      const activateButton = document.createElement("button");
      activateButton.textContent = "Activate";
      activateButton.addEventListener("click", async () => {
        try {
          await invoke("set_active_persona", { id: persona.id });
          flashSavedBanner(`Activated "${persona.name}".`);
          await renderPersonasTab(paneElement);
        } catch (activateError) {
          flashSavedBanner(`Activate failed: ${errorMessageOf(activateError)}`);
        }
      });
      buttonsElement.appendChild(activateButton);
    }
    if (!persona.isBuiltIn) {
      const deleteButton = document.createElement("button");
      deleteButton.textContent = "Delete";
      deleteButton.className = "danger";
      deleteButton.style.marginLeft = "6px";
      deleteButton.addEventListener("click", async () => {
        if (!window.confirm(`Delete persona "${persona.name}"?`)) return;
        try {
          await invoke("delete_persona", { id: persona.id });
          flashSavedBanner(`Deleted "${persona.name}".`);
          await renderPersonasTab(paneElement);
        } catch (deleteError) {
          flashSavedBanner(`Delete failed: ${errorMessageOf(deleteError)}`);
        }
      });
      buttonsElement.appendChild(deleteButton);
    }

    headerElement.appendChild(nameElement);
    headerElement.appendChild(buttonsElement);
    rowElement.appendChild(headerElement);

    // Provider + model + reasoning grid.
    const modelGrid = document.createElement("div");
    modelGrid.style.display = "grid";
    modelGrid.style.gridTemplateColumns = "1fr 1fr auto auto";
    modelGrid.style.gap = "8px";
    modelGrid.style.marginBottom = "8px";

    const providerSelect = document.createElement("select");
    for (const provider of providers) {
      const opt = document.createElement("option");
      opt.value = provider.id;
      opt.textContent = provider.displayName;
      if (provider.id === persona.modelProvider) opt.selected = true;
      providerSelect.appendChild(opt);
    }

    const modelDatalistId = `models-${persona.id}`;
    const modelInput = document.createElement("input");
    modelInput.type = "text";
    modelInput.setAttribute("list", modelDatalistId);
    modelInput.value = persona.modelId;
    modelInput.placeholder = "model id";
    const modelDatalist = document.createElement("datalist");
    modelDatalist.id = modelDatalistId;
    const refreshModelDatalist = (providerId: string): void => {
      modelDatalist.innerHTML = "";
      const found = providers.find((p) => p.id === providerId);
      for (const id of found?.suggestedModels ?? []) {
        const opt = document.createElement("option");
        opt.value = id;
        modelDatalist.appendChild(opt);
      }
    };
    refreshModelDatalist(persona.modelProvider);

    const reasoningLabel = document.createElement("label");
    reasoningLabel.style.display = "flex";
    reasoningLabel.style.alignItems = "center";
    reasoningLabel.style.gap = "6px";
    reasoningLabel.style.color = "var(--text-secondary)";
    reasoningLabel.style.fontSize = "12px";
    const reasoningInput = document.createElement("input");
    reasoningInput.type = "checkbox";
    reasoningInput.checked = persona.reasoningEnabled;
    const reasoningSpan = document.createElement("span");
    reasoningSpan.textContent = "Reasoning";
    reasoningLabel.appendChild(reasoningInput);
    reasoningLabel.appendChild(reasoningSpan);
    const applyReasoningAvailability = (providerId: string): void => {
      const provider = providers.find((p) => p.id === providerId);
      const supports = provider?.supportsReasoning ?? false;
      reasoningInput.disabled = !supports;
      reasoningLabel.style.opacity = supports ? "1" : "0.5";
      reasoningLabel.title = supports
        ? "Enable thinking / extended reasoning for this model"
        : "This provider doesn't expose a reasoning toggle";
    };
    applyReasoningAvailability(persona.modelProvider);

    const tempInput = document.createElement("input");
    tempInput.type = "number";
    tempInput.min = "0";
    tempInput.max = "2";
    tempInput.step = "0.1";
    tempInput.value = String(persona.temperature);
    tempInput.style.width = "76px";
    tempInput.title = "Sampling temperature (0 = deterministic, 1 = creative)";

    providerSelect.addEventListener("change", () => {
      refreshModelDatalist(providerSelect.value);
      applyReasoningAvailability(providerSelect.value);
      const newDefault = providers.find((p) => p.id === providerSelect.value)
        ?.suggestedModels[0];
      if (newDefault) modelInput.value = newDefault;
    });

    modelGrid.appendChild(providerSelect);
    modelGrid.appendChild(modelInput);
    modelGrid.appendChild(reasoningLabel);
    modelGrid.appendChild(tempInput);
    rowElement.appendChild(modelGrid);
    rowElement.appendChild(modelDatalist);

    const promptTextarea = document.createElement("textarea");
    promptTextarea.value = persona.systemPrompt;
    promptTextarea.rows = 4;
    promptTextarea.style.width = "100%";
    promptTextarea.style.fontFamily = "var(--font-mono)";
    promptTextarea.style.fontSize = "12px";

    const triggersInput = document.createElement("input");
    triggersInput.type = "text";
    triggersInput.value = persona.voiceTriggerPhrases.join(", ");
    triggersInput.placeholder = "voice trigger phrases (comma-separated)";
    triggersInput.style.width = "100%";
    triggersInput.style.marginTop = "6px";

    const saveEditsButton = document.createElement("button");
    saveEditsButton.textContent = "Save edits";
    saveEditsButton.style.marginTop = "6px";
    saveEditsButton.addEventListener("click", async () => {
      const updated: PersonaShape = {
        id: persona.id,
        name: persona.name,
        systemPrompt: promptTextarea.value,
        voiceTriggerPhrases: triggersInput.value
          .split(",")
          .map((entry) => entry.trim())
          .filter((entry) => entry.length > 0),
        isBuiltIn: persona.isBuiltIn,
        modelProvider: providerSelect.value,
        modelId: modelInput.value.trim(),
        reasoningEnabled: reasoningInput.checked && !reasoningInput.disabled,
        temperature: Number.isFinite(parseFloat(tempInput.value))
          ? parseFloat(tempInput.value)
          : 0.4,
      };
      try {
        await invoke("upsert_custom_persona", { persona: updated });
        flashSavedBanner("Saved.");
      } catch (saveError) {
        flashSavedBanner(`Save failed: ${errorMessageOf(saveError)}`);
      }
    });

    rowElement.appendChild(promptTextarea);
    rowElement.appendChild(triggersInput);
    rowElement.appendChild(saveEditsButton);
    return rowElement;
  }

  for (const persona of allPersonas) {
    personasListElement.appendChild(renderPersonaRow(persona));
  }

  addNewButton.addEventListener("click", async () => {
    const newName = window.prompt("Persona name:");
    if (!newName) return;
    const slugId = newName
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
    const newPersona: PersonaShape = {
      id: `custom-${slugId}-${Date.now()}`,
      name: newName,
      systemPrompt: "You are TipTour. Reply concisely.",
      voiceTriggerPhrases: [newName.toLowerCase()],
      isBuiltIn: false,
      modelProvider: "gemini",
      modelId: "gemini-2.5-flash",
      reasoningEnabled: false,
      temperature: 0.4,
    };
    try {
      await invoke("upsert_custom_persona", { persona: newPersona });
      flashSavedBanner(`Added "${newName}".`);
      await renderPersonasTab(paneElement);
    } catch (addError) {
      flashSavedBanner(`Add failed: ${errorMessageOf(addError)}`);
    }
  });
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
