import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, BRAND0, ui } from '@roedy/ui';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { session } from '#lib/state/session.svelte.ts';
import { sessionOf } from '#lib/test/session.ts';
import { TEST_USER, resetSignedIn, signIn } from '#lib/test/signed-in.ts';
import {
  type AdminBranding,
  type BrandingDraft,
  BrandingPage,
  bodyOf,
  brandingValues,
  checkFile,
  defaultDraft,
  differs,
  fileMeta,
  imageErrorOf,
  imageErrorText,
  imageSize,
  normalized,
  svgHasText,
} from './branding.svelte.ts';

/** Pixel size that the mocked `Image` reports (jsdom loads no images). */
let pixels = { width: 512, height: 512 };
/** Number of created `Image`s (the page does not measure SVGs). */
let images = 0;

class FakeImage {
  naturalWidth = pixels.width;
  naturalHeight = pixels.height;
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() {
    images++;
  }
  set src(_: string) {
    queueMicrotask(() => this.onload?.());
  }
}

const IMAGE = '/branding/image/0123456789abcdef0123456789abcdef.png';

/** Saved branding with a logo, as `GET admin/branding` returns it. */
const SAVED: AdminBranding = {
  ...BRAND0,
  kind: 'image',
  image: IMAGE,
  imageName: 'example.png',
  imageMeta: 'PNG · 12 KB',
  color: 'sage',
};

const ctx = { scrollTo: vi.fn() };

/** An answer that arrives only when `release` is called. */
function later<T>(body: T, status = 200) {
  let release: () => void = () => {};
  const reply = () =>
    new Promise<{ status: number; body: T }>((resolve) => {
      release = () => resolve({ status, body });
    });
  return { reply, release: () => release() };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  signIn('admin');
  pixels = { width: 512, height: 512 };
  images = 0;
  vi.stubGlobal('Image', FakeImage);
  vi.stubGlobal(
    'createImageBitmap',
    vi.fn(() => Promise.reject(new Error('unexpected'))),
  );
});

afterEach(() => {
  resetSignedIn();
  ctx.scrollTo.mockClear();
});

const sent = (calls: { method: string; path: string }[]) => calls.map((c) => `${c.method} ${c.path}`);
const png = (bytes = 3, name = 'logo.png', type = 'image/png') => new File([new Uint8Array(bytes)], name, { type });

