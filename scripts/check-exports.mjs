#!/usr/bin/env node
// Check that all functions exported in window._w actually exist.
// Catches the exact bug that just happened (formatLastSynced referenced but undefined).
import { readFileSync } from 'fs';

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error('Usage: node scripts/check-exports.mjs frontend/js/*.js');
  process.exit(1);
}

let hadError = false;

for (const f of files) {
  const src = readFileSync(f, 'utf8');

  // Find window._w = { ... } and extract all property names
  const wMatch = src.match(/window\._w\s*=\s*\{([^}]+)\}/);
  if (!wMatch) continue;

  const exports = wMatch[1]
    .split(',')
    .map(s => s.trim())
    .filter(s => /^\w+:/.test(s))
    .map(s => s.split(':')[0].trim());

  // Check each exported name has a definition
  for (const name of exports) {
    // Look for function declarations or var assignments
    const hasFunc = new RegExp(`function\\s+${name}\\s*\\(`).test(src);
    const hasVar = new RegExp(`(?:var|let|const)\\s+${name}\\s*=`).test(src);

    if (!hasFunc && !hasVar) {
      console.error(`✗ ${f}: window._w.${name} has no definition`);
      hadError = true;
    }
  }
}

if (hadError) process.exit(1);
console.log('✓ All window._w exports have definitions');
