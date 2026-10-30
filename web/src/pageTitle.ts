import type { Route } from './router';

const PRODUCT = 'Hairspring';

function heading(route: Route): string {
  switch (route.name) {
    case 'runs':
      return 'Runs';
    case 'run':
      return `Run ${route.id}`;
    case 'seeds':
      return `Seeds of run ${route.id}`;
    case 'trial':
      return `Trial ${route.seq} of run ${route.id}`;
    case 'seed':
      return `Seed ${route.seed} of run ${route.id}`;
    case 'failure':
      return `Failure ${route.id}`;
    case 'targets':
      return 'Targets';
    case 'workers':
      return 'Workers';
    case 'notfound':
      return 'Page not found';
  }
}

/** The document title of a page: what it is about, then the product name, so tabs and history entries are told apart. */
export function pageTitle(route: Route): string {
  return `${heading(route)} · ${PRODUCT}`;
}
