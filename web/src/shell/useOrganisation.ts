import { useEffect, useState } from 'react';
import { api } from '../api';
import { useTokenVersion } from '../useTokenVersion';
import { organisationName } from './organisation';

/**
 * The organisation name the server is configured with, or `undefined`. The server sends it only to a caller with a
 * valid token, so the request is made again when a token is entered. The name is decoration: when the request fails
 * the page simply has none (or keeps the one it had) and says nothing, since every other request reports a server that
 * cannot be reached.
 */
export function useOrganisation(): string | undefined {
  const tokenVersion = useTokenVersion();
  const [name, setName] = useState<string>();
  useEffect(() => {
    let current = true;
    api.health().then(
      (body) => {
        if (current) setName(organisationName(body));
      },
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [tokenVersion]);
  return name;
}
