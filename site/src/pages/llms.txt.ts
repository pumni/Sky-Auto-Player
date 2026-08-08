import { SITE } from '../data/site';

export const prerender = true;

export function GET() {
  const base = `${SITE.productionOrigin}${SITE.basePath}`;
  const body = `# ${SITE.name}

> Windows desktop helper for timing-first playback of user-provided sheet files.
> Unofficial community project. Not affiliated with or endorsed by thatgamecompany.

## Official pages

- [Home](${base}/)
- [FAQ](${base}/faq/)
- [Vietnamese home](${base}/vi/)
- [Vietnamese FAQ](${base}/vi/faq/)
- [Latest release](${SITE.releaseUrl})
- [Source repository](${SITE.repositoryUrl})

## Guides

- [How It Works](${base}/guides/how-it-works/)
- [Supported Sheet Formats](${base}/guides/sheet-formats/)
- [Windows Setup and First Launch](${base}/guides/windows-setup/)
- [The Timing Engine](${base}/guides/timing-engine/)
- [Security Boundaries](${base}/guides/security-boundaries/)
- [Troubleshooting](${base}/guides/troubleshooting/)

## Vietnamese guides

- [How It Works (VI)](${base}/vi/guides/how-it-works/)
- [Supported Sheet Formats (VI)](${base}/vi/guides/sheet-formats/)
- [Windows Setup and First Launch (VI)](${base}/vi/guides/windows-setup/)
- [The Timing Engine (VI)](${base}/vi/guides/timing-engine/)
- [Security Boundaries (VI)](${base}/vi/guides/security-boundaries/)
- [Troubleshooting (VI)](${base}/vi/guides/troubleshooting/)

## Safety and scope

- The application is a Windows music playback helper, not a game modification tool.
- It reads user-provided song files and simulates keyboard input through the Windows SendInput API only.
- It does not read game memory, modify game files, inject code, install hooks, or bypass anti-cheat systems.
- Source code: https://github.com/pumni/Sky-Auto-Player (GPL-3.0)
`;
  return new Response(body, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
