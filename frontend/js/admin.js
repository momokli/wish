var API = window.location.origin;
var filter = "all",
  sortCol = "id",
  sortDir = "desc",
  data = [],
  expandedSet = {};

function fmtsize(b) {
  if (!b) return "";
  if (b > 1e6) return (b / 1e6).toFixed(1) + " MB";
  if (b > 1024) return (b / 1024).toFixed(0) + " KB";
  return b + " B";
}

function viaLabel(err) {
  if (!err) return "";
  if (err.indexOf("downloaded via ") === 0)
    return '<span class="via success">' + esc(err.substring(14)) + "</span>";
  var s = err.substring(0, 60);
  return (
    '<span class="via error" title="' +
    escAttr(err) +
    '">' +
    esc(s) +
    (err.length > 60 ? "..." : "") +
    "</span>"
  );
}

document.querySelectorAll("[data-filter]").forEach(function (b) {
  b.addEventListener("click", function () {
    document.querySelectorAll("[data-filter]").forEach(function (x) {
      x.classList.remove("active");
    });
    b.classList.add("active");
    filter = b.dataset.filter;
    render();
  });
});

document.querySelectorAll("[data-sort]").forEach(function (th) {
  th.addEventListener("click", function () {
    document.querySelectorAll("th").forEach(function (x) {
      x.classList.remove("sorted");
    });
    th.classList.add("sorted");
    var col = th.dataset.sort;
    if (sortCol === col) sortDir = sortDir === "asc" ? "desc" : "asc";
    else {
      sortCol = col;
      sortDir = "asc";
    }
    render();
  });
});

