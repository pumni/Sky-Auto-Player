import astro from 'eslint-plugin-astro';

export default [
  ...astro.configs['flat/recommended'],
  {
    ignores: ['dist/**', '.astro/**', 'node_modules/**', 'playwright-report/**', 'test-results/**'],
  },
];
