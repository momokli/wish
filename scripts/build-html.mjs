#!/usr/bin/env node
// Build script: inlines external CSS and JS into HTML files.
// Reads skeleton HTMLs from frontend/, resolves <link> and <script src>,
// inlines file contents, writes output to frontend/dist/.
// Usage: node scripts/build-html.mjs

import { readFileSync, writeFileSync, mkdirSync, existsSync } from "fs";
import { join, dirname, relative } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const frontend = join(root, "frontend");
const dist = join(frontend, "dist");

// Input HTML files (relative to frontend/)
const pages = ["index.html", "admin.html"];

// Ensure dist exists
if (!existsSync(dist)) mkdirSync(dist, { recursive: true });

for (const page of pages) {
  const htmlPath = join(frontend, page);
  if (!existsSync(htmlPath)) {
    console.error(`✗ ${page} not found, skipping`);
    continue;
  }

  let html = readFileSync(htmlPath, "utf8");

  // Replace <link rel="stylesheet" href="..."> with <style> + file contents
  html = html.replace(
    /<link\s+rel=["']stylesheet["']\s+href=["']([^"']+)["']\s*\/?>/gi,
    (match, href) => {
      const cssPath = join(frontend, href);
      if (!existsSync(cssPath)) {
        console.error(`  ⚠ CSS not found: ${href} (from ${page})`);
        return match;
      }
      const css = readFileSync(cssPath, "utf8");
      console.log(`  ✓ inlined CSS: ${href}`);
      return `<style>\n${css}\n</style>`;
    }
  );

  // Replace <script src="..."></script> with <script> + file contents
  html = html.replace(
    /<script\s+src=["']([^"']+)["']\s*>\s*<\/script>/gi,
    (match, src) => {
      const jsPath = join(frontend, src);
      if (!existsSync(jsPath)) {
        console.error(`  ⚠ JS not found: ${src} (from ${page})`);
        return match;
      }
      const js = readFileSync(jsPath, "utf8");
      console.log(`  ✓ inlined JS: ${src}`);
      return `<script>\n${js}\n</script>`;
    }
  );

  const outPath = join(dist, page);
  writeFileSync(outPath, html, "utf8");
  console.log(`✓ ${page} → dist/${page}`);
}

console.log("\nBuild complete.");
