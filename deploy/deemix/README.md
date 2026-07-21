# Deemix for wish

Docker deployment of [deemix](https://deemix.app/) as the L3 quality-upgrade layer
for Spotify downloads in the wish pipeline.

## Production (projectmellon.de)

```bash
cd /home/momo/wish/deploy/deemix
docker compose up -d
```

Shares `/opt/download-service/downloads` with wish and dufs.

## Local dev

```bash
cd deploy/deemix
docker compose -f docker-compose.local.yml up -d
```

Uses `./downloads/` for local testing.

## Setup

After first start:

1. **Configure Spotify plugin** via UI at `http://localhost:6595`:
   - Settings → Plugins → Spotify → enter client ID + secret

2. **Inject ARL** (Deezer auth token, get from browser cookies at deezer.com):
   ```bash
   curl -X POST http://localhost:6595/api/loginArl \
     -H 'Content-Type: application/json' \
     -d '{"arl":"YOUR_ARL_HERE"}'
   ```

3. **Verify**:
   ```bash
   curl http://localhost:6595/api/getQueue
   ```

## Config key settings

| Setting | Value | Why |
|---|---|---|
| `maxBitrate: "3"` | 320kbps MP3 | prefer speed over FLAC file size |
| `fallbackBitrate: true` | auto-downgrade | if 320 not available, get best possible |
| `fallbackISRC: true` | cross-reference | find track even if Deezer ID differs |
| `spotifyPlugin: true` | accept Spotify URLs | enables `addToQueue` with Spotify links |
| `overwriteFile: "n"` | never overwrite | dupes auto-skipped, no wasted downloads |
