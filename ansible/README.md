# Ansible Deployment for Wish

One-command deployment of the entire Wish backend to projectmellon.de.

## Usage

```bash
# Install Ansible first
pip install ansible

# Deploy with secrets from environment
WISH_SPOTIFY_CLIENT_ID=your_spotify_client_id \
WISH_SPOTIFY_CLIENT_SECRET=your_spotify_client_secret \
WISH_DEEMIX_ARL=your_deezer_arl \
  ansible-playbook -i inventory.yml playbook.yml

# Redeploy just the binary (after code changes)
ansible-playbook -i inventory.yml playbook.yml --tags deploy

# Only update config
ansible-playbook -i inventory.yml playbook.yml --tags config
```

## What it sets up

| Component  | Port | Description                                               |
| ---------- | ---- | --------------------------------------------------------- |
| **wish**   | 8700 | Rust song request server (systemd)                        |
| **deemix** | 6595 | Deemix Docker container (320kbps MP3, Spotify plugin)     |
| **dufs**   | 8321 | Static file server for downloads (systemd)                |
| **Caddy**  | 443  | Reverse proxy (existing Docker container, routes updated) |

## Tags

- `deps` — system packages, yt-dlp, spotdl
- `deemix` — Docker container, config, ARL auth
- `wish` — Rust binary build, config, systemd
- `dufs` — file server
- `caddy` — reverse proxy routes
- `deploy` — redeploy source + rebuild binary
- `config` — update config files only
- `verify` — health check