function render() {
  // Only restore rows the USER explicitly expanded. All logs closed by default.
  var restoreIds = Object.assign({}, expandedSet);

  var filtered = data;
  if (filter !== "all") {
    if (filter === "pending")
      filtered = data.filter(function (r) {
        return r.status !== "ready" && r.status !== "failed";
      });
    else
      filtered = data.filter(function (r) {
        return r.status === filter;
      });
  }
  filtered.sort(function (a, b) {
    var va = a[sortCol] || "",
      vb = b[sortCol] || "";
    if (typeof va === "number") return sortDir === "asc" ? va - vb : vb - va;
    return sortDir === "asc"
      ? String(va).localeCompare(String(vb))
      : String(vb).localeCompare(String(va));
  });

  // Compute average time-to-ready for last 5 tracks
  var readyTimes = [];
  for (var i = 0; i < filtered.length; i++) {
    var r = filtered[i];
    if (r.status === "ready" && r.first_available_at && r.created_at) {
      readyTimes.push(r.first_available_at - r.created_at);
    }
    if (readyTimes.length >= 5) break;
  }
  var avgEl = document.getElementById("avg-time");
  if (avgEl) {
    if (readyTimes.length > 0) {
      var avgSecs = Math.round(
        readyTimes.reduce(function (a, b) {
          return a + b;
        }) / readyTimes.length,
      );
      var avgStr =
        avgSecs < 60
          ? avgSecs + "s"
          : Math.floor(avgSecs / 60) + "m " + (avgSecs % 60) + "s";
      avgEl.textContent = "avg: " + avgStr + " (last " + readyTimes.length + ")";
    } else {
      avgEl.textContent = "";
    }
  }

  var h = "";
  for (var i = 0; i < filtered.length; i++) {
    var r = filtered[i],
      s = r.status || "pending",
      sc = s.replace(/_/g, " ");
    var title = r.track_title || cleanUrl(r.spotify_url || "", r.source || "");
    var logs = [];
    try {
      if (r.attempts_json)
        logs = JSON.parse(r.attempts_json).map(function (e) {
          return typeof e === "string" ? JSON.parse(e) : e;
        });
    } catch (e) {}

    var cont = r.container || "";
    if (!cont && r.filename) {
      var ext = r.filename.split(".").pop();
      if (ext) cont = ext.toLowerCase();
    }
    if (!cont && logs.length) {
      for (var k = logs.length - 1; k >= 0; k--) {
        if (logs[k].container) {
          cont = logs[k].container;
          break;
        }
      }
    }

    var via = viaLabel(r.error_message);

    var elapsed = "";
    if (r.first_available_at && r.created_at && r.status === "ready") {
      var secs = r.first_available_at - r.created_at;
      if (secs < 60) elapsed = secs + "s";
      else if (secs < 3600) elapsed = Math.floor(secs / 60) + "m " + (secs % 60) + "s";
      else
        elapsed = Math.floor(secs / 3600) + "h " + Math.floor((secs % 3600) / 60) + "m";
    }

    var rowId = "row-" + r.id;
    h +=
      '<tr id="' +
      rowId +
      '">' +
      "<td>" +
      r.id +
      "</td>" +
      "<td>" +
      esc(title) +
      "</td>" +
      "<td>" +
      esc(r.track_artist || "") +
      "</td>" +
      "<td>" +
      esc(r.source || "") +
      "</td>" +
      '<td><span class="status ' +
      s +
      '">' +
      sc +
      "</span></td>" +
      "<td>" +
      esc(cont) +
      "</td>" +
      "<td>" +
      fmtsize(r.file_size) +
      "</td>" +
      "<td>" +
      elapsed +
      "</td>" +
      '<td><a href="/downloads/"' +
      encodeURIComponent(r.filename || "") +
      '">' +
      esc((r.filename || "").substring(0, 35)) +
      "</a></td>" +
      "<td>" +
      via +
      "</td>" +
      "<td>" +
      (logs.length
        ? '<button class="logs-btn" onclick="toggleLogs(event,' +
          r.id +
          ')">show logs (' +
          logs.length +
          ")</button>"
        : '<span style="color:var(--muted)">none</span>') +
      "</td>" +
      "</tr>";

    if (logs.length) {
      h +=
        '<tr class="logs-row" id="logs-' +
        r.id +
        '"><td colspan="11"><div class="logs-cell">' +
        '<div class="logs-title">Pipeline log — ' +
        logs.length +
        " attempt(s)</div>" +
        '<table class="log-table">' +
        '<tr><td class="log-ts" style="color:var(--muted)">Time</td><td class="log-layer" style="color:var(--muted)">Layer</td><td style="color:var(--muted)">Result</td><td style="color:var(--muted)">Details</td></tr>';
      for (var j = 0; j < logs.length; j++) {
        var l = logs[j];
        h +=
          "<tr>" +
          '<td class="log-ts">' +
          esc(l.ts || "") +
          "</td>" +
          '<td class="log-layer">' +
          esc(l.layer) +
          "</td>" +
          "<td>" +
          (l.ok
            ? '<span class="log-ok">OK</span>'
            : l.layer === "all"
              ? '<span class="log-fail">FAIL</span>'
              : '<span style="color:var(--amber)">&hellip;</span>') +
          "</td>" +
          "<td>" +
          (l.ok
            ? '<span class="log-file">' +
              esc(l.file || "") +
              "</span>" +
              (l.bitrate
                ? ' <span class="log-detail">[' + esc(l.bitrate) + "]</span>"
                : "") +
              (l.container
                ? ' <span class="log-detail">.' + esc(l.container) + "</span>"
                : "")
            : '<span class="log-detail">' + esc(l.error || "") + "</span>") +
          "</td>" +
          "</tr>";
      }
      h += "</table></div></td></tr>";
    }
  }
  document.getElementById("tbody").innerHTML =
    h || '<tr><td colspan="11" class="empty">No submissions found</td></tr>';

  // Restore only user-expanded rows
  for (var rid in restoreIds) {
    var row = document.getElementById("row-" + rid);
    var btn = row && row.querySelector(".logs-btn");
    if (row) {
      row.classList.add("expanded");
    }
    if (btn) {
      btn.classList.add("open");
    }
  }
}

window.toggleLogs = function (e, id) {
  e.stopPropagation();
  var row = document.getElementById("row-" + id);
  var btn = e.target;
  if (row.classList.contains("expanded")) {
    row.classList.remove("expanded");
    btn.classList.remove("open");
    delete expandedSet[id];
  } else {
    // Close others
    Object.keys(expandedSet).forEach(function (k) {
      var r = document.getElementById("row-" + k);
      var b = r && r.querySelector(".logs-btn");
      if (r) r.classList.remove("expanded");
      if (b) b.classList.remove("open");
    });
    expandedSet = {};
    row.classList.add("expanded");
    btn.classList.add("open");
    expandedSet[id] = true;
  }
};

