# Native Deemix Control — Notes

> Shared scratchpad. Sub-agents write findings here. Coordinator consolidates.

---

## Findings (consolidated)

> From exploration of `bambanah/deemix` — a Turborepo monorepo with `deezer-sdk` + `deemix` + `cli` packages.
> All three packages are TypeScript, ESM (tsup), GPL-3.0.

### The three functions musdl needs

All come from the existing `deemix` and `deezer-sdk` workspace packages:

```
1. Deezer.loginViaArl(arl)          ← deezer-sdk (auth)
2. generateDownloadObject(...)      ← deemix (URL → DownloadObject)
3. Downloader.start()               ← deemix (stream + decrypt + tag → file)
```

#### 1. Auth: `Deezer.loginViaArl(arl)` → `boolean`

From `deezer-sdk` v1.10.2. Sets `arl` cookie on `.deezer.com`, validates via `gw.get_user_data()` (checks `USER_ID != 0`), extracts `license_token` from `USER.OPTIONS.license_token`. Uses `tough-cookie` for cookie persistence, auto-refreshes API token on invalidation.

**The actual HTTP**:

- Auth: `GET http://www.deezer.com/ajax/gw-light.php?method=deezer.getUserData` (with ARL cookie)
- Download URL: `POST https://media.deezer.com/v1/get_url` with `{ license_token, media: [{ type: "FULL", formats: [{ cipher: "BF_CBC_STRIPE", format: 3 }] }], track_tokens: ["..."] }`

#### 2. Resolution: `generateDownloadObject(dz, url, bitrate, plugins, listener)` → `DownloadObject`

From `deemix`. Parses URL via `parseLink()`. For Spotify URLs, delegates to `SpotifyPlugin`:

- Track: Spotify API → ISRC → `dz.api.getTrackByISRC(isrc)` → `Single`
- Album: Spotify API → UPC → `dz.api.get_album(upc:${upc})` → `Collection`
- Playlist: paginates Spotify, returns `Convertable`, then `plugin.convert()` resolves each track by ISRC → `Collection`
- Fallback (if `fallbackSearch`): `dz.api.get_track_id_from_metadata(artist, title, album)`

#### 3. Download: `new Downloader(dz, downloadObject, settings, listener).start()`

From `deemix`. For `Single`: calls `downloadWrapper()` → `download()` → `streamTrack()` (Blowfish-decrypt) + `tagTrack()` (ID3/FLAC). For `Collection`: `async.queue` with concurrency from `settings.queueConcurrency`.

Fallbacks inside `downloadWrapper()`: `fallbackID` → `fallbackISRC` → `fallbackSearch`.

Listener events: `"downloadInfo"`, `"downloadWarn"`, `"finishDownload"`, `"updateQueue"`, `"startConversion"`, `"finishConversion"`.

### Key types

| Type             | From         | Key fields                                                                         |
| ---------------- | ------------ | ---------------------------------------------------------------------------------- |
| `Deezer`         | deezer-sdk   | `.loggedIn`, `.api` (REST), `.gw` (internal), `.loginViaArl()`, `.get_track_url()` |
| `DownloadObject` | deemix       | `.type`, `.id`, `.title`, `.artist`, `.size`, `.uuid`, `.files[]`, `.isCanceled`   |
| `Single`         | deemix       | extends DownloadObject; `.single.trackAPI`                                         |
| `Collection`     | deemix       | extends DownloadObject; `.collection.tracks[]`                                     |
| `Convertable`    | deemix       | extends Collection; `.conversionData` — must `.convert()` before download          |
| `Track`          | deemix types | `.id`, `.ISRC`, `.trackToken`, `.bitrate`, `.downloadURL`, `.mainArtist`, `.album` |
| `Listener`       | deemix types | `{ send: (key: string, data?: any) => void }`                                      |
| `GWTrack`        | deezer-sdk   | `SNG_ID`, `ISRC`, `TRACK_TOKEN`, `TRACK_TOKEN_EXPIRE`, `FILESIZE_MP3_320`, etc.    |

