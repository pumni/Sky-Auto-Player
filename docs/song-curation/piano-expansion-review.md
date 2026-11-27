# Ten piano arrangement expansion — candidate qualification

Status: **draft / not approved for release**. This change adds ten candidate arrangements to the proposed built-in catalog. The source selection is based on structural screening, not an acoustic quality certification. The upstream collection's compilation terms do not establish rights to distribute the compositions or arrangements. Verify rights before merge/release; a public PR is not evidence of permission.

## Provenance

All source paths are relative to [Ai-Vonie/Sky1984-Sheets-Collection at fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/tree/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea). The upstream .txt files contain JSON; only the destination extension/name changes. The destination Git blob SHA was checked against the source blob SHA for all ten files. The SHA-256 values below are for the actual committed bytes, independently computed and registered in the manifest. Source credits are clues, not verified attribution.

| Destination | Upstream path | Arrangement credit | SHA-256 |
| --- | --- | --- | --- |
| `songs/Nuvole Bianche.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/L/Ludovico%20Einaudi%20-%20Nuvole%20Bianche%20(by%20San).txt) | San / Sen Rovelle | `131d473e0675c2194cd8ce32daf29ed0d823c5a4f347bc169c38f4cf02c7c064` |
| `songs/Interstellar Main Theme.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/I/Interstellar%20Main%20Theme%20(created%20by%20The%20Friendly%20Crab%2C%20transferred%20to%20SM%20by%20RikkiTeiler).txt) | The Friendly Crab; transfer RikkiTeiler | `050ad78cbed8e58acd0ce8273958327446c96422ea547adbdd7bd7996d1a15c9` |
| `songs/Merry Christmas Mr. Lawrence.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/R/Ryuichi%20Sakamoto%20-%20Merry%20Christmas%20Mr.%20Lawrence%20(by%20Jyun).txt) | Jyun | `76781708d30a950e6b59bb43d3b58ecf49ae0562c9e4134d593b79da5fce7927` |
| `songs/Comptine d'un autre été.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/C/Comptine%20dun%20Autre%20ete.txt) | @thatskyspaidy | `8658729999d05891a40aad670ae19f687016358cbce61284e9df9c3d3ace22e1` |
| `songs/The Truth That You Leave.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/T/The%20Truth%20that%20You%20Leave%20_~%7B%5B%23v4%5D%7D~_.txt) | Not independently established | `5f29acf0b5345b186cf1a3ac4ee9f927bb93529e55fa189c76072ca84b7a0b8b` |
| `songs/Mariage d'Amour.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/R/Richard%20Clayderman%20-%20Mariage%20D'amour.txt) | Not independently established | `cfab6ae28229f9b329a1654ebf150bdc2a9e47629865ec419efc72e8c595738c` |
| `songs/Viva La Vida.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/V/Viva%20La%20Vida.txt) | Not independently established | `e46376641d3aeb674aa816503c62dbf6fe0e31545a187f9a78d3bef449c92e4a` |
| `songs/Hallelujah.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/L/Leonard%20Cohen%20-%20Hallelujah.txt) | Not independently established | `ebd1e32aa49f93f7e341edf6de1e6b71c8274574ce5db6a06dd19a4774d71008` |
| `songs/Kamado Tanjiro no Uta.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/G/Go%20Shiina%20-%20Kamado%20Tanjiro%20no%20Uta%20OST%20Demon%20Slayer%20Kimetsu%20no%20Yaiba%20(by%20KiMuJo).txt) | KiMuJo | `58d162ab84dc8bd6080d978609d9601edc51619184f5fcfec353c6d76085866b` |
| `songs/Sweden.json` | [source](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/Songs/C/C418%20-%20Sweden%20OST%20Minecraft.txt) | Not independently established | `a929eeea30984d8a5aeaff07ea6cd5dd71f49aa52c9bd4445abb1a4d9d9a450b` |

## Structural pre-screen (independent, not Rust audit output)

Calculated from authoritative composed columns or recorded notes using the documented timing rules. All ten have Sky/Piano metadata, pitch-inheriting tracks, valid key indices and layer masks in this screen, no duplicate physical onset/key pairs, and minimum same-key intervals above 16.67 ms. This is not proof of native playback compatibility or musical fidelity.

| Candidate | Schema | Pitch | Tracks | Events | Active s | Min repeat ms | Chord onset % | Max chord |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Nuvole Bianche | composed v3 | Ab | 2 | 1764 | 319.043 | 260 | 62.7% | 5 |
| Interstellar Main Theme | composed v3 | C | 1 | 1723 | 326.914 | 171 | 23.5% | 4 |
| Merry Christmas Mr. Lawrence | composed v3 | Db | 2 | 1924 | 272.143 | 142 | 68.9% | 5 |
| Comptine d'un autre été | composed v3 | G | 1 | 1108 | 152.941 | 147 | 36.8% | 4 |
| The Truth That You Leave | recorded v2 | C | 1 | 1756 | 212.976 | 232 | 67.6% | 5 |
| Mariage d'Amour | composed v3 | Bb | 1 | 1099 | 196.6 | 200 | 39.7% | 3 |
| Viva La Vida | composed v3 | Eb | 1 | 1307 | 162.565 | 141 | 73.1% | 5 |
| Hallelujah | composed v3 | C | 4 | 1241 | 280 | 333 | 38.8% | 7 |
| Kamado Tanjiro no Uta | composed v3 | F | 2 | 652 | 156.889 | 222 | 39.8% | 4 |
| Sweden | recorded v2 | D | 1 | 255 | 104.986 | 312 | 48.2% | 6 |

## Selection rationale

- **Nuvole Bianche:** Prefer San over SarVxV (longer, 6.4 s rest) and the thinner Kalimba arrangement.
- **Interstellar Main Theme:** Prefer balanced density and repeat margin over the Peter Buka MIDI-derived arrangement (34 ms repeat); other versions were shorter.
- **Merry Christmas Mr. Lawrence:** Prefer fuller chord texture over Sunny; winter piano version had a 20 ms repeat and 21 s rest; Ustinian is a short excerpt.
- **Comptine d'un autre été:** Prefer fuller arrangement over Cova, Cractycusly and the sparse recorded candidate; melodic fidelity still needs listening.
- **The Truth That You Leave:** Prefer 213 s chord-rich recording over the 44 s candidate.
- **Mariage d'Amour:** Prefer longer one-Piano arrangement over the 68 s ikina candidate.
- **Viva La Vida:** Strong rhythmic chord texture; other named variants were not exhaustively compared.
- **Hallelujah:** Prefer this long chorded version over the 38 s Haven recording; review four-track merge and seven-key chords.
- **Kamado Tanjiro no Uta:** Balanced candidate; alternate arrangements not exhaustively evaluated.
- **Sweden:** Chorded single-Piano candidate; alternate Sweden arrangements not exhaustively evaluated.

## Merge gates

- [x] Source JSON and structural pre-screen; upstream Git blob identity verified.
- [x] Register ten new opaque IDs and actual SHA-256 values; preserve existing active/retired identities.
- [ ] Run `cargo xtask builtin-catalog verify` and regenerate `cargo xtask song-audit inventory` (commit generated inventory.json, inventory.csv and inventory-summary.md). The existing 112-entry inventory remains a historical snapshot until regeneration.
- [ ] Inspect actual Rust planner diagnostics and runtime preparation at default timing and intended instrument pitch.
- [ ] Listen to the complete arrangements on a Sky-compatible piano; compare alternatives, particularly multi-track works and works with unexplored variants. Verify sections, tempo, musical fidelity and endings; reject or replace any weak candidates.
- [ ] Establish distribution rights for each composition and arrangement before merge/release. See [upstream README](https://github.com/Ai-Vonie/Sky1984-Sheets-Collection/blob/fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea/README.md).

No claims of listening review, legal clearance or successful Rust tests are made here. The environment used to prepare this PR had no Cargo toolchain and could not clone GitHub.
