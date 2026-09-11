import eslint from '@eslint/js';
import babelParser from '@babel/eslint-parser';
import reactHooks from 'eslint-plugin-react-hooks';
import globals from 'globals';

const typescriptParserOptions = {
  requireConfigFile: false,
  babelOptions: {
    presets: [
      ['@babel/preset-typescript', { ignoreExtensions: true }],
      ['@babel/preset-react', { runtime: 'automatic' }],
    ],
  },
};

export default [
  {
    ignores: [
      'dist/**',
      'coverage/**',
      'src-tauri/**',
      'src/bridge/generated/**',
      'node_modules/**',
      'playwright-report/**',
      'test-results/**',
    ],
  },
  {
    ...eslint.configs.recommended,
    files: ['**/*.{js,mjs,cjs,ts,tsx}'],
  },
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      parser: babelParser,
      parserOptions: typescriptParserOptions,
    },
    rules: {
      // Babel parses TypeScript syntax but cannot provide TypeScript's semantic
      // scope. The compiler owns these checks through the strict tsconfig.
      'no-undef': 'off',
      'no-unused-vars': 'off',
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    languageOptions: {
      globals: globals.browser,
    },
    ...reactHooks.configs.flat.recommended,
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    ignores: ['src/bridge/**', 'src/platform/**'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['@tauri-apps/*'],
              message: 'Import Tauri APIs only from src/bridge or src/platform adapters.',
            },
          ],
        },
      ],
    },
  },
  {
    files: ['**/*.{js,mjs,cjs}', 'vite.config.ts', 'vitest.config.ts', 'playwright.config.ts'],
    languageOptions: {
      globals: globals.node,
    },
  },
  {
    files: ['tests/e2e/**/*.{ts,tsx}'],
    languageOptions: {
      globals: { ...globals.node, ...globals.browser },
    },
  },
];
