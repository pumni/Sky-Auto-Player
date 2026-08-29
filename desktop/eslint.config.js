import eslint from '@eslint/js';

// TypeScript 7 is the locked frontend baseline. typescript-eslint currently
// rejects TS 7, so tsc is the authoritative TypeScript gate; ESLint keeps
// validating the executable config surface and ignores TS source explicitly.
export default [
  eslint.configs.recommended,
  {
    ignores: [
      '**/*.ts',
      '**/*.tsx',
      'dist/**',
      'src-tauri/**',
      'node_modules/**',
      'playwright-report/**',
      'test-results/**',
    ],
  },
  {
    languageOptions: {
      globals: {
        URL: 'readonly',
        fetch: 'readonly',
      },
    },
  },
];
