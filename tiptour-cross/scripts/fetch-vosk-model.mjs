// Fetch + unzip the Vosk small-en-us model into
// `src-tauri/resources/vosk-models/small-en-us/` so the Tauri bundler
// can include it as a runtime resource. After this script lands the
// model in the resources tree, `tauri.conf.json.bundle.resources`
// picks it up and the shipped .app/.exe contains the model — no
// runtime download required for end users.
//
// Idempotent: re-runs become no-ops once the model is on disk. Safe
// to call from CI (it caches via the unpacked dir). The download is
// retried up to 3 times with exponential backoff for transient CDN
// flakes.
//
// Usage:
//   node scripts/fetch-vosk-model.mjs           # fetches if missing
//   npm run fetch:vosk                           # same, npm-wrapped
//
// The Tauri bundle pipeline calls this via `prebuild:tauri`.

import { spawn } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, statSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..");
const MODEL_NAME = "small-en-us";
const MODEL_DIR = join(REPO_ROOT, "src-tauri", "resources", "vosk-models", MODEL_NAME);
const MODEL_ZIP_URL =
  "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip";
const TMP_ZIP = join(MODEL_DIR, "..", "small-en-us.zip");
const SENTINEL_FILE = join(MODEL_DIR, "README"); // any non-empty file inside

function logStep(text) {
  console.log(`[fetch-vosk] ${text}`);
}

function modelIsAlreadyPresent() {
  // The Vosk model directory contains, among others, a README and
  // an `am/final.mdl` blob. We sentinel on the README rather than
  // touching the audio model so a quick file probe is enough.
  if (!existsSync(SENTINEL_FILE)) return false;
  const stat = statSync(SENTINEL_FILE);
  return stat.size > 0;
}

async function downloadWithRetry(url, destinationPath) {
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      logStep(`download attempt ${attempt}/3 → ${url}`);
      const response = await fetch(url);
      if (!response.ok) {
        throw new Error(`${response.status} ${response.statusText}`);
      }
      const contentLength = Number(response.headers.get("content-length") ?? 0);
      const fileStream = createWriteStream(destinationPath);
      const reader = response.body.getReader();
      let bytesReceived = 0;
      let lastPrintedPercent = -10;
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        fileStream.write(Buffer.from(value));
        bytesReceived += value.length;
        if (contentLength > 0) {
          const percent = Math.floor((bytesReceived / contentLength) * 100);
          if (percent >= lastPrintedPercent + 10) {
            process.stdout.write(`\r[fetch-vosk] ${percent}% (${(bytesReceived / 1024 / 1024).toFixed(1)} MB)`);
            lastPrintedPercent = percent;
          }
        }
      }
      fileStream.end();
      await new Promise((resolveStream) => fileStream.on("close", resolveStream));
      process.stdout.write("\n");
      return;
    } catch (error) {
      console.warn(`[fetch-vosk] attempt ${attempt} failed: ${error.message}`);
      if (attempt === 3) {
        throw new Error(
          `Vosk model download failed 3 times. Network may block alphacephei.com. ` +
            `Last error: ${error.message}`,
        );
      }
      const delayMs = 1500 * 3 ** (attempt - 1);
      logStep(`backing off ${delayMs}ms…`);
      await new Promise((resolveDelay) => setTimeout(resolveDelay, delayMs));
    }
  }
}

function runCommand(command, args, options = {}) {
  return new Promise((resolveProc, rejectProc) => {
    const child = spawn(command, args, { stdio: "inherit", ...options });
    child.on("error", rejectProc);
    child.on("exit", (code) => {
      if (code === 0) resolveProc();
      else rejectProc(new Error(`${command} exited with code ${code}`));
    });
  });
}

async function unzipPreservingDirStructureStrippingTopLevel(zipPath, destinationDir) {
  // The upstream zip nests everything under a top-level
  // `vosk-model-small-en-us-0.15/` directory. We want our destination
  // to be the model root, so unzip + move the inner directory's
  // contents up one level. Cross-platform: use `unzip` on macOS/Linux
  // and `tar.exe -xf` (which understands zip) on Windows.
  await mkdir(destinationDir, { recursive: true });
  if (process.platform === "win32") {
    // Windows 10+ ships tar.exe with zip support.
    await runCommand("tar", ["-xf", zipPath, "-C", destinationDir]);
  } else {
    await runCommand("unzip", ["-q", "-o", zipPath, "-d", destinationDir]);
  }

  // Find the single top-level subdir and lift its contents.
  const fs = await import("node:fs/promises");
  const topLevelEntries = await fs.readdir(destinationDir, { withFileTypes: true });
  const topLevelDirs = topLevelEntries.filter(
    (entry) => entry.isDirectory() && entry.name.startsWith("vosk-model"),
  );
  if (topLevelDirs.length !== 1) {
    throw new Error(
      `Expected exactly one vosk-model* directory after unzip, found: ${topLevelDirs
        .map((d) => d.name)
        .join(", ")}`,
    );
  }
  const nestedDir = join(destinationDir, topLevelDirs[0].name);
  const nestedEntries = await fs.readdir(nestedDir, { withFileTypes: true });
  for (const entry of nestedEntries) {
    await fs.rename(join(nestedDir, entry.name), join(destinationDir, entry.name));
  }
  await fs.rmdir(nestedDir);
}

async function main() {
  if (modelIsAlreadyPresent()) {
    logStep(`model already present at ${MODEL_DIR}, skipping fetch.`);
    return;
  }
  mkdirSync(dirname(TMP_ZIP), { recursive: true });
  logStep(`downloading Vosk small-en-us model (~40MB)…`);
  await downloadWithRetry(MODEL_ZIP_URL, TMP_ZIP);
  logStep(`unzipping into ${MODEL_DIR}…`);
  mkdirSync(MODEL_DIR, { recursive: true });
  await unzipPreservingDirStructureStrippingTopLevel(TMP_ZIP, MODEL_DIR);
  // Clean up the zip — Tauri's bundler doesn't need it.
  const { unlink } = await import("node:fs/promises");
  await unlink(TMP_ZIP).catch(() => {});
  logStep(`done. Model is at ${MODEL_DIR}.`);
}

main().catch((error) => {
  console.error("[fetch-vosk] FAILED:", error.message);
  process.exit(1);
});
