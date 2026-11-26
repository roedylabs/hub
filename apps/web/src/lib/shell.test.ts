import { describe, expect, it, vi } from 'vitest';
import { confirmValues, overlayClose, pickKeys, skeletonValues, type PickKeysInput } from '#lib/shell.ts';

describe('confirmValues', () => {
  it('fills in the defaults: question mark, "Confirm", with "Cancel", without a word', () => {
    expect(confirmValues({ title: 'T', text: 'X' }, true, 'Confirm')).toEqual({
      title: 'T',
      text: 'X',
      icon: 'ti-help-circle',
      ok: 'Confirm',
      hasCancel: true,
      hasWord: false,
      word: '',
      danger: false,
      okDis: false,
    });
  });

  it('takes icon, label, danger and the word; "OK" stays disabled until the word matches', () => {
    const cf = confirmValues(
      { title: 'T', text: 'X', icon: 'ti-trash', ok: 'Delete', danger: true, word: 'DELETE', noCancel: true },
      false,
      'Confirm',
    );
    expect(cf).toMatchObject({ icon: 'ti-trash', ok: 'Delete', danger: true, hasWord: true, word: 'DELETE' });
    expect(cf).toMatchObject({ hasCancel: false, okDis: true });
  });
});

describe('pickKeys', () => {
  function keys(patch: Partial<PickKeysInput> = {}) {
    const input: PickKeysInput = {
      count: 3,
      open: false,
      disabled: false,
      idx: 1,
      hi: 1,
      mark: vi.fn(),
      show: vi.fn(),
      hide: vi.fn(),
      choose: vi.fn(),
      ...patch,
    };
    return { input, ...pickKeys(input) };
  }
  const key = (k: string) => ({ key: k, preventDefault: vi.fn() });

  it('opens with an arrow key at the selected row', () => {
    const { input, onKey } = keys();
    const e = key('ArrowDown');
    onKey(e);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(input.show).toHaveBeenCalled();
    expect(input.mark).toHaveBeenCalledWith(1);
  });

  it('moves the mark round and chooses with Enter or on releasing space', () => {
    const { input, onKey, onKeyUp } = keys({ open: true, hi: 2 });
    onKey(key('ArrowDown'));
    expect(input.mark).toHaveBeenLastCalledWith(0);
    onKey(key('ArrowUp'));
    expect(input.mark).toHaveBeenLastCalledWith(1);
    onKey(key('Enter'));
    expect(input.choose).toHaveBeenLastCalledWith(2);
    const space = key(' ');
    onKey(space);
    expect(space.preventDefault).toHaveBeenCalled();
    expect(input.choose).toHaveBeenCalledTimes(1);
    onKeyUp(key(' '));
    expect(input.choose).toHaveBeenCalledTimes(2);
  });

  it('closes with Tab and does nothing while disabled', () => {
    const open = keys({ open: true });
    open.onKey(key('Tab'));
    expect(open.input.hide).toHaveBeenCalled();
    expect(open.input.mark).toHaveBeenCalledWith(null);
    const off = keys({ open: true, disabled: true });
    off.onKey(key('Enter'));
    off.onKeyUp(key(' '));
    expect(off.input.choose).not.toHaveBeenCalled();
  });
});

describe('overlayClose', () => {
  it('closes only on the backdrop itself, without taking the focus', () => {
    const close = vi.fn();
    const scrim = {};
    const on = { target: scrim, currentTarget: scrim, preventDefault: vi.fn() };
    overlayClose(close)(on as never);
    expect(close).toHaveBeenCalledOnce();
    expect(on.preventDefault).toHaveBeenCalled();
    overlayClose(close)({ target: {}, currentTarget: scrim, preventDefault: vi.fn() } as never);
    expect(close).toHaveBeenCalledOnce();
  });
});

describe('skeletonValues', () => {
  it('shows a list of three rows or the overview', () => {
    const list = skeletonValues('list');
    expect(list).toMatchObject({ skSetList: true, skSetUeb: false });
    expect(list.skRows3.map((r) => [r.d, r.sep])).toEqual([
      ['0ms', false],
      ['140ms', true],
      ['280ms', true],
    ]);
    expect(skeletonValues('ueb')).toMatchObject({ skSetList: false, skSetUeb: true });
  });
});
