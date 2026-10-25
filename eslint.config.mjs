import js from '@eslint/js';
import globals from 'globals';
export default [
  { ignores: ['ui/repository-generated/**'] },
  { ...js.configs.recommended, files: ['ui/*.js'], languageOptions: { globals: globals.browser, ecmaVersion: 'latest', sourceType: 'module' }, rules: { ...js.configs.recommended.rules, 'no-unused-vars': ['error', { args: 'none', caughtErrors: 'none' }] } },
];
