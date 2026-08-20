import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";

/// Typed ESLint + React's official hooks preset. This is the stock
/// correctness stack, not a pile of style plugins:
/// - `strictTypeChecked` is typescript-eslint's "catch more bugs" config
///   (floating promises, unsafe `any`, always-true conditions, …)
/// - `switch-exhaustiveness-check` fails when a union grows a variant
/// - `recommended-latest` is the React Compiler hooks plugin (setState in
///   effect/render, immutability, purity)
export default tseslint.config(
  {
    ignores: ["dist", "node_modules", "src-tauri", "eslint.config.js", "vite.config.ts", "src/app/api/schema.ts"],
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [
      js.configs.recommended,
      ...tseslint.configs.strictTypeChecked,
      ...tseslint.configs.stylisticTypeChecked,
      reactHooks.configs.flat["recommended-latest"],
    ],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      "react-refresh": reactRefresh,
    },
    rules: {
      "react-hooks/exhaustive-deps": "error",
      "react-refresh/only-export-components": ["error", { allowConstantExport: true }],
      "@typescript-eslint/switch-exhaustiveness-check": [
        "error",
        { considerDefaultExhaustiveForUnions: false },
      ],
      // Numbers in templates (`Convert ${n} files`) are the UI, not a bug.
      "@typescript-eslint/restrict-template-expressions": [
        "error",
        { allowNumber: true, allowBoolean: true },
      ],
      // Keep the rule: it still catches `const x = setState()`. Cleanup
      // `() => void un.then(...)` is the Tauri unsubscribe idiom.
      "@typescript-eslint/no-confusing-void-expression": [
        "error",
        { ignoreArrowShorthand: true },
      ],
      // The generated OpenAPI schema has one door: the `@/app/api` module.
      // Everything outside it uses the client or the re-exported schema types.
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: [
                "@/app/api/schema",
                "./api/schema",
                "../api/schema",
                "../app/api/schema",
                "../../app/api/schema",
              ],
              message:
                "Import the conversion client or schema types from @/app/api, never the generated schema directly.",
            },
          ],
        },
      ],
    },
  },
  // Inside the API layer the rule above is lifted: that is the schema's door.
  {
    files: ["src/app/api/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": "off",
    },
  },
  // The design-system boundary, enforced rather than documented. `src/ui` may
  // depend on React and its own files, and on nothing else in the repo. Break
  // this and the library stops being liftable into `packages/ui`, which is the
  // whole reason it is a separate tree instead of a folder of components.
  {
    files: ["src/ui/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              // `**/` rather than a `../` and `../../` pair per directory.
              // These match the import specifier's text, not the file's place
              // on disk, so depth literals stop matching the moment the tree
              // nests one level deeper and the rule then passes while
              // enforcing nothing. `**/` absorbs any number of leading `..`
              // segments, so the boundary survives the next move.
              group: [
                "@/*",
                "@tauri-apps/*",
                "**/app/*",
                "**/domains/*",
                "**/platform/*",
                "**/shell/*",
              ],
              message:
                "src/ui must not depend on the app. Take what you need as a prop, or add a token.",
            },
          ],
        },
      ],
    },
  }
);