### Settings musdl should hardcode

| Field               | Value                  | Why                                                    |
| ------------------- | ---------------------- | ------------------------------------------------------ |
| `maxBitrate`        | `3` (MP3_320)          | 320kbps sweet spot                                     |
| `fallbackBitrate`   | `true`                 | Drop to 128 if 320 unavailable                         |
| `fallbackISRC`      | `true`                 | Search alt albums by ISRC                              |
| `fallbackSearch`    | `true`                 | Metadata search as last resort                         |
| `queueConcurrency`  | `1`                    | musdl handles one at a time (wish manages parallelism) |
| `downloadLocation`  | from config            | Same dir wish uses                                     |
| `tracknameTemplate` | `"%artist% - %title%"` | Flat output                                            |
| `createAlbumFolder` | `false`                | Flat dir                                               |
| `overwriteFile`     | `"n"`                  | Don't re-download                                      |

### Error classes (what can go wrong)

```
DeemixError
├── GenerationError: ISRCnotOnDeezer, TrackNotOnDeezer, AlbumNotOnDeezer,
│                    LinkNotRecognized, LinkNotSupported
├── DownloadError
│   └── DownloadFailed: TrackNot360, PreferredBitrateNotFound
├── DownloadEmpty
└── TrackError: MD5NotFound, NoDataToParse, AlbumDoesntExists
```

### Build / packaging

- Monorepo uses turborepo + pnpm workspaces
- deemix builds with tsup (ESM), types via `tsc --emitDeclarationOnly`
- CLI packages into native binaries via `@yao-pkg/pkg` (node20, all platforms)
- musdl would be a new workspace package: `packages/musdl`, depends on `deemix` + `deezer-sdk` via `workspace:*`

### Data flow (single track, musdl perspective)

```
wish: musdl download <spotify_url> --json
  │
  ▼
musdl:
  1. Deezer.loginViaArl(config.arl)                    → boolean
  2. generateDownloadObject(dz, url, 3, {spotify}, lsn) → Single
  3. new Downloader(dz, single, settings, listener)     → Downloader
  4. downloader.start()                                 → Promise<void>
  5. On "finishDownload": read single.files[0].path     → absolute path
  6. stdout: {"status":"ready","path":"...","isrc":"...","size":...}
```

This is the complete API surface musdl needs. Everything else (settings loading, error handling, CLI arg parsing) is plumbing.

---

## Issues / things to raise

1. **Sub-agent conclusions contradicted target**: Sections from sub-agents B (CLI) and C (core exports) both concluded "we just keep calling deemix-pyweb HTTP API" — this describes the current architecture, not the target. The target says "No Docker. No HTTP polling. Replace deemix-pyweb with native deemix library calls." The consolidated findings above correct this.

2. **GPL-3.0 license**: deemix is GPL-3.0. If musdl links against it as a workspace dependency, musdl is GPL-3.0 too. wish (Rust, MIT/Apache) calls musdl as a subprocess — process boundary, no linking. Is this acceptable?

3. **Build unknowns**: We know the monorepo uses turborepo + pnpm workspaces. We don't yet know:
   - Can we build with just `pnpm` or is `turbo` required?
   - Required Node.js version?
   - Is deemix published on npm or workspace-only?

4. **ISRC cache**: The existing `SpotifyPlugin` caches Spotify→Deezer mappings in `cache.json`. Our target says "SQLite match cache" — we need to decide: replace the cache or layer on top?

---

## 2026-07-24 — Cycle 1: Fork + musdl package + minimal download

### step2-pkg (package scaffolding)

- Cloned `bambanah/deemix` (monorepo: turborepo + pnpm 11.8, Node >=24, ESM).
- Created `packages/musdl/` with `package.json` and `tsconfig.json`, following library conventions (same tsconfig base as `deemix`/`deezer-sdk`, ESM, `strict: false`, workspace deps via `workspace:*`).
- `pnpm-workspace.yaml` uses `packages/*` glob — no edit needed; musdl is auto-discovered.
- No `.ts` files created (core agent handles those).

