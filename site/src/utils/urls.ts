const BASE = import.meta.env.BASE_URL.replace(/\/$/, '');

function routeWithoutBase(pathname: string): string {
  const normalized = `/${pathname}`.replace(/\/+/g, '/');
  if (normalized === BASE || normalized === `${BASE}/`) return '/';
  if (normalized.startsWith(`${BASE}/`)) return normalized.slice(BASE.length) || '/';
  return normalized;
}

export function withBase(path = '/'): string {
  const normalized = path.startsWith('/') ? path : `/${path}`;
  if (normalized === '/') return `${BASE}/`;
  return `${BASE}${normalized}`.replace(/\/+/g, '/');
}

export function localizedPath(pathname: string, locale: 'en' | 'vi'): string {
  const route = routeWithoutBase(pathname);
  const englishRoute = route === '/vi/' ? '/' : route.startsWith('/vi/') ? route.slice(3) : route;
  const target =
    locale === 'vi' ? (englishRoute === '/' ? '/vi/' : `/vi${englishRoute}`) : englishRoute;
  return withBase(target);
}

export function isExternalUrl(value: string): boolean {
  return /^(?:[a-z]+:)?\/\//i.test(value) || value.startsWith('mailto:');
}
