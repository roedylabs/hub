import { capabilityOf, CONTACTS, type ArgsOf, type ResponseOf } from '@roedy/jmap';
import { connectSettings } from '@roedy/ui';
import { render, screen } from '@testing-library/svelte';
import { expect, test } from 'vitest';
import { MAIL, type Mailbox } from '#lib/mail.ts';
import Page from './routes/+page.svelte';

test('the published components render with their German texts', () => {
  // As the layout does (its style sheets are left to `vite build`).
  connectSettings({ lang: () => 'de', mode: () => 'dark' });
  render(Page);
  expect(screen.getByRole('heading', { name: 'Seite nicht gefunden' })).toBeTruthy();
  expect(screen.getByLabelText('Bug')).toBeTruthy();
});

test('an app extends Methods and the capabilities', () => {
  const args: ArgsOf<'Mailbox/get'> = { accountId: 'a', ids: null };
  const list: ResponseOf<'Mailbox/get'>['list'] = [{ id: 'm', name: 'Inbox' } satisfies Mailbox];
  expect(args.accountId).toBe('a');
  expect(list).toHaveLength(1);
  expect(capabilityOf('Mailbox/get')).toBe(MAIL);
  expect(capabilityOf('ContactCard/get')).toBe(CONTACTS);
});