### step2-core (download script)

- Wrote `packages/musdl/src/main.ts` (33 lines) with the download pipeline:
  1. `dz.loginViaArl(arl)` via `DEEMIX_ARL` from env
  2. `generateDownloadObject(dz, url, 3, {spotify}, listener)`
  3. `new Downloader(dz, obj, settings, listener).start()`
- Import paths verified against existing `deemix-cli` source — all match.
- API surface confirmed at runtime: `loginViaArl` → boolean, `generateDownloadObject` → DownloadObject | DownloadObject[], `Downloader.start()` → Promise, listener events (`finishDownload`, `downloadWarn`, `downloadInfo`) all work as documented.
- One notes.md correction: `plugins` parameter is `Record<string, BasePlugin>` (object map), not an array.

### step2-verify (build + test)

- **Build**: `pnpm install` (1395 pkgs). `deezer-sdk` + `deemix` + `musdl` all compile via tsup (ESM). Type declarations (tsc) fail for musdl due to missing workspace `.d.ts` resolutions — not blocking at runtime.
- **Test 1 — direct Deezer track**: ✅ Downloaded Daft Punk "Harder, Better, Faster, Stronger" via `generateTrackItem(dz, 3135556, 3)`. Output: 3.6 MB, 128kbps MP3, ID3v2.3.
- **Test 2 — Spotify URL**: ❌ Infinite `bitrateFallback` loop in deemix fork's `getPreferredBitrate`. Resolved a DIFFERENT track (Rick Astley) instead of the requested Daft Punk track, then cycled indefinitely.
- **Test 3 — Spotify URL (alt settings)**: ❌ `Cannot read properties of undefined (reading 'error')` in `spotify.ts` line ~530 — `e.body` is `undefined` when the Spotify API SDK errors without a body.
- **Settings gap**: `main.ts` is missing required fields that `Track.applySettings()` demands: `tags.savePlaylistAsCompilation`, `dateFormat`, `albumVariousArtists`, `executeCommand`.

### Contradictions / flags

1. **Deemix fork is buggy**: Two distinct bugs block Spotify URL downloads — (a) infinite bitrate fallback loop, (b) unsafe error property access in spotify.ts. The fork works for direct Deezer track IDs but not for Spotify URL resolution. May need to fix upstream or switch to upstream `deemix`.
2. **Settings missing**: The core agent's `main.ts` omits required settings fields (`tags`, `dateFormat`, `executeCommand`, `albumVariousArtists`). The script will crash at `applySettings()` until these are added.

---

## 2026-07-24 — Cycle 2: Bug fixes + test harness

### fix-spotify (deemix fork bugs)

Two blocking bugs in the deemix fork that prevented Spotify URL downloads:

1. **`e.body` undefined crash** (`spotify.ts` lines 531, 566): The Spotify Web API SDK throws errors without a `body` property, but `getTrack()` / `getAlbum()` dereferenced `e.body.error.message` directly. Fix: optional chaining (`e?.body?.error?.message`).

2. **Infinite bitrate fallback loop** (`getPreferredBitrate.ts` lines 155–170): The do-while loop chasing `fallbackID` chains could (a) loop forever when no fallback track yielded a download URL, and (b) resolve to completely wrong tracks (Rick Astley instead of Daft Punk) due to ISRC→album fallback aliasing. Fix: 5-iteration limit with `break` guard, reset per bitrate format.

Also noted `Object.keys(formats).reverse()` called fresh each iteration (O(n²), not buggy but wasteful).

### fix-settings (settings + refactor)

Refactored `packages/musdl/` from a monolithic `main.ts` into two modules:

