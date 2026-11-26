// Account › Appearance: defaults, a custom accent, the accent locked by the branding and the colour editor open.
export default ({ mock, CONFIG, BRANDING }) => {
  const settings = (patch) => ({
    'GET settings': { body: { version: 1, settings: { lang: 'en', mode: null, accent: null, ...patch } } },
  });
  return [
    { id: 'appearance', path: '/account/appearance', opts: mock({ signedIn: true }) },
    {
      id: 'appearance custom accent',
      path: '/account/appearance',
      opts: mock({ signedIn: true, routes: settings({ accent: '#3a7bd5' }) }),
      prepare: (page) => page.getByRole('radio', { name: 'Custom #3A7BD5' }).first().waitFor(),
    },
    {
      id: 'appearance accent locked',
      path: '/account/appearance',
      opts: mock({
        signedIn: true,
        routes: {
          'GET public/config': { body: { ...CONFIG, branding: { ...BRANDING, userAccent: false, color: 'sage' } } },
        },
      }),
      prepare: (page) => page.getByText('Set by your administrator').first().waitFor(),
    },
    {
      id: 'appearance colour editor',
      path: '/account/appearance',
      opts: mock({ signedIn: true }),
      prepare: async (page) => {
        await page.getByRole('radio', { name: 'More', exact: true }).first().dispatchEvent('click');
        await page.getByRole('dialog', { name: 'More colors' }).first().waitFor();
      },
    },
  ];
};