describe('helpers', () => {
  it('checks format and size of files', () => {
    expect(checkFile({ type: 'image/gif', size: 10 })).toBe('unsupported');
    expect(checkFile({ type: 'image/svg+xml', size: 1_048_577 })).toBe('too_large');
    expect(checkFile({ type: 'image/webp', size: 1_048_576 })).toBeNull();
  });

  it('describes files with format and size', () => {
    expect(fileMeta({ type: 'image/svg+xml', size: 100 })).toBe('SVG · 1 KB');
    expect(fileMeta({ type: 'image/jpeg', size: 2_000_000 })).toBe('JPG · 1,953 KB');
    expect(fileMeta({ type: 'image/webp', size: 12_288 })).toBe('WEBP · 12 KB');
  });

  it('names the reasons of the server with their texts', () => {
    expect(imageErrorText('unsupported')).toBe('This format isn’t supported. Use PNG, SVG, JPG, or WebP.');
    expect(imageErrorText('too_large')).toBe('The file is larger than 1 MB.');
    expect(imageErrorText('unreadable')).toBe('The file couldn’t be read.');
    expect(imageErrorText('svg_text')).toBe('Text in SVG isn’t supported. Convert the text to paths.');
    expect(imageErrorText('dimensions')).toBe('The image is too large. Use no more than 4096 × 4096 pixels.');
    expect(imageErrorText('required')).toBe('Please upload an image');
  });

  it('finds text in the SVG on the parsed document', () => {
    const NS = 'xmlns="http://www.w3.org/2000/svg"';
    expect(svgHasText(`<svg ${NS}><text x="0">A</text></svg>`)).toBe(true);
    expect(svgHasText(`<svg ${NS}><g><text/></g></svg>`)).toBe(true);
    expect(svgHasText('<svg:svg xmlns:svg="http://www.w3.org/2000/svg"><svg:text>A</svg:text></svg:svg>')).toBe(true);
    expect(svgHasText('<svg><text>A</text></svg>')).toBe(true);
    expect(svgHasText(`<svg ${NS}><!-- <text>A</text> --><path d="M0 0"/></svg>`)).toBe(false);
    expect(svgHasText(`<svg ${NS}><style><![CDATA[ <text> ]]></style><path d="M0 0"/></svg>`)).toBe(false);
    expect(svgHasText(`<svg ${NS}><desc>&lt;text&gt;</desc></svg>`)).toBe(false);
    expect(svgHasText(`<svg ${NS}><textPath href="#p"/><path d="M0 0"/></svg>`)).toBe(false);
    expect(svgHasText(`<svg ${NS}><TEXT>A</TEXT></svg>`)).toBe(false);
  });

  it('takes the regex if the SVG is not valid XML or DOMParser is missing', () => {
    expect(svgHasText('<svg:svg><svg:text>A</svg:text></svg:svg>')).toBe(true);
    expect(svgHasText('<svg><text>A</svg>')).toBe(true);
    expect(svgHasText('<svg><path d="M0 0"></svg>')).toBe(false);
    vi.stubGlobal('DOMParser', undefined);
    expect(svgHasText('<svg><!-- <text> --></svg>')).toBe(true);
    expect(svgHasText('<svg><path d="M0 0"/></svg>')).toBe(false);
  });

  it('measures images via an Image, createImageBitmap only without an Image', async () => {
    pixels = { width: 4097, height: 10 };
    await expect(imageSize(png(), 'data:,')).resolves.toEqual({ width: 4097, height: 10 });
    expect(images).toBe(1);
    expect(globalThis.createImageBitmap).not.toHaveBeenCalled();

    vi.stubGlobal('Image', undefined);
    const close = vi.fn();
    vi.stubGlobal(
      'createImageBitmap',
      vi.fn(() => Promise.resolve({ width: 300, height: 200, close })),
    );
    await expect(imageSize(png(), 'data:,')).resolves.toEqual({ width: 300, height: 200 });
    expect(close).toHaveBeenCalled();
  });

  it('reads the code at fields.image only from 422, 413 as too large', () => {
    const invalid = (code: string, field = 'image') =>
      new ApiError(422, { error: 'invalid', fields: { [field]: { code } } });
    expect(imageErrorOf(invalid('svg_text'))).toBe('svg_text');
    expect(imageErrorOf(invalid('dimensions'))).toBe('dimensions');
    expect(imageErrorOf(invalid('required'))).toBe('required');
    expect(imageErrorOf(invalid('invalid'))).toBeNull();
    expect(imageErrorOf(invalid('too_long', 'letter'))).toBeNull();
    expect(imageErrorOf(new ApiError(413, {}))).toBe('too_large');
    expect(imageErrorOf(new ApiError(500, { error: 'internal' }))).toBeNull();
  });

  it('normalises the draft before saving and compares drafts', () => {
    expect(normalized({ ...defaultDraft(), letter: ' A ' }).letter).toBe('A');
    expect(differs({ ...defaultDraft(), letter: ' ' }, defaultDraft())).toBe(false);
    expect(differs({ ...defaultDraft(), tilt: false }, defaultDraft())).toBe(true);
  });

  it('sends only changed fields, a new logo with details, a removed one as null', () => {
    const prev: BrandingDraft = { ...SAVED };
    expect(bodyOf({ ...prev, tilt: false, color: 'rose' }, prev)).toEqual({ tilt: false, color: 'rose' });
    expect(
      bodyOf({ ...prev, image: 'data:image/png;base64,AAAA', imageName: 'new.png', imageMeta: 'PNG · 1 KB' }, prev),
    ).toEqual({ image: 'data:image/png;base64,AAAA', imageName: 'new.png', imageMeta: 'PNG · 1 KB' });
    expect(bodyOf({ ...prev, kind: 'standard', image: null, imageName: '', imageMeta: '' }, prev)).toEqual({
      kind: 'standard',
      image: null,
    });
  });
});

