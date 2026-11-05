// Fails when package.json, Cargo.toml and tauri.conf.json disagree on the version.
import { readFileSync } from "node:fs";
const pkg = JSON.parse(readFileSync("package.json", "utf8")).version;
const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const cargo = /^version\s*=\s*"([^"]+)"/m.exec(readFileSync("src-tauri/Cargo.toml", "utf8"))?.[1];
if (pkg !== conf || pkg !== cargo) {
  console.error(`Version mismatch: package.json=${pkg} tauri.conf.json=${conf} Cargo.toml=${cargo}`);
  process.exit(1);
}
console.log(`Versions agree: ${pkg}`);
