# Ansible Deployment for Wish

One-command deployment of the entire Wish backend to projectmellon.de.

## Usage

```bash
# Install Ansible first
pip install ansible

# Deploy with secrets from environment
WISH_SPOTIFY_CLIENT_ID=e7b09b7a085d4a029a1b454574ace53b \
WISH_SPOTIFY_CLIENT_SECRET=a637050f76fa4f15b24f23e672bc8a9b \
WISH_DEEMIX_ARL=1be8e7f970c14d422142026da4decece609e52b7cf95fd3e9ece087667893b0ec94f77ef8b5560b7aa4a2d6a0c031969a5ac94e797dec4984600e720973edcd149c5a0fb63bbb091392a951322be76da14c375ae3f6fc45b5daa84014f6e4fdb \
  ansible-playbook -i inventory.yml playbook.yml

# Redeploy just the binary (after code changes)
ansible-playbook -i inventory.yml playbook.yml --tags deploy

# Only update config
ansible-playbook -i inventory.yml playbook.yml --tags config
```

## What it sets up

| Component | Port | Description |
|---|---|---|
| **wish** | 8700 | Rust song request server (systemd) |
| **deemix** | 6595 | Deemix Docker container (320kbps MP3, Spotify plugin) |
| **dufs** | 8321 | Static file server for downloads (systemd) |
| **Caddy** | 443 | Reverse proxy (existing Docker container, routes updated) |

## Tags

- `deps` — system packages, yt-dlp, spotdl
- `deemix` — Docker container, config, ARL auth
- `wish` — Rust binary build, config, systemd
- `dufs` — file server
- `caddy` — reverse proxy routes
- `deploy` — redeploy source + rebuild binary
- `config` — update config files only
- `verify` — health check