describe('BrandingPage', () => {
  it('starts with public/config and takes admin/branding with the file details', async () => {
    const api = mockApi({ 'GET admin/branding': [{ body: SAVED }] });
    const p = new BrandingPage(ctx);
    expect(p.draft.kind).toBe('standard');
    expect(p.draft.imageName).toBe('');
    await p.load();
    expect(sent(api.calls)).toEqual(['GET admin/branding']);
    expect(session.branding).toMatchObject({ kind: 'image', image: IMAGE, color: 'sage' });
    expect(session.branding).not.toHaveProperty('imageName');
    expect(session.logo).toMatchObject({ isPad: true, src: IMAGE });
    expect(p.draft).toMatchObject({ imageName: 'example.png', imageMeta: 'PNG · 12 KB' });
    expect(p.dirty).toBe(false);
  });

  it('stays with public/config if admin/branding fails; 403 reloads the account', async () => {
    mockApi({ 'GET admin/branding': [{ status: 500, body: { error: 'internal' } }] });
    const p = new BrandingPage(ctx);
    await p.load();
    expect(p.draft.kind).toBe('standard');
    expect(ui.toast).toBeNull();

    const api = mockApi({
      'GET admin/branding': [{ status: 403, body: { error: 'forbidden' } }],
      'GET session': [{ body: sessionOf({ ...TEST_USER, role: 'user' }) }],
    });
    await p.load();
    await vi.waitFor(() => expect(session.isAdmin).toBe(false));
    expect(sent(api.calls)).toEqual(['GET admin/branding', 'GET session']);
  });

  it('of two overlapping loads only the later one applies', async () => {
    const first = later({ ...SAVED, color: 'rose' as const });
    const api = mockApi({ 'GET admin/branding': [first.reply, { body: SAVED }] });
    const p = new BrandingPage(ctx);
    const one = p.load();
    await p.load();
    first.release();
    await one;
    expect(sent(api.calls)).toEqual(['GET admin/branding', 'GET admin/branding']);
    expect(session.branding.color).toBe('sage');
  });

  it('a load that returns after a save does not bring back the old state', async () => {
    const slow = later(SAVED);
    mockApi({
      'GET admin/branding': [slow.reply],
      'PUT admin/branding': [{ body: { ...SAVED, kind: 'standard', image: null, imageName: '', imageMeta: '' } }],
    });
    const p = new BrandingPage(ctx);
    const loading = p.load();
    p.set('tilt', false);
    await p.save();
    slow.release();
    await loading;
    expect(session.branding).toMatchObject({ kind: 'standard', image: null });
    expect(p.dirty).toBe(false);
  });

  it('a load does not write after the page is left or the session changed', async () => {
    const slow = later(SAVED);
    mockApi({ 'GET admin/branding': [slow.reply] });
    const p = new BrandingPage(ctx);
    const loading = p.load();
    p.dispose();
    slow.release();
    await loading;
    expect(session.branding).toEqual(BRAND0);

    const other = later(SAVED);
    mockApi({ 'GET admin/branding': [other.reply] });
    const q = new BrandingPage(ctx);
    const again = q.load();
    session.config = { ...session.config!, branding: { ...BRAND0, color: 'sky' } };
    other.release();
    await again;
    expect(session.branding.color).toBe('sky');
  });

  it('reads a file as a data URL and switches to Image', async () => {
    const p = new BrandingPage(ctx);
    await p.readFile(png(2048));
    expect(p.draft).toMatchObject({ kind: 'image', imageName: 'logo.png', imageMeta: 'PNG · 2 KB' });
    expect(p.draft.image).toMatch(/^data:image\/png;base64,/);
    expect(p.dirty).toBe(true);
    expect(p.error).toBeNull();
  });

  it('rejects an SVG with text, accepts one without text', async () => {
    const svg = (body: string) => new File([body], 'logo.svg', { type: 'image/svg+xml' });
    const p = new BrandingPage(ctx);
    await p.readFile(svg('<svg xmlns="http://www.w3.org/2000/svg"><text>Example</text></svg>'));
    expect(p.error).toBe('Text in SVG isn’t supported. Convert the text to paths.');
    expect(p.dirty).toBe(false);
    await p.readFile(svg('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0h10v10z"/></svg>'));
    expect(p.error).toBeNull();
    expect(p.draft).toMatchObject({ kind: 'image', imageName: 'logo.svg' });
    expect(images).toBe(0);
  });

  it('rejects images over 4096 pixels in width or height', async () => {
    const p = new BrandingPage(ctx);
    pixels = { width: 4097, height: 100 };
    await p.readFile(png(100));
    expect(p.error).toBe('The image is too large. Use no more than 4096 × 4096 pixels.');
    expect(p.dirty).toBe(false);
    pixels = { width: 100, height: 4097 };
    await p.readFile(png(100, 'logo.jpg', 'image/jpeg'));
    expect(p.error).toBe('The image is too large. Use no more than 4096 × 4096 pixels.');
    pixels = { width: 4096, height: 4096 };
    await p.readFile(png(100, 'logo.webp', 'image/webp'));
    expect(p.error).toBeNull();
    expect(p.draft).toMatchObject({ kind: 'image', imageName: 'logo.webp' });
  });

  it('an image that cannot be decoded is unreadable', async () => {
    class BrokenImage {
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      set src(_: string) {
        queueMicrotask(() => this.onerror?.());
      }
    }
    vi.stubGlobal('Image', BrokenImage);
    const p = new BrandingPage(ctx);
    await p.readFile(png(100));
    expect(p.error).toBe('The file couldn’t be read.');
    expect(p.dirty).toBe(false);
  });

  it('rejects wrong formats and too large files', async () => {
    const p = new BrandingPage(ctx);
    await p.readFile(png(3, 'logo.gif', 'image/gif'));
    expect(p.error).toBe('This format isn’t supported. Use PNG, SVG, JPG, or WebP.');
    expect(p.dirty).toBe(false);
    await p.readFile(png(1_048_577));
    expect(p.error).toBe('The file is larger than 1 MB.');
    p.setKind('icon');
    expect(p.error).toBeNull();
  });

  it('Remove clears logo and details, Discard reverts everything', async () => {
    mockApi({ 'GET admin/branding': [{ body: SAVED }] });
    const p = new BrandingPage(ctx);
    await p.load();
    p.removeImage();
    expect(p.draft).toMatchObject({ image: null, imageName: '', imageMeta: '' });
    expect(p.dirty).toBe(true);
    p.discard();
    expect(p.draft.image).toBe(IMAGE);
    expect(p.dirty).toBe(false);
  });

  it('initial at most two characters, emoji stay whole', () => {
    const p = new BrandingPage(ctx);
    p.setLetter('ABC');
    expect(p.draft.letter).toBe('AB');
    p.setLetter('A😀B');
    expect(p.draft.letter).toBe('A😀');
  });

  it('Discard, Remove, Reset, another kind and leaving the page cancel a running read', async () => {
    for (const cancel of [
      (p: BrandingPage) => p.discard(),
      (p: BrandingPage) => p.removeImage(),
      (p: BrandingPage) => p.reset(),
      (p: BrandingPage) => p.setKind('icon'),
      (p: BrandingPage) => p.dispose(),
    ]) {
      const p = new BrandingPage(ctx);
      const reading = p.readFile(png(100));
      cancel(p);
      await reading;
      expect(p.draft.image).toBeNull();
      expect(p.draft.kind).not.toBe('image');
    }
  });

  it('a later file wins over an earlier one that is still being read', async () => {
    const p = new BrandingPage(ctx);
    const first = p.readFile(png(100, 'first.png'));
    const second = p.readFile(png(100, 'second.png'));
    await Promise.all([first, second]);
    expect(p.draft.imageName).toBe('second.png');
  });

  it('does not save without an image, but jumps to the logo', async () => {
    const api = mockApi({});
    const p = new BrandingPage(ctx);
    p.setKind('image');
    await p.save();
    expect(ui.toast?.text).toBe('Please upload an image');
    expect(ctx.scrollTo).toHaveBeenCalledWith('logo');
    expect(api.calls).toEqual([]);
  });

  it('sends the changed fields and takes the response over into the session', async () => {
    const api = mockApi({
      'PUT admin/branding': [
        {
          body: {
            ...BRAND0,
            kind: 'image',
            image: IMAGE,
            imageName: 'logo.png',
            imageMeta: 'PNG · 1 KB',
            userAccent: false,
          },
        },
      ],
    });
    const p = new BrandingPage(ctx);
    p.set('userAccent', false);
    await p.readFile(png(100));
    const saving = p.save();
    expect(p.saving).toBe(true);
    expect(session.branding.userAccent).toBe(true);
    await saving;
    expect(p.saving).toBe(false);
    expect(sent(api.calls)).toEqual(['PUT admin/branding']);
    expect(api.calls[0]?.body).toMatchObject({
      kind: 'image',
      userAccent: false,
      imageName: 'logo.png',
      imageMeta: 'PNG · 1 KB',
    });
    expect((api.calls[0]?.body as { image: string }).image).toMatch(/^data:image\/png;base64,/);
    expect(session.branding).toMatchObject({ image: IMAGE, userAccent: false });
    expect(session.logo).toMatchObject({ isPad: true, src: IMAGE });
    expect(p.draft).toMatchObject({ image: IMAGE, imageName: 'logo.png' });
    expect(p.dirty).toBe(false);
    expect(ui.toast).toMatchObject({ text: 'Changes saved', error: false });
  });

  it('only whitespace in the initial: nothing to send', async () => {
    const api = mockApi({});
    const p = new BrandingPage(ctx);
    p.setLetter(' ');
    expect(p.dirty).toBe(false);
    await p.save();
    expect(api.calls).toEqual([]);
  });

  it('keeps inputs made while saving and ignores a second save meanwhile', async () => {
    const put = later({ ...SAVED, kind: 'standard', image: null, imageName: '', imageMeta: '', color: 'rose' });
    const api = mockApi({ 'PUT admin/branding': [put.reply] });
    const p = new BrandingPage(ctx);
    p.set('color', 'rose');
    const saving = p.save();
    p.set('tilt', false);
    await p.save();
    put.release();
    await saving;
    expect(sent(api.calls)).toEqual(['PUT admin/branding']);
    expect(api.calls[0]?.body).toEqual({ color: 'rose' });
    expect(session.branding.color).toBe('rose');
    expect(p.edits).toEqual({ tilt: false });
    expect(p.dirty).toBe(true);
  });

  it('Discard does nothing while saving', async () => {
    const put = later({ ...SAVED, kind: 'standard', image: null, imageName: '', imageMeta: '', color: 'rose' });
    mockApi({ 'PUT admin/branding': [put.reply] });
    const p = new BrandingPage(ctx);
    p.set('color', 'rose');
    const saving = p.save();
    p.discard();
    expect(p.draft.color).toBe('rose');
    put.release();
    await saving;
  });

  it('shows logo errors in brErr and keeps the draft', async () => {
    mockApi({
      'GET admin/branding': [{ body: SAVED }],
      'PUT admin/branding': [{ status: 422, body: { error: 'invalid', fields: { image: { code: 'svg_text' } } } }],
    });
    const p = new BrandingPage(ctx);
    await p.load();
    await p.readFile(png(10, 'logo.svg', 'image/svg+xml'));
    await p.save();
    expect(session.branding.image).toBe(IMAGE);
    expect(p.error).toBe('Text in SVG isn’t supported. Convert the text to paths.');
    expect(ctx.scrollTo).toHaveBeenCalledWith('logo');
    expect(p.draft).toMatchObject({ imageName: 'logo.svg', imageMeta: 'SVG · 1 KB' });
    expect(p.draft.image).toMatch(/^data:image\/svg\+xml;base64,/);
    expect(p.dirty).toBe(true);
    expect(ui.toast).toBeNull();
  });

  it('shows logo errors as a toast if the page has already been left', async () => {
    const put = later({ error: 'invalid', fields: { image: { code: 'unsupported' } } }, 422);
    mockApi({ 'PUT admin/branding': [put.reply] });
    const p = new BrandingPage(ctx);
    await p.readFile(png(10));
    const saving = p.save();
    p.dispose();
    put.release();
    await saving;
    expect(session.branding.image).toBeNull();
    expect(ui.toast).toMatchObject({ text: 'This format isn’t supported. Use PNG, SVG, JPG, or WebP.', error: true });
    expect(ctx.scrollTo).not.toHaveBeenCalled();
  });

  it('a save does not mark or clear the logo error of a file chosen meanwhile', async () => {
    const put = later({ error: 'invalid', fields: { image: { code: 'svg_text' } } }, 422);
    mockApi({ 'PUT admin/branding': [put.reply] });
    const p = new BrandingPage(ctx);
    await p.readFile(png(10, 'logo.svg', 'image/svg+xml'));
    const saving = p.save();
    await p.readFile(png(20, 'other.png'));
    put.release();
    await saving;
    // The rejected SVG is no longer the draft: a toast, no mark at the new image.
    expect(p.error).toBeNull();
    expect(ctx.scrollTo).not.toHaveBeenCalled();
    expect(ui.toast).toMatchObject({ text: 'Text in SVG isn’t supported. Convert the text to paths.', error: true });
    expect(p.draft.imageName).toBe('other.png');

    const ok = later({ ...SAVED, imageName: 'logo.png', imageMeta: 'PNG · 1 KB' });
    mockApi({ 'PUT admin/branding': [ok.reply] });
    const q = new BrandingPage(ctx);
    await q.readFile(png(10));
    const again = q.save();
    await q.readFile(png(10, 'logo.gif', 'image/gif'));
    ok.release();
    await again;
    // The newer file's error stays.
    expect(q.error).toBe('This format isn’t supported. Use PNG, SVG, JPG, or WebP.');
  });

  it('a file still being read when saving owns the logo error once it is done', async () => {
    // The draft shows an SVG, a PNG is still being read on "Save": the SVG goes, the PNG comes in afterwards.
    const put = later({ error: 'invalid', fields: { image: { code: 'svg_text' } } }, 422);
    const api = mockApi({ 'PUT admin/branding': [put.reply] });
    const p = new BrandingPage(ctx);
    await p.readFile(png(10, 'logo.svg', 'image/svg+xml'));
    const reading = p.readFile(png(20, 'other.png'));
    const saving = p.save();
    expect((api.calls[0]?.body as { imageName: string }).imageName).toBe('logo.svg');
    await reading;
    expect(p.draft.imageName).toBe('other.png');
    put.release();
    await saving;
    // The rejection belongs to the SVG: a toast, no mark at the PNG.
    expect(p.error).toBeNull();
    expect(ctx.scrollTo).not.toHaveBeenCalled();
    expect(ui.toast).toMatchObject({ text: 'Text in SVG isn’t supported. Convert the text to paths.', error: true });

    // The read fails while a successful save runs: its error stays.
    const ok = later({ ...SAVED, imageName: 'logo.png', imageMeta: 'PNG · 1 KB' });
    mockApi({ 'PUT admin/branding': [ok.reply] });
    const q = new BrandingPage(ctx);
    await q.readFile(png(10));
    pixels = { width: 5000, height: 100 };
    const big = q.readFile(png(10, 'big.png'));
    const again = q.save();
    await big;
    expect(q.error).toBe('The image is too large. Use no more than 4096 × 4096 pixels.');
    ok.release();
    await again;
    expect(q.error).toBe('The image is too large. Use no more than 4096 × 4096 pixels.');
  });

  it('saves run one after another across page instances, so the session ends with the state stored last', async () => {
    const plain = { ...SAVED, kind: 'standard', image: null, imageName: '', imageMeta: '' } as const;
    const slow = later({ ...plain, color: 'rose' });
    const fast = later({ ...plain, color: 'sky' });
    const api = mockApi({ 'PUT admin/branding': [slow.reply, fast.reply] });
    const a = new BrandingPage(ctx);
    a.set('color', 'rose');
    const first = a.save();
    a.dispose();
    const b = new BrandingPage(ctx);
    b.set('color', 'sky');
    const second = b.save();
    await tick();
    // The later save waits for the earlier one: the server cannot store them in the other order.
    expect(sent(api.calls)).toEqual(['PUT admin/branding']);
    expect(b.saving).toBe(true);
    slow.release();
    await first;
    expect(session.branding.color).toBe('rose');
    await vi.waitFor(() => expect(sent(api.calls)).toEqual(['PUT admin/branding', 'PUT admin/branding']));
    expect(api.calls[1]?.body).toEqual({ color: 'sky' });
    fast.release();
    await second;
    expect(session.branding.color).toBe('sky');
    expect(b.draft.color).toBe('sky');
    expect(b.dirty).toBe(false);
  });

  it('a queued save builds its request on the state the save before it stored', async () => {
    const plain = { ...SAVED, kind: 'standard', image: null, imageName: '', imageMeta: '' } as const;
    session.config = { ...session.config!, branding: { ...plain } };
    const before = plain.color;
    const slow = later({ ...plain, color: 'rose' });
    const fast = later({ ...plain, color: before, tilt: !plain.tilt });
    const api = mockApi({ 'PUT admin/branding': [slow.reply, fast.reply] });
    const a = new BrandingPage(ctx);
    a.set('color', 'rose');
    const first = a.save();
    a.dispose();
    // Opened before the first save answered: it still sees the old colour and sets it explicitly.
    const b = new BrandingPage(ctx);
    b.set('color', before === 'rose' ? 'sky' : before);
    b.set('color', before);
    b.set('tilt', !plain.tilt);
    const second = b.save();
    slow.release();
    await first;
    await vi.waitFor(() => expect(sent(api.calls)).toEqual(['PUT admin/branding', 'PUT admin/branding']));
    // Against the stored "rose" the colour is a change and goes along; the tilt as well.
    expect(api.calls[1]?.body).toEqual({ color: before, tilt: !plain.tilt });
    fast.release();
    await second;
    expect(session.branding.color).toBe(before);
  });

  it('a failed save does not hold up the next one', async () => {
    const failing = later({ error: 'internal' }, 500);
    const api = mockApi({
      'PUT admin/branding': [
        failing.reply,
        { body: { ...SAVED, kind: 'standard', image: null, imageName: '', imageMeta: '', color: 'sky' } },
      ],
    });
    const a = new BrandingPage(ctx);
    a.set('color', 'rose');
    const first = a.save();
    a.dispose();
    const b = new BrandingPage(ctx);
    b.set('color', 'sky');
    const second = b.save();
    failing.release();
    await Promise.all([first, second]);
    expect(sent(api.calls)).toEqual(['PUT admin/branding', 'PUT admin/branding']);
    expect(session.branding.color).toBe('sky');
  });

  it('a save that answers after the page was left still applies if it is the latest', async () => {
    const slow = later({ ...SAVED, kind: 'standard', image: null, imageName: '', imageMeta: '', color: 'rose' });
    mockApi({ 'PUT admin/branding': [slow.reply] });
    const p = new BrandingPage(ctx);
    p.set('color', 'rose');
    const saving = p.save();
    p.dispose();
    slow.release();
    await saving;
    expect(session.branding.color).toBe('rose');
  });

  it('other errors show the error toast and keep the inputs', async () => {
    mockApi({
      'PUT admin/branding': [{ status: 422, body: { error: 'invalid', fields: { letter: { code: 'too_long' } } } }],
    });
    const p = new BrandingPage(ctx);
    p.setKind('initial');
    p.setLetter('XY');
    await p.save();
    expect(p.draft).toMatchObject({ kind: 'initial', letter: 'XY' });
    expect(p.error).toBeNull();
    expect(ui.toast).toMatchObject({ text: 'That didn’t work. Please try again.', error: true });
  });

  it('without a connection the toast says so', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('offline'))),
    );
    const p = new BrandingPage(ctx);
    p.set('tilt', false);
    await p.save();
    expect(ui.toast?.error).toBe(true);
    expect(ui.toast?.text).not.toBe('That didn’t work. Please try again.');
    expect(p.dirty).toBe(true);
  });

  it('401 and 403 on save reload the account', async () => {
    const api = mockApi({
      'PUT admin/branding': [{ status: 403, body: { error: 'forbidden' } }],
      'GET session': [{ body: sessionOf({ ...TEST_USER, role: 'user' }) }],
    });
    const p = new BrandingPage(ctx);
    p.set('tilt', false);
    await p.save();
    await vi.waitFor(() => expect(session.isAdmin).toBe(false));
    expect(sent(api.calls)).toEqual(['PUT admin/branding', 'GET session']);
    expect(ui.toast?.error).toBe(true);
  });

  it('Restore defaults sets BRAND0 as a draft and removes the logo when saving', async () => {
    const api = mockApi({
      'GET admin/branding': [{ body: SAVED }],
      'PUT admin/branding': [{ body: { ...BRAND0, imageName: '', imageMeta: '' } }],
    });
    const p = new BrandingPage(ctx);
    await p.load();
    expect(p.notDefault).toBe(true);
    p.reset();
    expect(p.notDefault).toBe(false);
    expect(p.dirty).toBe(true);
    await p.save();
    expect(api.calls[1]?.body).toEqual({ kind: 'standard', color: 'apricot', image: null });
    expect(session.branding).toEqual(BRAND0);
    await tick();
    expect(p.dirty).toBe(false);
  });
});

