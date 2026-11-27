# Built-in song inventory summary

- Active manifest entries audited: 153
- Manifest SHA-256: `fc3b24eb32c45301ec840dfb8fb2aed1a03cd9ef46b46fe8dcd9dc7ffe2239a3`
- Generated with: `cargo xtask song-audit inventory`
- Runtime/parser policy: existing source decoder, planner and frozen 60 FPS / one-frame / zero-margin policy.

## Schema counts

| Schema | Songs |
| --- | ---: |
| composed_v3 | 48 |
| legacy_flat | 94 |
| recorded_v2 | 11 |

## Validation coverage

- Strict source decoding: 153/153 passed.
- Planner status `ready`: 59.
- Planner status `ready_legacy_compatibility`: 94.

## Screening flags

These filters only surface measurements. They do not rate musical quality.

### Under 45 seconds (6)

- `sheets/Chúc bé ngủ ngon  (Minh Châu 2906).txt`
- `sheets/Memories-Maroon 5.json`
- `sheets/Senorita   (Snejok).skysheet`
- `sheets/take me to church.json`
- `sheets/We Wish You A Merry Christmas.json`
- `sheets/You are my Sunshine.json`

### 45–75 seconds (7)

- `sheets/Cheap Thrills.skysheet`
- `sheets/he's a pirate.json`
- `sheets/In The End.json`
- `sheets/Jingle Bells.json`
- `sheets/Ocean Eyes.json`
- `sheets/save your tear.json`
- `sheets/Something Just Like This.json`

### Below 2 authored onsets per active second (14)

- `sheets/easy on me.json`
- `sheets/Glimpse of Us.json`
- `sheets/Howl's Moving Castle v2.json`
- `sheets/In The End.json`
- `sheets/Kiss The Rain.json`
- `sheets/Let You Break My Heart Again.json`
- `sheets/Lovely-Billie Eilish ft. Khalid.json`
- `sheets/Nothing's New.json`
- `sheets/Say So - Doja Cat.json`
- `sheets/Somewhere Only We Know.json`
- `sheets/Sweden.json`
- `sheets/thần thoại OST.json`
- `sheets/To Zanarkand.json`
- `sheets/What Was I Made For.json`

### Maximum authored rest of at least 5 seconds (3)

- `sheets/KILL BILL.json`
- `sheets/Lovely-Billie Eilish ft. Khalid.json`
- `sheets/Renai Circulation.json`

## Physical-key collision screening

- Files with extra physical time/key events: 25
- Extra physical events after deduplication: 3435
- Of those, extras from same-source layer expansion: 419

- sheets/A Thousand Years.json: 39 extra physical event(s), 39 from same-source layer expansion
- sheets/All Of Me.json: 9 extra physical event(s), 0 from same-source layer expansion
- sheets/Alway With Me.json: 54 extra physical event(s), 0 from same-source layer expansion
- sheets/Blinding Lights.json: 204 extra physical event(s), 204 from same-source layer expansion
- sheets/Careless Whisper.json: 224 extra physical event(s), 0 from same-source layer expansion
- sheets/Comedy easy ver.json: 3 extra physical event(s), 0 from same-source layer expansion
- sheets/Dandelions.json: 13 extra physical event(s), 0 from same-source layer expansion
- sheets/Howl's Moving Castle.json: 447 extra physical event(s), 0 from same-source layer expansion
- sheets/I Love You So.json: 454 extra physical event(s), 0 from same-source layer expansion
- sheets/I'm yours.json: 22 extra physical event(s), 0 from same-source layer expansion
- sheets/KILL BILL.json: 1 extra physical event(s), 0 from same-source layer expansion
- sheets/Lemon.json: 690 extra physical event(s), 0 from same-source layer expansion
- sheets/Lily (uma).json: 96 extra physical event(s), 96 from same-source layer expansion
- sheets/Memories.json: 52 extra physical event(s), 0 from same-source layer expansion
- sheets/Ocean Eyes.json: 102 extra physical event(s), 0 from same-source layer expansion
- sheets/On My Way.json: 80 extra physical event(s), 80 from same-source layer expansion
- sheets/Path Of The Wind.json: 444 extra physical event(s), 0 from same-source layer expansion
- sheets/Play Date.json: 451 extra physical event(s), 0 from same-source layer expansion
- sheets/see u again.json: 7 extra physical event(s), 0 from same-source layer expansion
- sheets/Souvenir.json: 1 extra physical event(s), 0 from same-source layer expansion
- sheets/Stay - The Kid LAROI, Justin Bieber.json: 1 extra physical event(s), 0 from same-source layer expansion
- sheets/Summertime.json: 2 extra physical event(s), 0 from same-source layer expansion
- sheets/Way Back Home.json: 2 extra physical event(s), 0 from same-source layer expansion
- sheets/We Wish You A Merry Christmas.json: 19 extra physical event(s), 0 from same-source layer expansion
- sheets/Yoru ni kakeru_YOASOBI.json: 18 extra physical event(s), 0 from same-source layer expansion

See `README.md` for metric definitions and limitations.
