import eslint from '@eslint/js';
import babelParser from '@babel/eslint-parser';
import reactHooks from 'eslint-plugin-react-hooks';

export default [
  {
    ...eslint.configs.recommended,
    files: ['**/*.{js,mjs,cjs}'],
  },
  {
    ignores: [
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
  {
    files: ['**/*.ts', '**/*.tsx'],
    languageOptions: {
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          presets: [
            ['@babel/preset-typescript', { ignoreExtensions: true }],
            ['@babel/preset-react', { runtime: 'automatic' }],
          ],
        },
      },
    },
    ...reactHooks.configs.flat.recommended,
  },
];
