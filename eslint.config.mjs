// ESLint config for wish frontend JS
// Only checks for undefined variables, syntax, and basic errors.
// Zero opinionated rules — just catches bugs like missing functions.

import js from "@eslint/js";

export default [
  js.configs.recommended,
  {
    languageOptions: {
      globals: {
        window: "readonly",
        document: "readonly",
        console: "readonly",
        localStorage: "readonly",
        fetch: "readonly",
        AbortController: "readonly",
        setTimeout: "readonly",
        clearTimeout: "readonly",
        setInterval: "readonly",
        requestAnimationFrame: "readonly",
        encodeURIComponent: "readonly",
        decodeURIComponent: "readonly",
        confirm: "readonly",
      }
    },
    rules: {
      "no-unused-vars": "off",
      "no-undef": "error",
      "no-redeclare": "error",
      "no-dupe-keys": "error",
    }
  }
];
