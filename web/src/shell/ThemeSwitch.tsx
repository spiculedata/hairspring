import { useEffect, useState } from 'react';
import { THEME_MODES, applyTheme, readStoredTheme, storeTheme, type ThemeMode } from './theme';

const LABELS: Readonly<Record<ThemeMode, string>> = { system: 'System', light: 'Light', dark: 'Dark' };

/** Three buttons that choose the colour theme. Each is a real button, so Tab and Enter or Space work; the pressed one is marked for screen readers. */
export function ThemeSwitch() {
  const [mode, setMode] = useState<ThemeMode>(() => readStoredTheme());
  useEffect(() => applyTheme(mode), [mode]);
  return (
    <div className="theme-switch" role="group" aria-label="Colour theme">
      {THEME_MODES.map((m) => (
        <button
          key={m}
          type="button"
          aria-pressed={m === mode}
          onClick={() => {
            storeTheme(m);
            setMode(m);
          }}
        >
          {LABELS[m]}
        </button>
      ))}
    </div>
  );
}
