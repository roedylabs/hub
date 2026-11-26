// Administration › Organization: loaded, unsaved changes, field errors (live and from a 422), saving.
export default ({ mock, ORG }) => {
  const routes = (extra = {}) =>
    mock({
      signedIn: true,
      routes: { 'GET admin/org': { body: { ...ORG, website: 'https://example.com' } }, ...extra },
    });
  const path = '/admin/organization';
  const field = (page, name) => page.getByRole('textbox', { name, exact: true });
  const ready = (page) => field(page, 'Website').waitFor();
  const save = (page) =>
    page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' }).dispatchEvent('click');
  return [
    { id: 'organization', path, opts: routes(), prepare: ready },
    {
      id: 'organization unsaved',
      path,
      opts: routes(),
      prepare: async (page) => {
        await ready(page);
        await field(page, 'Name').fill('Example Inc');
      },
    },
    {
      id: 'organization field errors',
      path,
      opts: routes(),
      prepare: async (page) => {
        await ready(page);
        await field(page, 'Website').fill('example.com');
        await field(page, 'Support email').fill('support');
        await page.getByRole('alert').first().waitFor();
      },
    },
    {
      id: 'organization rejected',
      path,
      opts: routes({
        'PUT admin/org': { status: 422, body: { error: 'invalid', fields: { source: { code: 'too_long' } } } },
      }),
      prepare: async (page) => {
        await ready(page);
        await field(page, 'Source code').fill('https://git.example.com/example/hub');
        await save(page);
        await page.getByRole('alert').first().waitFor();
      },
    },
    {
      id: 'organization saving',
      path,
      opts: routes({ 'PUT admin/org': () => new Promise(() => {}) }),
      prepare: async (page) => {
        await ready(page);
        await field(page, 'Name').fill('Example Inc');
        await save(page);
        await page.locator('button[aria-busy="true"]').first().waitFor();
      },
    },
  ];
};
