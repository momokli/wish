// Playlist functions for the guest app (index.html).
// Depends on window._w from app.js for shared utilities.
(function () {
    var _w = window._w;
    if (!_w) {
        console.error("playlists.js: window._w not found — app.js must load first");
        return;
    }

    var API = _w.API;
    var escHtml = _w.escHtml;
    var shortError = _w.shortError;
    var playlistTitle = _w.playlistTitle;
    var formatLastSynced = _w.formatLastSynced;
    var showToast = _w.showToast;

    // ── Playlists ──
    function fetchPlaylists() {
        fetch(API + "/playlists")
            .then(function (r) {
                if (!r.ok) return r.json().then(function(e) { throw new Error(e.error || e.message || "Request failed"); });
                return r.json();
            })
            .then(function (data) {
                var badge = document.getElementById("playlist-badge");
                badge.textContent = data.length || 0;
                if (data.length > 0) badge.classList.add("visible");
                else badge.classList.remove("visible");
                renderPlaylists(data);
            })
            .catch(function () {
                document.getElementById("playlist-list").innerHTML =
                    '<div class="empty-state">Failed to load playlists.</div>';
            });
    }

    function renderPlaylists(playlists) {
        var list = document.getElementById("playlist-list");
        if (!playlists || !playlists.length) {
            list.innerHTML =
                '<div class="empty-state">No playlists added yet. Paste a playlist URL above to get started.</div>';
            return;
        }
        var sourceSvgs = {
            spotify:
                '<svg width="14" height="14" viewBox="0 0 24 24" fill="#1DB954"><path d="M12 0C5.4 0 0 5.4 0 12s5.4 12 12 12 12-5.4 12-12S18.66 0 12 0zm5.521 17.34c-.24.359-.66.48-1.021.24-2.82-1.74-6.36-2.101-10.561-1.141-.418.122-.779-.179-.899-.539-.12-.421.18-.78.54-.9 4.56-1.021 8.52-.6 11.64 1.32.42.18.479.659.301 1.02zm1.44-3.3c-.301.42-.841.6-1.262.3-3.239-1.98-8.159-2.58-11.939-1.38-.479.12-1.02-.12-1.14-.6-.12-.48.12-1.021.6-1.141C9.6 9.9 15 10.561 18.72 12.84c.361.181.54.78.241 1.2zm.12-3.36C15.24 8.4 8.82 8.16 5.16 9.301c-.6.179-1.2-.181-1.38-.721-.18-.601.18-1.2.72-1.381 4.26-1.26 11.28-1.02 15.721 1.621.539.3.719 1.02.419 1.56-.299.421-1.019.599-1.559.3z"/></svg>',
            youtube:
                '<svg width="16" height="12" viewBox="0 0 24 24" fill="#FF0000"><path d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z"/></svg>',
            soundcloud:
                '<svg width="16" height="10" viewBox="0 0 24 24" fill="#FC3D0A"><path d="M1.175 12.475l.005 2.893-.005.024c-.005.022-.027.205-.027.205.15 2.38 2.103 4.261 4.513 4.316l11.365-.003c3.822-.178 6.825-3.347 6.834-7.202.009-3.872-3.015-7.024-6.868-7.024-.556 0-1.097.067-1.617.195-.614-2.273-2.658-3.91-5.057-3.879-2.832.034-5.125 2.338-5.143 5.187v3.388zm2.934-.153l-.005 2.618c0 .052.042.094.094.094h.147a.094.094 0 0 0 .094-.094l.005-2.618a.094.094 0 0 0-.094-.094h-.147a.094.094 0 0 0-.094.094zm2.159-1.865l-.009 4.483c0 .052.042.094.094.094h.147a.094.094 0 0 0 .094-.094l.009-4.483a.094.094 0 0 0-.094-.094h-.147a.094.094 0 0 0-.094.094z"/></svg>',
        };
        var html = "";
        for (var i = 0; i < playlists.length; i++) {
            var p = playlists[i];
            var src = p.source || "spotify";
            var srcIcon = sourceSvgs[src] || sourceSvgs.spotify;
            var newBadge =
                p.new_since_sync > 0
                    ? '<span style="color:var(--want-btn-text)">' +
                      p.new_since_sync +
                      " new</span> · "
                    : "";
            var errorHtml = p.last_error
                ? '<div style="font-size:10px;color:var(--status-failed);margin-top:4px;">' +
                  escHtml(shortError(p.last_error, p.source)) +
                  "</div>"
                : "";
            html +=
                '<div class="playlist-card" data-id="' +
                p.id +
                '">' +
                '<div class="card-main" style="height:auto;padding:16px;flex-direction:column;align-items:stretch;cursor:default;">' +
                '<div style="display:flex;align-items:center;gap:8px;margin-bottom:8px;">' +
                srcIcon +
                '<span style="flex:1;font-size:15px;color:#fff;">' +
                escHtml(p.title || playlistTitle(p.url, p.source)) +
                "</span>" +
                '<button class="sync-btn" onclick="syncPlaylist('
                p.id +
                ')" style="font-size:10px;">sync</button>' +
                '<button onclick="deletePlaylist(' +
                p.id +
                ')" style="background:none;border:none;color:var(--text-muted);cursor:pointer;font-size:16px;">×</button>' +
                "</div>" +
                '<div class="pl-counts" style="font-family:var(--font-mono);font-size:11px;color:var(--text-muted);">' +
                (p.track_count || 0) +
                " tracks · " +
                newBadge +
                formatLastSynced(p.last_synced) +
                "</div>" +
                errorHtml +
                "</div></div>";
        }
        list.innerHTML = html;
    }

    function addPlaylist() {
        var input = document.getElementById("playlistUrlInput");
        var url = input.value.trim();
        if (!url) return;
        var btn = document.getElementById("addPlaylistBtn");
        btn.disabled = true;
        btn.textContent = "⋯";
        var list = document.getElementById("playlist-list");
        var skeleton = document.createElement("div");
        skeleton.className = "playlist-card";
        skeleton.innerHTML = '<div class="card-main" style="padding:16px;color:var(--text-muted);font-style:italic;">⋯ adding playlist…</div>';
        list.insertBefore(skeleton, list.firstChild);
        input.value = "";
        fetch(API + "/playlists", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ url: url }),
        })
            .then(function (r) {
                if (!r.ok) return r.json().then(function(e) { throw new Error(e.error || "Failed"); });
                return r.json();
            })
            .then(function () {
                skeleton.remove();
                fetchPlaylists();
                showToast("Playlist added!");
            })
            .catch(function (err) {
                skeleton.remove();
                fetchPlaylists();
                showToast(err.message || "Failed to add playlist", "error");
            })
            .finally(function () {
                btn.disabled = false;
                btn.textContent = "+ Add";
            });
    }
    window.addPlaylist = addPlaylist;

    window.syncPlaylist = function(id) {
        var card = document.querySelector('.playlist-card[data-id="' + id + '"]');
        if (!card) return;
        var btn = card.querySelector('.sync-btn');
        if (btn.disabled) return;
        btn.disabled = true;
        btn.textContent = "⋯";
        var meta = card.querySelector('.pl-counts');
        var origText = meta ? meta.textContent : "";
        if (meta) meta.textContent = "syncing…";
        fetch(API + "/playlists/" + id + "/sync", { method: "POST" })
            .then(function(r) {
                if (!r.ok) throw new Error("Sync failed");
                return r.json();
            })
            .then(function() {
                fetchPlaylists();
                showToast("Synced!");
            })
            .catch(function(err) {
                fetchPlaylists();
                showToast("Sync failed: " + (err.message || "unknown"), "error");
            });
    };

    function deletePlaylist(id) {
        if (!confirm("Remove this playlist?")) return;
        fetch(API + "/playlists/" + id, { method: "DELETE" })
            .then(function (r) {
                if (!r.ok) throw new Error("Delete failed");
                fetchPlaylists();
                showToast("Playlist removed");
            })
            .catch(function (err) {
                showToast(
                    "Delete failed: " + (err.message || "unknown error"),
                    "error",
                );
            });
    }
    window.deletePlaylist = deletePlaylist;

    // Expose fetchPlaylists globally so app.js setInterval can call it
    window.fetchPlaylists = fetchPlaylists;

    fetchPlaylists();
})();
