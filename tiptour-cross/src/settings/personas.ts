// Personas tab — switch active persona, edit system prompt inline,
// add custom personas. Built-ins are protected from delete; their
// fields are still editable so the user can dial them in.

import { invoke } from "@tauri-apps/api/core";

interface PersonaShape {
  id: string;
  name: string;
  systemPrompt: string;
  voiceTriggerPhrases: string[];
  isBuiltIn: boolean;
}

export async function renderPersonasTab(paneElement: HTMLElement): Promise<void> {
  const allPersonas = await invoke<PersonaShape[]>("list_personas");
  const activePersona = await invoke<PersonaShape>("get_active_persona");
  const activePersonaId = activePersona.id;

  paneElement.innerHTML = `
    <h2>Personas</h2>
    <p class="lede">System-prompt presets injected into Gemini at session open. Click to activate; edit inline; or add a new one.</p>

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

  function flashSavedBanner(message: string): void {
    statusBannerElement.textContent = message;
    statusBannerElement.hidden = false;
    setTimeout(() => {
      statusBannerElement.hidden = true;
    }, 1500);
  }

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
    headerElement.style.marginBottom = "6px";

    const nameElement = document.createElement("strong");
    nameElement.textContent = persona.name;
    if (persona.id === activePersonaId) {
      const activeBadge = document.createElement("span");
      activeBadge.textContent = " · active";
      activeBadge.style.color = "var(--accent-500)";
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

function errorMessageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