function fetchData() {
  fetch(API + "/admin/data")
    .then(function (r) {
      if (!r.ok)
        return r.json().then(function (e) {
          throw new Error(e.error || e.message || "Request failed");
        });
      return r.json();
    })
    .then(function (d) {
      data = d;
      render();
      document.getElementById("refresh").textContent =
        "auto 10s · " + data.length + " rows";
    })
    .catch(function () {});
}

function esc(s) {
  var d = document.createElement("div");
  d.textContent = (s || "").toString();
  return d.innerHTML;
}
function escAttr(s) {
  return (s || "")
    .toString()
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function playlistTitle(url, source) {
  if (!url) return "Playlist";
  var m = url.match(/spotify\.com\/playlist\/([^/?]+)/);
  if (m) return "Spotify playlist " + m[1].substring(0, 12) + "…";
  m = url.match(/[?&]list=([^&]+)/);
  if (m) return "YT playlist " + m[1].substring(0, 12) + "…";
  m = url.match(/soundcloud\.com\/.+\/sets\/([^/?]+)/);
  if (m) return "SC: " + decodeURIComponent(m[1]).replace(/[-_]/g, " ");
  return "Playlist";
}

function cleanUrl(url, source) {
  if (!url) return "?";
  if (url.indexOf("soundcloud.com/") !== -1) {
    var p = url.replace(/\?.*$/, "").split("/");
    return p[p.length - 1].replace(/-/g, " ").replace(/_/g, " ");
  }
  if (url.indexOf("youtube.com/") !== -1 || url.indexOf("youtu.be/") !== -1) {
    var m = url.match(/[?&]v=([^&]+)/) || url.match(/youtu\.be\/([^?&]+)/);
    if (m) return "YouTube: " + m[1].substring(0, 11);
    return "YouTube track";
  }
  if (url.indexOf("spotify:track:") === 0) return "Spotify track";
  return url.length > 40 ? url.substring(0, 37) + "..." : url;
}

// ── Playlists ──
function fetchPlaylists() {
  // Save currently expanded playlist rows
  var prevExpanded = {};
  document.querySelectorAll("tr.expanded").forEach(function (r) {
    var mid = r.id.replace("pl-row-", "");
    if (mid) prevExpanded[mid] = true;
  });

  fetch(API + "/playlists")
    .then(function (r) {
      if (!r.ok)
        return r.json().then(function (e) {
          throw new Error(e.error || e.message || "Request failed");
        });
      return r.json();
    })
    .then(function (d) {
      var h =
        "<table><thead><tr>" +
        "<th>#</th><th>Title</th><th>URL</th><th>Source</th>" +
        "<th>Tracks</th><th>New</th><th>Last synced</th><th>Logs</th><th>Actions</th>" +
        "</tr></thead><tbody>";
      for (var i = 0; i < d.length; i++) {
        var p = d[i];
        var logs = [];
        try {
          if (p.attempts_json)
            logs = JSON.parse(p.attempts_json).map(function (e) {
              return typeof e === "string" ? JSON.parse(e) : e;
            });
        } catch (e) {}
        var rel = p.last_synced ? relTime(p.last_synced) : "never";
        h +=
          '<tr id="pl-row-' +
          p.id +
          '">' +
          "<td>" +
          p.id +
          "</td>" +
          "<td>" +
          esc(p.title || playlistTitle(p.url, p.source)) +
          "</td>" +
          '<td><a href="' +
          escAttr(p.url) +
          '" target="_blank" style="color:var(--muted);font-size:11px;">' +
          esc(p.url.substring(0, 40)) +
          "...</a></td>" +
          "<td>" +
          esc(p.source) +
          "</td>" +
          "<td>" +
          p.track_count +
          "</td>" +
          "<td>" +
          (p.new_since_sync > 0
            ? '<span class="new-badge">+' + p.new_since_sync + "</span>"
            : "0") +
          "</td>" +
          '<td style="font-size:11px;color:var(--muted)">' +
          rel +
          "</td>" +
          "<td>" +
          (logs.length
            ? '<button class="logs-btn" onclick="togglePlaylistLogs(event,' +
              p.id +
              ')\">show logs (' +
              logs.length +
              ")</button>"
            : '<span style="color:var(--muted)">none</span>') +
          "</td>" +
          '<td><button class="logs-btn" onclick="syncPlaylist(' +
          p.id +
          ')\">sync</button></td>' +
          "</tr>";

        if (logs.length) {
          h +=
            '<tr class="logs-row" id="pl-logs-' +
            p.id +
            '\"><td colspan="9"><div class="logs-cell">' +
            '<div class="logs-title">Sync log — ' +
            logs.length +
            " attempt(s)</div>" +
            '<table class="log-table">' +
            '<tr><td class="log-ts">Time</td><td>Result</td><td>Tracks</td><td>New</td><td>Error</td></tr>';
          for (var j = 0; j < logs.length; j++) {
            var l = logs[j];
            h +=
              "<tr>" +
              '<td class="log-ts">' +
              esc(l.ts || "") +
              "</td>" +
              "<td>" +
              (l.ok
                ? '<span class="log-ok">OK</span>'
                : '<span class="log-fail">FAIL</span>') +
              "</td>" +
              "<td>" +
              (l.track_count || 0) +
              "</td>" +
              "<td>" +
              (l.new_count || 0) +
              "</td>" +
              '<td><span class="log-detail">' +
              esc(l.error || "") +
              "</span></td>" +
              "</tr>";
          }
          h += "</table></div></td></tr>";
        }
      }
      h += "</tbody></table>";
      document.getElementById("playlist-content").innerHTML =
        h || '<div class="empty">No playlists subscribed yet</div>';
      document.getElementById("playlist-refresh").textContent = d.length + " playlist(s)";

      // Restore expanded playlist rows
      for (var rid in prevExpanded) {
        var row = document.getElementById("pl-row-" + rid);
        var btn = row && row.querySelector(".logs-btn");
        if (row) row.classList.add("expanded");
        if (btn) btn.classList.add("open");
      }
    })
    .catch(function () {});
}

window.togglePlaylistLogs = function (e, id) {
  e.stopPropagation();
  var row = document.getElementById("pl-row-" + id);
  var btn = e.target;
  if (row.classList.contains("expanded")) {
    row.classList.remove("expanded");
    btn.classList.remove("open");
  } else {
    document.querySelectorAll("tr.expanded").forEach(function (r) {
      r.classList.remove("expanded");
    });
    document.querySelectorAll(".logs-btn.open").forEach(function (b) {
      b.classList.remove("open");
    });
    row.classList.add("expanded");
    btn.classList.add("open");
  }
};

window.syncPlaylist = function (id) {
  fetch(API + "/playlists/" + id + "/sync", { method: "POST" })
    .then(function (r) {
      if (!r.ok)
        return r.json().then(function (e) {
          throw new Error(e.error || e.message || "Sync failed");
        });
      fetchPlaylists();
    })
    .catch(function () {
      fetchPlaylists();
    });
};

window.syncAllPlaylists = function () {
  fetch(API + "/playlists")
    .then(function (r) {
      if (!r.ok)
        return r.json().then(function (e) {
          throw new Error(e.error || e.message || "Request failed");
        });
      return r.json();
    })
    .then(function (ps) {
      var promises = ps.map(function (p) {
        return fetch(API + "/playlists/" + p.id + "/sync", {
          method: "POST",
        });
      });
      Promise.all(promises).then(function () {
        fetchPlaylists();
      });
    })
    .catch(function () {
      fetchPlaylists();
    });
};

function relTime(ts) {
  var diff = Math.floor(Date.now() / 1000) - ts;
  if (diff < 60) return "just now";
  if (diff < 3600) return Math.floor(diff / 60) + "m ago";
  if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
  return Math.floor(diff / 86400) + "d ago";
}

fetchData();
setInterval(fetchData, 10000);
fetchPlaylists();
setInterval(fetchPlaylists, 10000);
