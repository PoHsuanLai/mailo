// The relay's npm licences, held to an allow-list as deny.toml holds the Rust crates': every
// package in package-lock.json, development tools included, under a licence named here, and none
// under any GPL. Run by CI (`npm run licences`); prints each package that fails and exits 1.

import { readFileSync } from "node:fs";

const ALLOWED = new Set([
  "MIT",
  "Apache-2.0",
  "ISC",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "0BSD",
  "CC0-1.0",
  "MPL-2.0",
  "BlueOak-1.0.0",
  "Python-2.0",
]);

/// Whether an SPDX expression can be taken: any one of its `OR` choices, with every part of
/// that choice's `AND` on the list. Nothing that names a GPL is, whatever it is offered beside.
function allowed(license) {
  if (typeof license !== "string" || /GPL/i.test(license)) {
    return false;
  }
  return license
    .replace(/[()]/g, "")
    .split(/\s+OR\s+/)
    .some((choice) => choice.split(/\s+AND\s+/).every((part) => ALLOWED.has(part.trim())));
}

const lock = JSON.parse(readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"));
const refused = [];
for (const [path, entry] of Object.entries(lock.packages ?? {})) {
  // The relay itself, and a link to a package that has an entry of its own (the sharp stub).
  if (path === "" || entry.link === true) {
    continue;
  }
  if (!allowed(entry.license)) {
    refused.push(`${path} ${entry.version ?? ""}: ${entry.license ?? "no licence given"}`);
  }
}
if (refused.length > 0) {
  console.error("Licences not on the relay's allow-list (relay/scripts/licences.mjs):");
  for (const line of refused) {
    console.error(`  ${line}`);
  }
  process.exit(1);
}
console.log(`${Object.keys(lock.packages ?? {}).length - 1} packages, every licence allowed`);
