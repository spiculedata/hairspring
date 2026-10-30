import { createContext, useContext, useEffect } from 'react';

/** Lets a page tell the app that the thing it is about does not exist, so the title says so. */
export const MissingPageContext = createContext<(missing: boolean) => void>(() => undefined);

export function useReportMissing(missing: boolean): void {
  const report = useContext(MissingPageContext);
  useEffect(() => {
    report(missing);
    return () => report(false);
  }, [missing, report]);
}