describe('brandingValues', () => {
  it('returns preview, kinds, icons and colours from the draft', () => {
    const p = new BrandingPage(ctx);
    let v = brandingValues(p, ctx);
    expect(v.brTabs.map((t) => t.label)).toEqual(['Logo', 'Brand color']);
    expect(v.brPrev).toMatchObject({ name: 'Example Org', bg: '#f5a06e', ink: '#3a1b06', tilt: 'rotate(-4deg)' });
    expect(v.brPrev.logo.isMark).toBe(true);
    expect(v.brKindOpts.map((o) => [o.label, o.on])).toEqual([
      ['Default', true],
      ['Initials', false],
      ['Icon', false],
      ['Image', false],
    ]);
    expect(v.brKindExtra).toBe(false);
    expect(v.brIcons).toHaveLength(12);
    expect(v.brIcons[0]).toMatchObject({ icon: 'ti-mountain', label: 'Mountain', pressed: true });
    expect(v.brColors.map((c) => c.name)).toEqual(['apricot', 'sage', 'lavender', 'sky', 'rose']);
    expect(v.brColors.find((c) => c.on)?.name).toBe('apricot');
    expect(v.brImgName).toBe('Logo');
    expect(v.brLetterPh).toBe('E');

    v.brKindOpts[1]?.onClick();
    v.brPlateOpts[2]?.onClick();
    v.tgBrTilt.onClick();
    v.tgBrUserAcc.onClick();
    v = brandingValues(p, ctx);
    expect(v.brIsInit).toBe(true);
    expect(v.brPrev).toMatchObject({ bg: '#2b221c', ink: '#fffaf3', tilt: 'none' });
    expect(v.brPrev.logo).toMatchObject({ isInit: true, text: 'E' });
    expect(v.tgBrUserAcc.on).toBe(false);
    expect(v.brDirty).toBe(true);
    expect(v.brNotDefault).toBe(true);

    v.brTabs[1]?.onClick();
    expect(ctx.scrollTo).toHaveBeenCalledWith('color');
  });
});
