import { fireEvent, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { brandingOf, type AdminBranding } from '#lib/branding.svelte.ts';
import { BRAND0, session } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { type ApiRoutes, renderSignedIn, resetSignedIn, setPhone } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));
const sections = vi.hoisted(() => ({ scrollToSection: vi.fn() }));
vi.mock('#lib/sections.ts', () => sections);

/** State from `GET admin/branding`: the default. */
const ADMIN: AdminBranding = { ...BRAND0, imageName: '', imageMeta: '' };
const IMAGE = '/branding/image/0123456789abcdef0123456789abcdef.png';

beforeEach(() => {
  // jsdom loads no images: pixel size for the check on upload.
  vi.stubGlobal(
    'Image',
    class {
      naturalWidth = 512;
      naturalHeight = 512;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      set src(_: string) {
        queueMicrotask(() => this.onload?.());
      }
    },
  );
});

afterEach(() => {
  resetSignedIn();
  sections.scrollToSection.mockClear();
});

/** Page with the state from `admin/branding`; waits for the response. */
async function renderPage(branding: AdminBranding = ADMIN, routes: ApiRoutes = {}) {
  const view = renderSignedIn(Page, {
    role: 'admin',
    path: '/admin/branding',
    config: { branding: brandingOf(branding) },
    api: { 'GET admin/branding': [{ body: branding }], ...routes },
  });
  await vi.waitFor(() => expect(view.calls).toHaveLength(1));
  await Promise.resolve();
  return view;
}

const field = (label: string) => screen.getByRole('textbox', { name: label });
const sw = (label: string) => screen.getByRole('switch', { name: label });
const radios = (group: string) => within(screen.getByRole('radiogroup', { name: group })).getAllByRole('radio');
const radio = (group: string, name: string) =>
  within(screen.getByRole('radiogroup', { name: group })).getByRole('radio', { name });
const stage = (label: string) => screen.getByText(label).closest('figure') as HTMLElement;
const bar = () => screen.queryByRole('region', { name: 'Unsaved changes' });
const fileInput = (container: HTMLElement) => container.querySelector('input[type="file"]') as HTMLInputElement;
const png = (name = 'logo.png', type = 'image/png') => new File([new Uint8Array(2048)], name, { type });
const puts = (calls: { method: string; path: string; body: unknown }[]) =>
  calls.filter((c) => c.method === 'PUT').map((c) => [c.path, c.body]);

