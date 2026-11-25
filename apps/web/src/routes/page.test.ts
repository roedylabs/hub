import { render, screen } from '@testing-library/svelte';
import { describe, expect, test } from 'vitest';
import Page from './+page.svelte';

describe('home page', () => {
  test('names the hub', () => {
    render(Page);
    expect(screen.getByRole('heading', { level: 1, name: 'Roedy Hub' })).toBeInTheDocument();
  });
});
