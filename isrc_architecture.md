# ISRC-Only Download Architecture

> **Rule**: Always download by ISRC. Never by title, artist, URL, or metadata.
> **Rule**: No fallback. If ISRC not on Deezer, record it. Don't search.
> **Rule**: Three runs. Track fluctuations. Top 3 unstable tracks get flagged.

---

## Why ISRC-only

ISRC is the universal recording identifier. Same ISRC = same recording on every
platform. Deezer, Spotify, Apple Music all use it. No ambiguity, no metadata
collisions (no "Fortuna instead of Gippeul"), no wrong-artist fallback bugs.

The current deemix flow (`generateDownloadObject` with `SpotifyPlugin`) does:
```
Spotify URL → Spotify API → ISRC → Deezer ISRC lookup → download
                                              │
                                     if fails: metadata search
                                              │
                                     if fails: bitrate fallback loop
```

The problem is steps 2 and 3 — they introduce ambiguity and bugs. By going
ISRC-only, we collapse to:

```
ISRC → deemix itemgen.generateTrackItem(dz, "isrc:XXXX") → download
```

If the ISRC isn't on Deezer: done. Record it. Move on. No fallback. No loops.

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                      wish (Rust)                         │
│                                                          │
│  Spotify API → resolve playlist tracks → ISRCs          │
│  Store: submissions with isrc column                     │
│                                                          │
│  For download: extract ISRC from submission               │
│              → pass to musdl                             │
└──────────────────────────┬───────────────────────────────┘
                           │
                  ISRC string (no URL, no title)
                           │
                           ▼
┌──────────────────────────────────────────────────────────┐
│                      musdl (TS)                          │
│                                                          │
│  downloadByISRC(isrc, bitrate, dest):                     │
│    dz.loginViaArl(arl)                                   │
│    obj = itemgen.generateTrackItem(dz, "isrc:"+isrc, 3)  │
│    downloader = new Downloader(dz, obj, settings, lsn)   │
│    await downloader.start()                              │
│    → { status, path, isrc, bitrate, error? }            │
│                                                          │
│  NO SpotifyPlugin. NO generateDownloadObject.            │
│  NO metadata search. NO fallback.                        │
│                                                          │
│  ISRC not on Deezer → Downloader throws                  │
│  ISRCnotOnDeezer → return { status: "not_on_deezer" }    │
└──────────────────────────────────────────────────────────┘
```

### Key API: `itemgen.generateTrackItem`

```ts
import { itemgen } from "deemix";

// Direct ISRC download — no resolution, no plugin, no search
const downloadObject = itemgen.generateTrackItem(
  dz,              // authenticated Deezer instance
  "isrc:USQX91300108",  // ISRC with "isrc:" prefix
  3                // bitrate (3 = MP3_320)
);
// Returns: Single (DownloadObject)
// Throws: ISRCnotOnDeezer if ISRC doesn't exist on Deezer
```

This is the same function `SpotifyPlugin` calls internally after ISRC resolution.
We just skip the Spotify API middleman.

---

## Test plan: 3-run stability

### Input
The 60 tracks from `playlist-tracks.json` (all have ISRC).

### Per-run process
```
for each track:
  isrc = track.isrc
  result = musdl.downloadByISRC(isrc)
  record: { isrc, artist, title, status, path?, error? }
```

### Run 3 times
Run 1 → results_run1.json
Run 2 → results_run2.json
Run 3 → results_run3.json

### Compare
Diff all three runs. Categorize:

| Category | Definition |
|----------|-----------|
| **Stable-OK** | Downloaded correctly all 3 runs |
| **Stable-FAIL** | Failed same way all 3 runs (not on Deezer, etc.) |
| **Fluctuating** | Different results between runs (sometimes OK, sometimes fail) |

### Output
Present:
1. Overall stats per run (X OK, Y fail, Z not-on-deezer)
2. Top 3 fluctuating tracks (most variance)
3. List of permanently unavailable ISRCs (for later processing)

---

## musdl changes needed

### New function: `downloadByISRC`

```ts
// In download.ts — alternative to downloadTrack()
export async function downloadByISRC(
  dz: Deezer,
  isrc: string,
  settings: Record<string, any>
): Promise<DownloadResult> {
  try {
    const obj = itemgen.generateTrackItem(dz, `isrc:${isrc}`, settings.maxBitrate ?? 3);
    const downloader = new Downloader(dz, obj, settings, {
      send: (_key: string, _data?: any) => {},
    });
    await downloader.start();
    
    // Extract result from downloadObject
    const file = obj.files?.[0];
    return {
      status: "done",
      path: file?.path,
      isrc,  // we already know the ISRC
      title: obj.title,
      artist: obj.artist,
    };
  } catch (e: any) {
    if (e?.name === "ISRCnotOnDeezer") {
      return { status: "not_on_deezer", error: "ISRC not found on Deezer" };
    }
    return { status: "failed", error: e?.message || String(e) };
  }
}
```

### New CLI entry: `musdl download-isrc <isrc>`

```
musdl download-isrc USQX91300108 --bitrate 320 --dest ./out --json
→ {"status":"done","path":"...","isrc":"USQX91300108",...}
```

---

## What happens to tracks not on Deezer

Record them. Per-ISRC failure is deterministic — same ISRC will always fail.
These go into a "pending" list. Later processing options:
- Submit to spotDL (YouTube-based fallback)
- Submit to yt-dlp
- Mark as permanently unavailable

But that's a separate cycle. First: establish the ISRC-only baseline.

---

## Implementation

Single step: rewrite `download.ts` to add `downloadByISRC()`, update `cli.ts`,
run 3-pass test on all 60 ISRCs, diff results.

No new packages. No SQLite (yet). No JSON output changes (already have --json).
Just the function + the test script.
