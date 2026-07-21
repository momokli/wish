#!/usr/bin/env node
// Validate frontend HTML files have valid JavaScript syntax.
// Handles one or more <script> blocks per file.
// Usage: node scripts/lint-html.mjs frontend/dist/*.html

import { readFileSync, writeFileSync, mkdtempSync } from "fs";
import { execSync } from "child_process";
import { tmpdir } from "os";
import { join } from "path";

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("Usage: node scripts/lint-html.mjs frontend/dist/*.html");
  process.exit(1);
}

let hadError = false;

for (const f of files) {
  let html;
  try {
    html = readFileSync(f, "utf8");
  } catch {
    console.error(`✗ ${f} (not found)`);
    hadError = true;
    continue;
  }

  // Collect ALL <script>...</script> blocks
  const scriptRegex = /<script>([\s\S]*?)<\/script>/g;
  const scripts = [];
  let match;
  while ((match = scriptRegex.exec(html)) !== null) {
    scripts.push(match[1]);
  }

  if (scripts.length === 0) {
    console.error(`✗ ${f} (no <script> found)`);
    hadError = true;
    continue;
  }

  // Concatenate all scripts for a single syntax check
  const combined = scripts.join("\n;\n");

  const tmpFile = join(mkdtempSync(join(tmpdir(), "wish-lint-")), "check.js");
  writeFileSync(tmpFile, combined);

  try {
    execSync(`node --check "${tmpFile}"`, { stdio: "pipe" });
    console.log(`✓ ${f}`);
  } catch (e) {
    const msg = e.stderr.toString().replace(tmpFile, f);
    const line =
      msg.split("\n").find((l) => l.includes("SyntaxError")) || msg.trim().split("\n")[0];
    console.error(`✗ ${f}: ${line}`);
    hadError = true;
  }
}

process.exit(hadError ? 1 : 0);
