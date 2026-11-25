import { SITE } from '../data/site';

export const prerender = true;

export function GET() {
  const base = `${SITE.productionOrigin}${SITE.basePath}`;
  const body = `# ${SITE.name}

> Windows 11 x64 desktop app for timing-first playback of user-provided sheet files.
> Unofficial community project. Not affiliated with or endorsed by thatgamecompany.

## Current package and library

- The canonical package is a Tauri NSIS installer for current-user installation.
- Windows may show Unknown Publisher or a SmartScreen warning because the installer is unsigned for Authenticode under the current policy. The Tauri updater separately verifies the .exe.sig sidecar; manual installation does not require it.
- The Library includes built-in songs, liked songs, and playlists. Import local JSON, .skysheet, or JSON-compatible TXT files or folders into a playlist; imports reference the original files without moving or deleting them.
- Playback timing measurements are sender-side. Windows scheduling and Sky input sampling determine when the game registers input; the app does not guarantee in-game sound or frame timing.

## Official pages

- [Home](${base}/)
- [FAQ](${base}/faq/)
- [Vietnamese home](${base}/vi/)
- [Vietnamese FAQ](${base}/vi/faq/)
- [Releases](${SITE.releaseUrl})
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

- Automated playback may conflict with Sky's Terms of Service; technical boundaries do not guarantee account safety.
- The application is a Windows music playback helper, not a game modification tool.
- It reads user-provided song files and simulates keyboard input through the Windows SendInput API only.
- It does not read game memory, modify game files, inject code, install keyboard/input hooks, or bypass anti-cheat systems. One audited out-of-context foreground event watcher publishes hints only and cannot authorize input.
- Source code: https://github.com/pumni/Sky-Auto-Player (GPL-3.0)
`;
  return new Response(body, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
