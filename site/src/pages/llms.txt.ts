import { SITE } from '../data/site';

export const prerender = true;

export function GET() {
  const base = `${SITE.productionOrigin}${SITE.basePath}`;
  const body = `# ${SITE.name}

> Windows desktop helper for timing-first playback of user-provided sheet files.

## Official pages

- [Home](${base}/)
- [FAQ](${base}/faq/)
- [Vietnamese home](${base}/vi/)
- [Vietnamese FAQ](${base}/vi/faq/)
- [Latest release](${SITE.releaseUrl})
- [Source repository](${SITE.repositoryUrl})

## Safety and scope

- The application is a Windows music playback helper, not a game modification tool.
- It reads user-provided song files and simulates keyboard input through the Windows SendInput API only.
- It does not read game memory, modify game files, inject code, install hooks, or bypass anti-cheat systems.
`;
  return new Response(body, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
