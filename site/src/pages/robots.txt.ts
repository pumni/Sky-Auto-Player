import { SITE } from '../data/site';

export const prerender = true;

export function GET() {
  const sitemap = `${SITE.productionOrigin}${SITE.basePath}/sitemap-index.xml`;
  return new Response(`User-agent: *\nAllow: /\nSitemap: ${sitemap}\n`, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
