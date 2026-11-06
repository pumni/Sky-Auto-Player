export function formatSongCount(count: number): string {
  return `${count} ${count === 1 ? 'song' : 'songs'}`;
}
