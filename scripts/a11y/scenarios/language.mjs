// Account › Language: a language chosen, "System" with the note on mails, and the page in German.
export default ({ mock }) => {
  const settings = (lang) => ({
    'GET settings': { body: { version: 1, settings: { lang, mode: null, accent: null } } },
  });
  return [
    { id: 'language', path: '/account/language', opts: mock({ signedIn: true }) },
    {
      id: 'language system',
      path: '/account/language',
      opts: mock({ signedIn: true, routes: settings('system') }),
      prepare: (page) => page.getByText('use the language of your last sign-in').first().waitFor(),
    },
    {
      id: 'language german',
      path: '/account/language',
      opts: mock({ signedIn: true, routes: settings('de') }),
      prepare: (page) => page.getByRole('heading', { name: 'Sprache', level: 1 }).first().waitFor(),
    },
  ];
};
