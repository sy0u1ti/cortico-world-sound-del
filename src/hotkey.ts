export interface DirectHotkey {
  keys: number[];
  taps: number;
  mode: 'hold' | 'toggle';
}

const KEYS: Readonly<Record<string, number>> = {
  Ctrl: 0x11, LeftCtrl: 0xa2, RightCtrl: 0xa3,
  Alt: 0x12, LeftAlt: 0xa4, RightAlt: 0xa5,
  Shift: 0x10, LeftShift: 0xa0, RightShift: 0xa1,
  Win: 0x5b, RightWin: 0x5c,
  Space: 0x20, Tab: 0x09, CapsLock: 0x14, Backquote: 0xc0, Enter: 0x0d,
  Insert: 0x2d, Delete: 0x2e, Home: 0x24, End: 0x23, PageUp: 0x21, PageDown: 0x22,
  Pause: 0x13, ScrollLock: 0x91, Mouse3: 0x04, Mouse4: 0x05, Mouse5: 0x06,
};

export function parseHotkey(value: unknown, mode: 'hold' | 'toggle'): DirectHotkey | null {
  if (typeof value !== 'string' || value.length > 160) return null;
  const match = /^(.*?)(?:\s*\*\s*([1-3]))?$/.exec(value.trim());
  if (!match) return null;
  const names = match[1].split('+').map((part) => part.trim()).filter(Boolean);
  if (names.length === 0 || names.length > 8) return null;
  const keys: number[] = [];
  for (const name of names) {
    let code = Object.hasOwn(KEYS, name) ? KEYS[name] : undefined;
    if (code === undefined && /^[A-Z0-9]$/.test(name)) code = name.charCodeAt(0);
    const fn = /^F([1-9]|1\d|2[0-4])$/.exec(name);
    if (code === undefined && fn) code = 0x6f + Number(fn[1]);
    if (code === undefined) return null;
    if (!keys.includes(code)) keys.push(code);
  }
  return { keys, taps: Number(match[2] ?? 1), mode };
}
