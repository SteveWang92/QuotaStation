// Regenerates the Windows application icon from the master artwork.
//
// `tauri icon` writes every icon every platform could want — Android densities, iOS sizes,
// an .icns, a dozen Windows Store tiles — and QuotaStation is a Windows desktop application
// that bundles exactly one of them. Generating into a scratch directory and keeping the .ico
// is what stops the other forty files from coming back each time the artwork changes.

import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const master = join(root, "src-tauri", "icons", "app-icon.png");
const smallMaster = join(root, "src-tauri", "icons", "app-icon-small.svg");
const destination = join(root, "src-tauri", "icons", "icon.ico");
const resizeScript = join(root, "scripts", "resize-icon.ps1");
// The CLI's own entry point rather than the `tauri` shim, so this needs no shell and the
// arguments reach it exactly as written.
const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");

function readIco(path) {
  const bytes = readFileSync(path);
  const count = bytes.readUInt16LE(4);
  return Array.from({ length: count }, (_, index) => {
    const entryOffset = 6 + index * 16;
    const width = bytes[entryOffset] || 256;
    const height = bytes[entryOffset + 1] || 256;
    const byteLength = bytes.readUInt32LE(entryOffset + 8);
    const imageOffset = bytes.readUInt32LE(entryOffset + 12);
    return {
      width,
      height,
      colorCount: bytes[entryOffset + 2],
      planes: bytes.readUInt16LE(entryOffset + 4),
      bitCount: bytes.readUInt16LE(entryOffset + 6),
      image: bytes.subarray(imageOffset, imageOffset + byteLength),
    };
  });
}

function writeIco(path, entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);

  const directory = Buffer.alloc(entries.length * 16);
  let imageOffset = header.length + directory.length;
  entries.forEach((entry, index) => {
    const entryOffset = index * 16;
    directory[entryOffset] = entry.width === 256 ? 0 : entry.width;
    directory[entryOffset + 1] = entry.height === 256 ? 0 : entry.height;
    directory[entryOffset + 2] = entry.colorCount;
    directory.writeUInt16LE(entry.planes, entryOffset + 4);
    directory.writeUInt16LE(entry.bitCount, entryOffset + 6);
    directory.writeUInt32LE(entry.image.length, entryOffset + 8);
    directory.writeUInt32LE(imageOffset, entryOffset + 12);
    imageOffset += entry.image.length;
  });

  writeFileSync(path, Buffer.concat([header, directory, ...entries.map((entry) => entry.image)]));
}

const scratch = mkdtempSync(join(tmpdir(), "quotastation-icons-"));
try {
  const fullOutput = join(scratch, "full");
  const smallOutput = join(scratch, "small");
  const sizeOutput = join(scratch, "sizes");
  mkdirSync(fullOutput);
  mkdirSync(smallOutput);
  execFileSync(process.execPath, [cli, "icon", master, "--output", fullOutput], {
    stdio: "inherit",
  });
  execFileSync(process.execPath, [cli, "icon", smallMaster, "--output", smallOutput], {
    stdio: "inherit",
  });

  const fullEntries = readIco(join(fullOutput, "icon.ico"));
  const smallSizes = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96];
  execFileSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      resizeScript,
      "-Source",
      join(smallOutput, "128x128@2x.png"),
      "-Output",
      sizeOutput,
      "-Sizes",
      smallSizes.join(","),
    ],
    { stdio: "inherit" },
  );

  const entries = smallSizes.map((size) => ({
    width: size,
    height: size,
    colorCount: 0,
    planes: 1,
    bitCount: 32,
    image: readFileSync(join(sizeOutput, `${size}.png`)),
  }));
  const fullEntry = fullEntries.find((candidate) => candidate.width === 256 && candidate.height === 256);
  if (!fullEntry) {
    throw new Error("generated full icon.ico is missing its 256x256 entry");
  }
  entries.push(fullEntry);
  writeIco(destination, entries);
  console.log(`icon.ico regenerated from ${smallMaster} (16-96px) and ${master} (256px)`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
