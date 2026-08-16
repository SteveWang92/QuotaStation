// Regenerates the Windows application icon from the master artwork.
//
// `tauri icon` writes every icon every platform could want — Android densities, iOS sizes,
// an .icns, a dozen Windows Store tiles — and QuotaStation is a Windows desktop application
// that bundles exactly one of them. Generating into a scratch directory and keeping the .ico
// is what stops the other forty files from coming back each time the artwork changes.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const master = join(root, "src-tauri", "icons", "app-icon.png");
const destination = join(root, "src-tauri", "icons", "icon.ico");
// The CLI's own entry point rather than the `tauri` shim, so this needs no shell and the
// arguments reach it exactly as written.
const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");

const scratch = mkdtempSync(join(tmpdir(), "quotastation-icons-"));
try {
  execFileSync(process.execPath, [cli, "icon", master, "--output", scratch], {
    stdio: "inherit",
  });
  copyFileSync(join(scratch, "icon.ico"), destination);
  console.log(`icon.ico regenerated from ${master}`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
