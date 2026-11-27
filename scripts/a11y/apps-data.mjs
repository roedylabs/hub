// Made-up apps for the scenarios of the home page, Administration › Apps and › Users (`scenarios/`): as
// `GET apps` and `GET admin/apps` return them, with simple icons (`GET apps/<slug>/icon.svg`).
const icon = (shape) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#5b6b8c" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${shape}</svg>`;
const SHAPES = {
  mail: '<rect x="3" y="5" width="18" height="14" rx="3"/><path d="m4 7 8 6 8-6"/>',
  notes: '<rect x="5" y="3" width="14" height="18" rx="3"/><path d="M9 8h6M9 12h6M9 16h4"/>',
  calendar: '<rect x="3" y="5" width="18" height="16" rx="3"/><path d="M3 10h18M8 3v4M16 3v4"/>',
};

const at = (hoursAgo) => new Date(Date.now() - hoursAgo * 3_600_000).toISOString();
const iconUrl = (slug) => `/api/v1/apps/${slug}/icon.svg?v=0123456789abcdef`;

/** Answers for the icons. */
export const ICONS = Object.fromEntries(
  Object.entries(SHAPES).map(([slug, shape]) => [
    `GET apps/${slug}/icon.svg`,
    { raw: icon(shape), contentType: 'image/svg+xml' },
  ]),
);

/** Tiles of the home page. */
export const HOME_APPS = [
  {
    slug: 'mail',
    name: 'Mail',
    description: 'Email for the team',
    url: '/mail/',
    icon: iconUrl('mail'),
    status: 'online',
  },
  {
    slug: 'calendar',
    name: 'Calendar',
    description: 'Shared calendars and meeting rooms for everyone in the organization',
    url: '/calendar/',
    icon: iconUrl('calendar'),
    status: 'online',
  },
  {
    slug: 'notes',
    name: 'Notes',
    description: 'Shared notes',
    url: '/notes/',
    icon: iconUrl('notes'),
    status: 'offline',
  },
];

const events = { pending: 0, attempts: 0, failing_since: null, retry_at: null, last_error: null, given_up: false };
const roles = (what) => [
  { id: 'user', name: { en: 'User', de: 'Benutzer' }, description: { en: what, de: what }, default: true },
  {
    id: 'admin',
    name: { en: 'Administrator', de: 'Administrator' },
    description: { en: 'Also manages the app', de: 'Verwaltet auch die App' },
    admin: true,
  },
];
const paired = (slug, name, description, over = {}) => ({
  id: `app-${slug}`,
  slug,
  internal_url: `http://${slug}.internal:8080`,
  public_url: `https://hub.example.com/${slug}`,
  state: 'paired',
  manifest_id: `example.${slug}`,
  version: '1.4.0',
  name: { en: name, de: name },
  description: { en: description, de: description },
  home: '/',
  url: `/${slug}/`,
  icon: iconUrl(slug),
  roles: roles(description),
  access_default: 'all',
  position: 0,
  health: 'online',
  health_at: at(0.01),
  online_at: at(0.01),
  contacted_at: at(0.05),
  paired_at: at(400),
  created_at: at(401),
  pairing: null,
  user_count: 5,
  events,
  ...over,
});

/** Apps of the administration: two paired (one offline with failing events) and one pending. */
export const ADMIN_APPS = [
  paired('mail', 'Mail', 'Email for the team'),
  paired('notes', 'Notes', 'Shared notes', {
    position: 1,
    access_default: 'selected',
    health: 'offline',
    online_at: at(3),
    user_count: 2,
    events: { ...events, pending: 4, attempts: 3, failing_since: at(2), retry_at: at(-0.2), last_error: 'timeout' },
  }),
  {
    ...paired('calendar', 'Calendar', ''),
    state: 'pending',
    position: 2,
    manifest_id: null,
    version: null,
    name: null,
    description: null,
    home: null,
    url: null,
    icon: null,
    roles: null,
    health: null,
    health_at: null,
    online_at: null,
    contacted_at: null,
    paired_at: null,
    pairing: { expires_at: at(-0.2) },
    user_count: 0,
  },
];

export const PAIRING = {
  code: 'rhp_7K2MABCDEFGHJKMNPQRSTVWXYZ0123456',
  display: 'rhp_ 7K2M ABCD EFGH JKMN PQRS TVWX YZ01 2345 6',
  expires_at: at(-0.25),
};