describe('Branding page', () => {
  it('shows heading, tabs, preview and sections with the saved state', async () => {
    const view = await renderPage();
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Branding');
    for (const tab of ['Logo', 'Brand color']) {
      expect(view.getByRole('button', { name: tab })).toBeInTheDocument();
      expect(view.getByRole('heading', { level: 2, name: tab })).toBeInTheDocument();
    }
    // No app name and no sign-in page: the hub's name is the organization's.
    expect(view.queryByRole('heading', { level: 2, name: 'Name' })).toBeNull();
    expect(view.queryByRole('heading', { level: 2, name: 'Sign-in page' })).toBeNull();
    expect(view.queryAllByRole('textbox')).toHaveLength(0);
    expect(view.getByText('Default accent color for all users.')).toBeInTheDocument();

    // Preview: header with rotated tile in the accent colour and the hub's mark, app icon.
    const header = stage('Header');
    expect(within(header).getByText('Example Org')).toBeInTheDocument();
    const tile = header.querySelector('.brand-tile') as HTMLElement;
    expect(tile).toHaveStyle({ backgroundColor: '#f5a06e', color: '#3a1b06', transform: 'rotate(-4deg)' });
    expect(tile.querySelectorAll('svg.mark rect')).toHaveLength(4);
    expect(within(stage('App icon')).getByText('Example Org')).toBeInTheDocument();

    // Logo: kinds, plate, switch; default without an extra row.
    expect(radios('Type').map((r) => r.textContent)).toEqual(['Default', 'Initials', 'Icon', 'Image']);
    expect(radio('Type', 'Default')).toHaveAttribute('aria-checked', 'true');
    expect(radios('Tile').map((r) => r.textContent)).toEqual(['Accent color', 'Light', 'Dark']);
    expect(radio('Tile', 'Accent color')).toHaveAttribute('aria-checked', 'true');
    expect(view.queryByRole('radiogroup', { name: 'Size' })).toBeNull();
    expect(sw('Slightly tilted')).toHaveAttribute('aria-checked', 'true');
    expect(sw('Allow custom accent color')).toHaveAttribute('aria-checked', 'true');

    // Brand colour: five colours, apricot selected.
    expect(radios('Brand color').map((r) => r.textContent?.trim())).toEqual([
      'Apricot',
      'Sage',
      'Lavender',
      'Sky',
      'Rose',
    ]);
    expect(radio('Brand color', 'Apricot')).toHaveAttribute('aria-checked', 'true');

    // The default: neither "Restore defaults" nor the bar.
    expect(view.queryByRole('button', { name: 'Restore defaults' })).toBeNull();
    expect(bar()).toBeNull();
  });

  it('shows the saved logo file with name and details', async () => {
    const view = await renderPage({
      ...ADMIN,
      kind: 'image',
      image: IMAGE,
      imageName: 'example.png',
      imageMeta: 'PNG · 3 KB',
    });
    expect(radio('Type', 'Image')).toHaveAttribute('aria-checked', 'true');
    expect(await view.findByText('example.png')).toBeInTheDocument();
    expect(view.getByText('PNG · 3 KB')).toBeInTheDocument();
    expect(stage('Header').querySelector('img')).toHaveAttribute('src', IMAGE);
    expect(radio('Size', 'Padded')).toHaveAttribute('aria-checked', 'true');
  });

  it('switches the kind: initial with character, icon with grid', async () => {
    const view = await renderPage();
    await fireEvent.click(radio('Type', 'Initials'));
    expect(radio('Type', 'Initials')).toHaveAttribute('aria-checked', 'true');
    // No native maxlength: it counts UTF-16 units and would block `A😀`; the model limits code points.
    expect(field('Characters')).not.toHaveAttribute('maxlength');
    expect(field('Characters')).toHaveAttribute('placeholder', 'E');
    expect(
      view.getByText('One or two characters. Leave empty to use the first letter of the organization’s name.'),
    ).toBeInTheDocument();
    expect(within(stage('Header')).getByText('E')).toBeInTheDocument();
    await fireEvent.input(field('Characters'), { target: { value: 'XY' } });
    expect(within(stage('Header')).getByText('XY')).toBeInTheDocument();
    expect(bar()).not.toBeNull();
    // Two code points of which one is an emoji; a third character is cut off, also in the field.
    await fireEvent.input(field('Characters'), { target: { value: 'A😀' } });
    expect(field('Characters')).toHaveValue('A😀');
    expect(within(stage('Header')).getByText('A😀')).toBeInTheDocument();
    await fireEvent.input(field('Characters'), { target: { value: 'A😀B' } });
    expect(field('Characters')).toHaveValue('A😀');
    expect(within(stage('Header')).getByText('A😀')).toBeInTheDocument();

    await fireEvent.click(radio('Type', 'Icon'));
    expect(view.queryByRole('textbox', { name: 'Characters' })).toBeNull();
    expect(view.getByText('Shown in the text color on the tile')).toBeInTheDocument();
    expect(radios('Icon').map((r) => r.getAttribute('aria-label'))).toEqual([
      'Mountain',
      'Leaf',
      'Bolt',
      'Flame',
      'Star',
      'Heart',
      'Cloud',
      'Compass',
      'Diamond',
      'Rocket',
      'Feather',
      'Planet',
    ]);
    expect(radio('Icon', 'Mountain')).toHaveAttribute('aria-checked', 'true');
    await fireEvent.click(radio('Icon', 'Rocket'));
    expect(radio('Icon', 'Rocket')).toHaveAttribute('aria-checked', 'true');
    expect(radio('Icon', 'Mountain')).toHaveAttribute('aria-checked', 'false');

    // Plate, colour and switch change the preview.
    await fireEvent.click(radio('Tile', 'Dark'));
    const tile = stage('Header').querySelector('.brand-tile') as HTMLElement;
    expect(tile).toHaveStyle({ backgroundColor: '#2b221c', color: '#fffaf3' });
    await fireEvent.click(sw('Slightly tilted'));
    expect(sw('Slightly tilted')).toHaveAttribute('aria-checked', 'false');
    expect(tile).toHaveStyle({ transform: 'none' });
    await fireEvent.click(radio('Brand color', 'Sage'));
    expect(radio('Brand color', 'Sage')).toHaveAttribute('aria-checked', 'true');
    expect(radio('Brand color', 'Apricot')).toHaveAttribute('aria-checked', 'false');

    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Discard' }));
    expect(bar()).toBeNull();
    expect(radio('Type', 'Default')).toHaveAttribute('aria-checked', 'true');
    expect(radio('Brand color', 'Apricot')).toHaveAttribute('aria-checked', 'true');
    expect(sw('Slightly tilted')).toHaveAttribute('aria-checked', 'true');
  });

  it('picks an image, shows name and details, replaces and removes it', async () => {
    const view = await renderPage();
    await fireEvent.click(radio('Type', 'Image'));
    expect(view.getByText('Square, at least 256 × 256 pixels')).toBeInTheDocument();
    const drop = view.getByRole('button', { name: /Drag an image here or choose one/ });
    expect(drop).toHaveTextContent('PNG, SVG, JPG, or WebP, up to 1 MB');
    expect(radios('Size').map((r) => r.textContent)).toEqual(['Padded', 'Fill']);

    // The drop zone opens the hidden file field.
    const input = fileInput(view.container);
    expect(input).toHaveAttribute('accept', 'image/png,image/jpeg,image/webp,image/svg+xml');
    const click = vi.spyOn(input, 'click');
    await fireEvent.click(drop);
    expect(click).toHaveBeenCalledTimes(1);

    drop.focus();
    await fireEvent.change(input, { target: { files: [png()] } });
    await vi.waitFor(() => expect(view.getByText('logo.png')).toBeInTheDocument());
    expect(view.getByText('PNG · 2 KB')).toBeInTheDocument();
    expect(input.value).toBe('');
    expect(view.queryByRole('button', { name: /Drag an image here/ })).toBeNull();
    // The drop zone had the focus and is gone: the focus goes to Replace.
    expect(view.getByRole('button', { name: 'Replace' })).toHaveFocus();
    expect(stage('Header').querySelector('img')).toHaveAttribute('src', expect.stringMatching(/^data:image\/png/));
    expect(bar()).not.toBeNull();

    await fireEvent.click(view.getByRole('button', { name: 'Replace' }));
    expect(click).toHaveBeenCalledTimes(2);
    view.getByRole('button', { name: 'Remove image' }).focus();
    await fireEvent.click(view.getByRole('button', { name: 'Remove image' }));
    expect(view.queryByText('logo.png')).toBeNull();
    // Remove had the focus and is gone: the focus goes to the drop zone.
    expect(view.getByRole('button', { name: /Drag an image here/ })).toHaveFocus();

    // A focus moved elsewhere stays when the image arrives.
    radio('Type', 'Image').focus();
    await fireEvent.change(input, { target: { files: [png()] } });
    await vi.waitFor(() => expect(view.getByText('logo.png')).toBeInTheDocument());
    expect(radio('Type', 'Image')).toHaveFocus();
    await fireEvent.click(view.getByRole('button', { name: 'Remove image' }));

    // Without an image "Save" asks for one and leads to the logo.
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    expect(ui.toast?.text).toBe('Please upload an image');
    expect(sections.scrollToSection.mock.calls[0]?.[1]).toBe('logo');
    expect(puts(view.calls)).toEqual([]);
  });

  it('accepts an image by dropping and reports a wrong format at the drop zone', async () => {
    const view = await renderPage();
    await fireEvent.click(radio('Type', 'Image'));
    const drop = view.getByRole('button', { name: /Drag an image here/ });

    expect(await fireEvent.dragOver(drop, { dataTransfer: { files: [] } })).toBe(false);
    expect(drop).toHaveClass('drag');
    await fireEvent.dragLeave(drop);
    expect(drop).not.toHaveClass('drag');

    await fireEvent.drop(drop, { dataTransfer: { files: [png('notes.txt', 'text/plain')] } });
    expect(view.getByRole('alert')).toHaveTextContent('This format isn’t supported. Use PNG, SVG, JPG, or WebP.');
    expect(drop).not.toHaveClass('drag');

    await fireEvent.drop(drop, { dataTransfer: { files: [png('mark.webp', 'image/webp')] } });
    await vi.waitFor(() => expect(view.getByText('mark.webp')).toBeInTheDocument());
    expect(view.getByText('WEBP · 2 KB')).toBeInTheDocument();
    expect(view.queryByRole('alert')).toBeNull();

    // Switching the kind removes the error.
    await fireEvent.drop(view.getByRole('group', { name: 'Image' }), {
      dataTransfer: { files: [png('big.gif', 'image/gif')] },
    });
    expect(view.getByRole('alert')).toBeInTheDocument();
    await fireEvent.click(radio('Type', 'Default'));
    await fireEvent.click(radio('Type', 'Image'));
    expect(view.queryByRole('alert')).toBeNull();

    // Text in the SVG shows the reason at the same place.
    const svg = new File(['<svg><text>Example</text></svg>'], 'mark.svg', { type: 'image/svg+xml' });
    await fireEvent.drop(view.getByRole('group', { name: 'Image' }), { dataTransfer: { files: [svg] } });
    await vi.waitFor(() =>
      expect(view.getByRole('alert')).toHaveTextContent('Text in SVG isn’t supported. Convert the text to paths.'),
    );
  });

  it('saves the changed fields with a busy bar and updates the public branding', async () => {
    let release: () => void = () => {};
    const saved: AdminBranding = { ...ADMIN, kind: 'initial', letter: 'N', color: 'sky' };
    const view = await renderPage(ADMIN, {
      'PUT admin/branding': [
        () =>
          new Promise((resolve) => {
            release = () => resolve({ body: saved });
          }),
      ],
    });
    await fireEvent.click(radio('Type', 'Initials'));
    await fireEvent.input(field('Characters'), { target: { value: ' N' } });
    await fireEvent.click(radio('Brand color', 'Sky'));
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    const save = within(bar() as HTMLElement).getByRole('button', { name: /Saving/ });
    expect(save).toHaveAttribute('aria-busy', 'true');
    expect(puts(view.calls)).toEqual([['admin/branding', { kind: 'initial', letter: 'N', color: 'sky' }]]);

    release();
    await vi.waitFor(() => expect(bar()).toBeNull());
    expect(ui.toast?.text).toBe('Changes saved');
    expect(session.config?.branding).toMatchObject({ kind: 'initial', letter: 'N', color: 'sky' });
    expect(session.logo).toMatchObject({ isInit: true, text: 'N' });
    expect(field('Characters')).toHaveValue('N');
    expect(view.getByRole('button', { name: 'Restore defaults' })).toBeInTheDocument();
  });

  it('shows a logo error of the server at the image', async () => {
    const view = await renderPage(ADMIN, {
      'PUT admin/branding': [{ status: 422, body: { error: 'invalid', fields: { image: { code: 'dimensions' } } } }],
    });
    await fireEvent.click(radio('Type', 'Image'));
    await fireEvent.change(fileInput(view.container), { target: { files: [png()] } });
    await vi.waitFor(() => expect(view.getByText('logo.png')).toBeInTheDocument());
    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Save' }));
    await vi.waitFor(() =>
      expect(view.getByRole('alert')).toHaveTextContent('The image is too large. Use no more than 4096 × 4096 pixels.'),
    );
    expect(sections.scrollToSection.mock.calls.at(-1)?.[1]).toBe('logo');
    expect(bar()).not.toBeNull();
    expect(session.branding.image).toBeNull();
  });

  it('restores the default as a draft', async () => {
    const custom: AdminBranding = { ...ADMIN, kind: 'icon', icon: 'ti-rocket', color: 'rose', userAccent: false };
    const view = await renderPage(custom);
    expect(radio('Icon', 'Rocket')).toHaveAttribute('aria-checked', 'true');
    expect(sw('Allow custom accent color')).toHaveAttribute('aria-checked', 'false');

    await fireEvent.click(view.getByRole('button', { name: 'Restore defaults' }));
    expect(view.queryByRole('button', { name: 'Restore defaults' })).toBeNull();
    expect(radio('Type', 'Default')).toHaveAttribute('aria-checked', 'true');
    expect(radio('Brand color', 'Apricot')).toHaveAttribute('aria-checked', 'true');
    expect(sw('Allow custom accent color')).toHaveAttribute('aria-checked', 'true');
    expect(bar()).not.toBeNull();

    await fireEvent.click(within(bar() as HTMLElement).getByRole('button', { name: 'Discard' }));
    expect(radio('Type', 'Icon')).toHaveAttribute('aria-checked', 'true');
    expect(view.getByRole('button', { name: 'Restore defaults' })).toBeInTheDocument();
  });

  it('scrolls to the section via the tabs', async () => {
    const view = await renderPage();
    await fireEvent.click(view.getByRole('button', { name: 'Brand color' }));
    expect(sections.scrollToSection).toHaveBeenCalledTimes(1);
    const [root, key] = sections.scrollToSection.mock.calls[0] as [HTMLElement, string, boolean];
    expect(key).toBe('color');
    expect(root.querySelector('[data-sec="color"]')).toHaveTextContent('Brand color');
    expect([...root.querySelectorAll('section')].map((s) => s.dataset.sec)).toEqual(['logo', 'color']);
  });

  it('shows only the header, shorter texts and icon buttons at the image on mobile', async () => {
    setPhone(true);
    const view = await renderPage();
    expect(ui.isMobile).toBe(true);
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Branding');
    expect(stage('Header')).toBeInTheDocument();
    expect(view.queryByText('App icon')).toBeNull();
    expect(view.queryByText('What appears on the logo tile')).toBeNull();
    expect(view.getByText('Show the logo tile at an angle')).toBeInTheDocument();

    await fireEvent.click(radio('Type', 'Initials'));
    expect(view.getByText('One or two characters')).toBeInTheDocument();
    await fireEvent.click(radio('Type', 'Image'));
    expect(view.getByRole('button', { name: /Choose image/ })).toBeInTheDocument();
    await fireEvent.change(fileInput(view.container), { target: { files: [png()] } });
    await vi.waitFor(() => expect(view.getByText('logo.png')).toBeInTheDocument());
    expect(view.getByRole('button', { name: 'Replace image' })).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Replace' })).toBeNull();

    const region = bar() as HTMLElement;
    expect(within(region).queryByText('Unsaved changes')).toBeNull();
    expect(
      within(region)
        .getAllByRole('button')
        .map((b) => b.textContent),
    ).toEqual(['Discard', 'Save']);
    await fireEvent.click(view.getByRole('button', { name: 'Remove image' }));
    expect(view.getByRole('button', { name: /Choose image/ })).toBeInTheDocument();
  });
});
