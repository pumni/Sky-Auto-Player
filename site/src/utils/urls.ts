const BASE = import.meta.env.BASE_URL.replace(/\/$/, '');

export function withBase(path = '/'): string {
  const normalized = path.startsWith('/') ? path : `/${path}`;
  if (normalized === '/') return `${BASE}/`;
  return `${BASE}${normalized}`.replace(/\/+/g, '/');
}

export function isExternalUrl(value: string): boolean {
  return /^(?:[a-z]+:)?\/\//i.test(value) || value.startsWith('mailto:');
}
