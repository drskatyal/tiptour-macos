// PCM16 framing helpers shared between mic capture and playback.
//
// Uplink to Gemini Live: 16 kHz mono PCM16 little-endian.
// Downlink from Gemini Live: 24 kHz mono PCM16 little-endian.
//
// Frames cross the Tauri IPC boundary as Uint8Array. Rust owns the actual
// audio devices via cpal; this module is the typed wire-format wrapper the
// frontend uses.

export const UPLINK_SAMPLE_RATE_HZ = 16_000;
export const DOWNLINK_SAMPLE_RATE_HZ = 24_000;

export function pcm16ToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export function base64ToPcm16(base64: string): Uint8Array {
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}