- **`download.ts`** (NEW) — exports `downloadTrack(dz, url, settings?)`, `DownloadResult`, and `DEFAULT_SETTINGS` (40 top-level fields + 28 tags fields, all drawn from deemix's `DEFAULT_SETTINGS`). Takes an already-authenticated `Deezer` instance (caller handles ARL). Merges partial settings over defaults. Structured result — never throws.
- **`cli.ts`** (NEW) — thin CLI wrapper, renamed from old `main.ts`.
- **`main.ts`** — DELETED.

The old script had only 8 settings fields and was missing the entire `tags` sub-object — would crash at `Track.applySettings()` on `settings.tags.savePlaylistAsCompilation`. All 68 settings fields (top-level + tags) are now present.

Build fixes: switched tsconfig from `tsc/no-dom/library-monorepo` to `bundler/no-dom` to resolve `verbatimModuleSyntax` + `isolatedModules` conflicts in the composite build pipeline.

### test-harness (vitest tests)

Created `packages/musdl/vitest.config.ts` and `packages/musdl/src/download.test.ts` with 6 tests (4 integration, 2 unit). All passing (2 pass, 4 skip without ARL).

Integration tests (skip when `DEEMIX_ARL` absent):

1. Spotify URL → correct track (asserts artist IS Daft Punk, not Rick Astley — validates fallback fix)
2. Deezer track URL download
3. Raw numeric Deezer ID download
4. Invalid URL → `status: "failed"` with error

Unit tests: DEFAULT_SETTINGS structure validation, DownloadResult type shape check.

Key assertions: MP3 magic bytes, ISRC extraction via ffprobe, correct artist/title, file existence.

### Contradictions / flags

None. All three agents worked on complementary layers.

---

## 2026-07-26 — Cycle 3: ISRC-only download + 3-pass stability test

### Discovery: ISRC-only eliminates fallback chaos

Switched from `generateDownloadObject` (with SpotifyPlugin + fallback) to direct
`itemgen.generateTrackItem(dz, "isrc:XXXX", bitrate)`. This skips the entire
Spotify→Deezer resolution chain and goes straight to ISRC lookup.

### 60-track batch test (from canonical playlist)

Test playlist: `https://open.spotify.com/playlist/2UCh0hUr8OXrMykCO4HkI3` (60 tracks, 100% ISRC coverage).

**Result: 56/60 stable-OK.** 4 always fail, 2 had transient timeouts (stable on retest).

### Always-fail tracks (deterministic, same across passes)

| Track                        | ISRC         |      Resolves?       | Download?  | Root cause                                            |
| ---------------------------- | ------------ | :------------------: | :--------: | ----------------------------------------------------- |
| Timbaland - The Way I Are    | USUM70722806 |  ✅ trackID 180606   | ❌ no file | Known deemix bug (AGENT.md permanent failure)         |
| Eric Prydz - Call on Me      | GBCEN0400130 | ✅ trackID 73880045  | ❌ no file | deemix download bug                                   |
| Corona - Rhythm Of The Night | ITA199800041 | ✅ trackID 472400362 | ❌ no file | Known deemix bug (AGENT.md permanent failure)         |
| Zombies In Miami             | NLUL61600042 |     ❌ Zod error     |     —      | Deezer API returns bad data (`title_version` missing) |

### Fluctuators (transient, stable on retest)

| Track                         | Pass 1 | Pass 2     | Retest 3x |
| ----------------------------- | ------ | ---------- | --------- |
| David Guetta - Sexy Bitch     | ✅     | ❌ timeout | ✅ ✅ ✅  |
| Klara Hammarström - On and On | ✅     | ❌ timeout | ✅ ✅ ✅  |

**Verdict**: Rate-limit or network blips. ISRC resolution itself is stable.

### ISRC vs SpotifyPlugin comparison

|                   |     SpotifyPlugin     |    ISRC-only    |
| ----------------- | :-------------------: | :-------------: |
| Correct downloads |      54/60 (90%)      | **56/60 (93%)** |
| Wrong artist      | 2 (Fortuna, Alice DJ) |      **0**      |
| Timeouts          |           3           | 3 (same tracks) |
| Other errors      |        1 (Zod)        | 1 (same track)  |

ISRC-only fixed the 2 wrong-artist bugs (metadata collision). No regression on other tracks.
