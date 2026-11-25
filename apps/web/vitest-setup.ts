import '@testing-library/jest-dom/vitest';

// The tests check the English texts.
if (typeof navigator !== 'undefined') {
  Object.defineProperty(navigator, 'languages', { configurable: true, value: ['en-US'] });
  Object.defineProperty(navigator, 'language', { configurable: true, value: 'en-US' });
}
