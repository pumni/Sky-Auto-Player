# PR #236 Player Modes — Actual Tauri Evidence

These captures were taken from the committed implementation at source SHA
`395a599743bb9b13193b4cf78f659addbb900673`, running as a real Windows Tauri/Wry
window with the native bridge. The screenshots were captured from that window's
WebView2 surface through its local debugging connection; they are not browser-shell
or mock-bridge screenshots. The client sizes below are physical pixels at 125% Windows
scale (`devicePixelRatio = 1.25`).

The Playing captures use the Playback Profile's **Test playback (no input)** action.
They demonstrate the rendered transport state only and do not claim SendInput or real
Sky playback qualification. Auto Play was toggled in the native-backed settings store;
quick-profile reopen and Settings/quick-profile mirroring were checked in the same run.

| Capture | State |
| --- | --- |
| `idle-shuffle-off-1200x760.png` | Idle, Shuffle Off, 1200×760 |
| `idle-shuffle-on-1200x760.png` | Idle, Shuffle On, 1200×760 |
| `playing-shuffle-on-1200x760.png` | Dry-run Playing, Shuffle On, 1200×760 |
| `quick-profile-auto-play-on-1200x760.png` | Quick profile, Auto Play On, 1200×760 |
| `quick-profile-auto-play-off-1200x760.png` | Quick profile, Auto Play Off, 1200×760 |
| `quick-profile-auto-play-on-1280x720.png` | Quick profile, Auto Play On, 1280×720 |
| `quick-profile-auto-play-off-1280x720.png` | Quick profile, Auto Play Off, 1280×720 |
| `settings-auto-play-on-1280x720.png` | Settings > Playback, Auto Play On, 1280×720 |
| `settings-auto-play-off-1280x720.png` | Settings > Playback, Auto Play Off, 1280×720 |
| `playing-shuffle-on-800x560.png` | Dry-run Playing, Shuffle On, compact 800×560 |
| `quick-profile-auto-play-on-800x560.png` | Quick profile, Auto Play On, compact 800×560 |
| `quick-profile-auto-play-off-800x560.png` | Quick profile, Auto Play Off, compact 800×560 |

At 1200×760 the Play/Pause center matched the WebView viewport center exactly and the
secondary controls were borderless with transparent backgrounds. At 800×560, the
logical viewport was 640×448; Play/Pause was 0.013 CSS px from its center axis, the
document had no horizontal overflow, and the 320×269 CSS px profile popover remained
inside the viewport. The profile popover also fit at 1280×720.

SHA-256 digests:

| File | SHA-256 |
| --- | --- |
| `idle-shuffle-off-1200x760.png` | `701CBD4468E64F24E7FE096F4107C21E5CF975B8AD78615EE2769952BFA6E03E` |
| `idle-shuffle-on-1200x760.png` | `F158F56113653DE35734F46DF6471F6D1E4168F23E7CE2014ABE16C3E2D3F93F` |
| `playing-shuffle-on-1200x760.png` | `56B6E06D5C3E85A881363C020822E7F67F6B40A8BB53C117CC8565DAC8522946` |
| `playing-shuffle-on-800x560.png` | `954D64418CEFBAFC965CFAD8CC8320D78AE98DDEEB8990BB947A75997FFB5550` |
| `quick-profile-auto-play-off-1200x760.png` | `CCF7B81C8D4818F13B567B931912882CE69ABAD2AD17FB4996A9802A1FAE9569` |
| `quick-profile-auto-play-off-1280x720.png` | `49057E06710DCAAD2B7728417B528BA0C922B0181DCD339F9C5AF0BDBCDD604D` |
| `quick-profile-auto-play-off-800x560.png` | `3751DE5A159FD0AAF7741A919147CC23814F2DDA2207866A26C60653638B0481` |
| `quick-profile-auto-play-on-1200x760.png` | `42B84B338A92CAB0155FBE5A728E9A261DDF5966C314BA5B3C7D97D88B709D6E` |
| `quick-profile-auto-play-on-1280x720.png` | `DE6DA1CBD766C4A031F7D76C147123E8EA24CCC181B2C558E42C8DC4F9C848A6` |
| `quick-profile-auto-play-on-800x560.png` | `ADFB6948EE0E0A4E1514995E44EA187A6FA7BEDA0CC999F66D6D71D191E40168` |
| `settings-auto-play-off-1280x720.png` | `3C57E4FEC27CDE5132C1E477109483839F7971131E532986EE63188B9588E889` |
| `settings-auto-play-on-1280x720.png` | `01258491BDDF6AA289B0AD9AAC78FB9BB47CF4AD76FC7313B3D1806EE4F519D7` |
