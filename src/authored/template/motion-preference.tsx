import { MotionConfig } from 'motion/react';
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

const storageKey = 'mochi:accessibility-mode';
type MotionPreference = {
  accessibilityMode: boolean;
  reducedMotion: boolean;
  setAccessibilityMode: (enabled: boolean) => void;
};
const MotionContext = createContext<MotionPreference | null>(null);

export function MotionPreferenceProvider({ children }: { children: ReactNode }) {
  const [accessibilityMode, setEnabled] = useState(false);
  // Conservative during hydration: never start animation before reading preferences.
  const [systemReducedMotion, setSystemReducedMotion] = useState(true);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const query = window.matchMedia('(prefers-reduced-motion: reduce)');
    const updateSystemPreference = () => setSystemReducedMotion(query.matches);
    updateSystemPreference();
    try { setEnabled(localStorage.getItem(storageKey) === 'true'); } catch { /* Storage may be unavailable. */ }
    setReady(true);
    query.addEventListener('change', updateSystemPreference);
    const syncPreference = (event: StorageEvent) => {
      if (event.key === storageKey || event.key === null) setEnabled(event.newValue === 'true');
    };
    window.addEventListener('storage', syncPreference);
    return () => {
      query.removeEventListener('change', updateSystemPreference);
      window.removeEventListener('storage', syncPreference);
    };
  }, []);

  const reducedMotion = !ready || systemReducedMotion || accessibilityMode;
  useEffect(() => {
    const previous = document.documentElement.getAttribute('data-reduced-motion');
    document.documentElement.setAttribute('data-reduced-motion', String(reducedMotion));
    return () => {
      if (previous === null) document.documentElement.removeAttribute('data-reduced-motion');
      else document.documentElement.setAttribute('data-reduced-motion', previous);
    };
  }, [reducedMotion]);

  const setAccessibilityMode = useCallback((enabled: boolean) => {
    setEnabled(enabled);
    try { localStorage.setItem(storageKey, String(enabled)); } catch { /* The in-memory choice still applies. */ }
  }, []);
  const preference = useMemo(() => ({ accessibilityMode, reducedMotion, setAccessibilityMode }),
    [accessibilityMode, reducedMotion, setAccessibilityMode]);

  return <MotionContext.Provider value={preference}><MotionConfig reducedMotion={reducedMotion ? "always" : "never"}>{children}</MotionConfig></MotionContext.Provider>;
}

export function useMotionPreference(): MotionPreference {
  const preference = useContext(MotionContext);
  if (!preference) throw new Error('Motion controls require MotionPreferenceProvider');
  return preference;
}
