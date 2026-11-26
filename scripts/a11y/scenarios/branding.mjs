// Administration › Branding (`/admin/branding`): default, initial, icon grid,
// a saved image with its file card, a rejected file at the drop zone, the bar
// "Unsaved changes" and a brand colour with a dark plate.

/** A small logo as a data URL (the audit answers `/branding/…` with 404). */
const LOGO =
  'data:image/svg+xml;base64,' +
  Buffer.from(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><circle cx="24" cy="24" r="18" fill="#2b221c"/></svg>',
  ).toString('base64');

export default ({ mock, button, BRANDING }) => {
  const admin = (over = {}) => ({ ...BRANDING, imageName: '', imageMeta: '', ...over });
  const at = (branding, extra = {}) =>
    mock({
      signedIn: true,
      routes: {
        'GET admin/branding': { body: branding },
        // Never answers: the bar stays busy ("Saving …").
        'PUT admin/branding': () => new Promise(() => {}),
        ...extra,
      },
    });
  const radio = (page, group, name) =>
    page.getByRole('radiogroup', { name: group }).getByRole('radio', { name, exact: true }).first().click();
  return [
    {
      id: 'admin branding',
      path: '/admin/branding',
      opts: at(admin()),
      // The page itself, not an error page with its own h1.
      prepare: (page) => page.getByRole('radiogroup', { name: 'Type' }).waitFor(),
    },
    {
      id: 'admin branding initial',
      path: '/admin/branding',
      opts: at(admin({ kind: 'initial', letter: 'EO', plate: 'dark', color: 'sage' })),
      prepare: (page) => page.getByRole('textbox', { name: 'Characters' }).waitFor(),
    },
    {
      id: 'admin branding icon',
      path: '/admin/branding',
      opts: at(admin({ kind: 'icon', icon: 'ti-rocket', plate: 'light', color: 'lavender', tilt: false })),
      prepare: (page) => page.getByRole('radiogroup', { name: 'Icon' }).waitFor(),
    },
    {
      id: 'admin branding image',
      path: '/admin/branding',
      opts: at(admin({ kind: 'image', image: LOGO, imageName: 'example.svg', imageMeta: 'SVG · 1 KB', fit: 'fill' })),
      prepare: (page) => page.getByText('example.svg').first().waitFor(),
    },
    {
      id: 'admin branding image error',
      path: '/admin/branding',
      opts: at(admin()),
      prepare: async (page) => {
        await radio(page, 'Type', 'Image');
        await page
          .locator('input[type=file]')
          .setInputFiles({ name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('x') });
        await page.getByRole('alert').first().waitFor();
      },
    },
    {
      id: 'admin branding saving',
      path: '/admin/branding',
      opts: at(admin()),
      prepare: async (page) => {
        await radio(page, 'Brand color', 'Rose');
        await page.getByRole('region', { name: 'Unsaved changes' }).waitFor();
        await button(page, 'Save');
        await page.locator('[aria-busy=true]').first().waitFor();
      },
    },
  ];
};
