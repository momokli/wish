(function () {
  var API = window.location.origin;
  var EVENT = localStorage.getItem("wish_event_name") || "wish";

  document.getElementById("event-name").textContent = EVENT;
  document.title = EVENT;

  // ── State ──
  var state = {
    sources: { spotify: true, youtube: true, soundcloud: true },
    query: "",

    searchTimeout: null,
    abortControllers: {},
    cache: null,
    CACHE_TTL: 5 * 60 * 1000,
  };

  var DOM = {
    searchInput: document.getElementById("searchInput"),
    resultsContainer: document.getElementById("results-container"),
    pills: document.querySelectorAll(".filter-pill"),
    searchView: document.getElementById("view-search"),
    searchWrapper: document.getElementById("searchWrapper"),
    queueBadge: document.getElementById("queue-badge"),
  };

  // ── Filter Pills — toggle visibility, never re-search ──
  DOM.pills.forEach(function (pill) {
    pill.addEventListener("click", function () {
      var src = pill.dataset.source;
      state.sources[src] = !state.sources[src];
      pill.classList.toggle("off", !state.sources[src]);
      renderAllFromCache();
    });
  });

  // ── Search Input ──
  DOM.searchInput.addEventListener("input", function (e) {
    state.query = e.target.value.trim();
    clearTimeout(state.searchTimeout);
    if (state.query.length < 2) {
      DOM.resultsContainer.innerHTML = "";
      updateCounts();
      return;
    }
    state.searchTimeout = setTimeout(triggerSearch, 400);
  });

  // ── Scroll shadow ──
  DOM.searchView.addEventListener("scroll", function () {
    if (DOM.searchView.scrollTop > 10) {
      DOM.searchWrapper.classList.add("scrolled");
    } else {
      DOM.searchWrapper.classList.remove("scrolled");
    }
  });

  function triggerSearch() {
    var q = state.query;
    if (
      state.cache &&
      state.cache.query === q &&
      Date.now() - state.cache.ts < state.CACHE_TTL
    ) {
      renderAllFromCache();
      return;
    }
    state.cache = { query: q, results: {}, ts: Date.now() };
    DOM.resultsContainer.innerHTML = "";
    var ALL_SOURCES = ["spotify", "youtube", "soundcloud"];
    ALL_SOURCES.forEach(function (source) {
      if (state.abortControllers[source]) state.abortControllers[source].abort();
      var controller = new AbortController();
      state.abortControllers[source] = controller;
      if (state.sources[source]) {
        var skeletonHtml =
          '<div class="source-section" id="section-' +
          source +
          '">' +
          '<div class="section-header">' +
          source.toUpperCase() +
          "</div>" +
          createSkeletonCard() +
          createSkeletonCard() +
          createSkeletonCard() +
          "</div>";
        DOM.resultsContainer.insertAdjacentHTML("beforeend", skeletonHtml);
      }
      fetchDataForSource(source, q, controller.signal);
    });
  }

  function fetchDataForSource(source, query, signal) {
    fetch(API + "/search?q=" + encodeURIComponent(query) + "&limit=8&source=" + source, {
      signal: signal,
    })
      .then(function (res) {
        if (!res.ok)
          return res.json().then(function (e) {
            throw new Error(e.error || e.message || "Request failed");
          });
        return res.json();
      })
      .then(function (data) {
        if (state.cache && state.cache.query === query)
          state.cache.results[source] = data.results || [];
        if (state.sources[source]) renderResultsForSource(source, data.results);
        updatePillCount(source, (data.results || []).length);
      })
      .catch(function (err) {
        if (err.name !== "AbortError") {
          if (state.cache && state.cache.query === query)
            state.cache.results[source] = null;
          if (state.sources[source]) renderResultsForSource(source, [], true);
          updatePillCount(source, 0);
        }
      });
  }

  function createSkeletonCard() {
    return '<div class="skeleton-card"><div class="skeleton-img"></div><div class="skeleton-text-container"><div class="skeleton-text-1"></div><div class="skeleton-text-2"></div></div><div class="skeleton-btn"></div></div>';
  }

  function renderAllFromCache() {
    if (!state.cache) return;
    DOM.resultsContainer.innerHTML = "";
    var ALL_SOURCES = ["spotify", "youtube", "soundcloud"];
    var anyVisible = false;
    ALL_SOURCES.forEach(function (source) {
      if (!state.sources[source]) return;
      anyVisible = true;
      var results = state.cache.results[source];
      if (results === undefined) {
        var skel =
          '<div class="source-section" id="section-' +
          source +
          '">' +
          '<div class="section-header">' +
          source.toUpperCase() +
          "</div>" +
          createSkeletonCard() +
          createSkeletonCard() +
          createSkeletonCard() +
          "</div>";
        DOM.resultsContainer.insertAdjacentHTML("beforeend", skel);
        return;
      }
      if (results === null) {
        var errHtml =
          '<div class="source-section" id="section-' +
          source +
          '">' +
          '<div class="section-header">' +
          source.toUpperCase() +
          " · 0</div>" +
          '<div class="empty-state" style="padding:20px;">No results found on ' +
          source +
          ".</div></div>";
        DOM.resultsContainer.insertAdjacentHTML("beforeend", errHtml);
        return;
      }
      var sec = document.createElement("div");
      sec.className = "source-section";
      sec.id = "section-" + source;
      sec.innerHTML =
        '<div class="section-header">' +
        source.toUpperCase() +
        " · " +
        results.length +
        "</div>";
      for (var i = 0; i < results.length; i++) {
        var track = results[i];
        var trackUrl = track.spotifyUrl || track.sourceUrl || "";
        var dur = track.durationMs ? formatTime(track.durationMs) : "0:00";
        var cover = escAttr(track.coverUrl || "");
        var card = document.createElement("div");
        card.className = "result-card";
        card.setAttribute("data-url", escAttr(trackUrl));
        card.innerHTML =
          '<div class="card-main" onclick="toggleCard(this.parentElement)">' +
          '<img src="' +
          cover +
          '" alt="" class="card-art" onerror="this.src=\'data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 width=%2244%22 height=%2244%22><rect fill=%22%23292524%22 width=%2244%22 height=%2244%22/></svg>\'">' +
          '<div class="card-info"><div class="card-title">' +
          escHtml(track.title) +
          "</div>" +
          '<div class="card-meta">' +
          escHtml(track.artist) +
          " · " +
          dur +
          "</div></div>" +
          '<button class="want-btn' +
          '" onclick="wantClick(event, this)" data-url="' +
          escAttr(trackUrl) +
          '" data-source="' +
          source +
          '" data-title="' +
          escAttr(track.title) +
          '">WANT</button></div>"' +
          '<div class="card-expanded-content"><div class="expanded-inner">' +
          '<img src="' +
          cover +
          '" class="expanded-art" onerror="this.src=\'data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 width=%2280%22 height=%2280%22><rect fill=%22%23292524%22 width=%2280%22 height=%2280%22/></svg>\'">' +
          '<div class="expanded-details"><div class="expanded-full-title">' +
          escHtml(track.title) +
          "</div>" +
          '<div class="card-meta">' +
          escHtml(track.artist) +
          "</div>" +
          '<div class="expanded-actions"><a href="' +
          escAttr(trackUrl) +
          '" target="_blank" class="source-link" onclick="event.stopPropagation()">' +
          '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"></path><polyline points="15 3 21 3 21 9"></polyline><line x1="10" y1="14" x2="21" y2="3"></line></svg>' +
          '<span style="text-transform:capitalize">View on ' +
          source +
          "</span></a></div></div></div></div>";
        sec.appendChild(card);
      }
      DOM.resultsContainer.appendChild(sec);
    });
    if (!anyVisible)
      DOM.resultsContainer.innerHTML =
        '<div class="empty-state">Enable at least one source to view results.</div>';
  }

  function renderResultsForSource(source, results, isError) {
    var section = document.getElementById("section-" + source);
    if (!section) return;
    updatePillCount(source, results ? results.length : 0);
    if (isError || !results || results.length === 0) {
      section.innerHTML =
        '<div class="section-header">' +
        source.toUpperCase() +
        " · 0</div>" +
        '<div class="empty-state" style="padding:20px;">No results found on ' +
        source +
        ".</div>";
      return;
    }
    var html =
      '<div class="section-header">' +
      source.toUpperCase() +
      " · " +
      results.length +
      "</div>";
    for (var i = 0; i < results.length; i++) {
      var track = results[i];
      var trackUrl = track.spotifyUrl || track.sourceUrl || "";
      var dur = track.durationMs ? formatTime(track.durationMs) : "0:00";
      var cover = escAttr(track.coverUrl || "");
      html +=
        '<div class="result-card" data-url="' +
        escAttr(trackUrl) +
        '">' +
        '<div class="card-main" onclick="toggleCard(this.parentElement)">' +
        '<img src="' +
        cover +
        '" alt="" class="card-art" onerror="this.src=\'data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 width=%2244%22 height=%2244%22><rect fill=%22%23292524%22 width=%2244%22 height=%2244%22/></svg>\'">' +
        '<div class="card-info"><div class="card-title">' +
        escHtml(track.title) +
        "</div>" +
        '<div class="card-meta">' +
        escHtml(track.artist) +
        " · " +
        dur +
        "</div></div>" +
        '<button class="want-btn' +
        '" onclick="wantClick(event, this)" data-url="' +
        escAttr(trackUrl) +
        '" data-source="' +
        source +
        '" data-title="' +
        escAttr(track.title) +
        '">WANT</button></div>"' +
        '<div class="card-expanded-content"><div class="expanded-inner">' +
        '<img src="' +
        cover +
        '" class="expanded-art" onerror="this.src=\'data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 width=%2280%22 height=%2280%22><rect fill=%22%23292524%22 width=%2280%22 height=%2280%22/></svg>\'">' +
        '<div class="expanded-details"><div class="expanded-full-title">' +
        escHtml(track.title) +
        "</div>" +
        '<div class="card-meta">' +
        escHtml(track.artist) +
        "</div>" +
        '<div class="expanded-actions"><a href="' +
        escAttr(trackUrl) +
        '" target="_blank" class="source-link" onclick="event.stopPropagation()">' +
        '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"></path><polyline points="15 3 21 3 21 9"></polyline><line x1="10" y1="14" x2="21" y2="3"></line></svg>' +
        '<span style="text-transform:capitalize">View on ' +
        source +
        "</span></a></div></div></div></div></div>";
    }
    section.innerHTML = html;
  }

  window.toggleCard = function (card) {
    document.querySelectorAll(".result-card.expanded").forEach(function (el) {
      if (el !== card) el.classList.remove("expanded");
    });
    card.classList.toggle("expanded");
  };

  window.wantClick = async function (e, btn) {
    e.stopPropagation();
    if (btn.classList.contains("wanted")) return;
    var url = btn.dataset.url;
    var source = btn.dataset.source;
    var title = btn.dataset.title;
    var orig = btn.textContent;
    btn.textContent = "✓";
    btn.style.opacity = "0.7";
    try {
      var res = await fetch(API + "/download", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ url: url, source: source }),
      });
      if (!res.ok) {
        var err = await res.json().catch(function () {
          return {};
        });
        throw new Error(err.error || "Request failed");
      }
      btn.classList.add("wanted");
      btn.textContent = "WANTED";
      btn.style.opacity = "1";
      showToast('"' + title + '" requested!');
      fetchQueueData();
      setTimeout(function () {
        btn.classList.remove("wanted");
        btn.textContent = "WANT";
      }, 3000);
    } catch (err) {
      btn.textContent = orig;
      btn.style.opacity = "1";
      showToast(err.message || "Failed to request song. Try again.", "error");
    }
  };

  function fetchQueueData() {
    fetch(API + "/stats")
      .then(function (r) {
        if (!r.ok)
          return r.json().then(function (e) {
            throw new Error(e.error || e.message || "Request failed");
          });
        return r.json();
      })
      .then(function (stats) {
        document.getElementById("stat-total").textContent = stats.total || 0;
        document.getElementById("stat-ready").textContent = stats.ready || 0;
        document.getElementById("stat-pending").textContent = stats.pending || 0;
        document.getElementById("stat-failed").textContent = stats.failed || 0;
        if (stats.total > 0) {
          DOM.queueBadge.textContent = stats.total;
          DOM.queueBadge.classList.add("visible");
        } else DOM.queueBadge.classList.remove("visible");
      })
      .catch(function () {});
    fetch(API + "/queue")
      .then(function (r) {
        if (!r.ok)
          return r.json().then(function (e) {
            throw new Error(e.error || e.message || "Request failed");
          });
        return r.json();
      })
      .then(function (data) {
        var tasks = data.tasks || data || [];
        if (!Array.isArray(tasks)) tasks = [];
        renderQueue(tasks);
      })
      .catch(function () {
        renderQueue([]);
      });
  }

  // ── Guest-facing status labels ──
  function guestLabel(s) {
    if (s === "ready") return "available";
    if (s === "failed") return "failed";
    return "requested";
  }

  function renderQueue(tasks) {
    var list = document.getElementById("queue-list");
    if (!tasks.length) {
      list.innerHTML = '<div class="empty-state">No songs have been requested yet.</div>';
      return;
    }

    var order = {
      ready: 0,
      pending: 1,
      stage2_deemix: 1,
      stage3_spotdl: 1,
      failed: 2,
    };
    tasks.sort(function (a, b) {
      return (order[a.status] || 3) - (order[b.status] || 3);
    });

    var html = "";
    for (var i = 0; i < tasks.length; i++) {
      var t = tasks[i];
      var status = t.status || "pending";
      var label = guestLabel(status);
      var title = t.title || t.track_title || null;
      var artist = t.artist || t.track_artist || "";
      var source = t.source || "";
      var url = t.spotify_url || "";
      var error = t.error_message || "";

      if (!title) title = cleanUrl(url, source);
      if (!artist && url.indexOf("soundcloud.com/") !== -1) artist = cleanArtist(url);

      var meta = "";
      if (artist) meta += escHtml(artist) + " · ";
      if (source) meta += "[" + escHtml(source) + "]";

      // Only show error reason for failed tracks (admin sees full pipeline in /admin)
      var errReason = "";
      if (status === "failed") errReason = shortError(error, source);

      html +=
        '<div class="queue-item"><div class="queue-info">' +
        '<div class="queue-title">' +
        escHtml(title) +
        "</div>" +
        '<div class="queue-meta">' +
        meta +
        (errReason
          ? '<br><span class="fail-reason">' + escHtml(errReason) + "</span>"
          : "") +
        "</div></div>" +
        '<div class="queue-status status-' +
        label +
        '">' +
        escHtml(label) +
        "</div></div>";
    }
    list.innerHTML = html;
  }

  function formatTime(ms) {
    var total = Math.floor(ms / 1000);
    var m = Math.floor(total / 60);
    var s = total % 60;
    return m + ":" + (s < 10 ? "0" : "") + s;
  }
  function escHtml(s) {
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

  function cleanArtist(url) {
    if (!url) return "";
    var parts = url.replace(/\?.*$/, "").replace(/\/$/, "").split("/");
    if (parts.length >= 2)
      return parts[parts.length - 2].replace(/-/g, " ").replace(/\b\w/g, function (c) {
        return c.toUpperCase();
      });
    return "";
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

  function shortError(err, source) {
    if (!err) return "";
    var e = err.toLowerCase();
    if (e.indexOf("sign in to confirm") !== -1 || e.indexOf("not a bot") !== -1)
      return "YouTube blocked this request";
    if (e.indexOf("drm protected") !== -1) return "Track is DRM protected";
    if (e.indexOf("404") !== -1 || e.indexOf("not found") !== -1)
      return "Track not found on " + source;
    if (e.indexOf("all stages exhausted") !== -1 || e.indexOf("all failed") !== -1)
      return "Could not download from any source";
    if (e.indexOf("spotdl succeeded but couldn't find") !== -1)
      return "Downloaded but file was lost";
    if (
      e.indexOf("playlist") !== -1 &&
      (e.indexOf("404") !== -1 || e.indexOf("not found") !== -1)
    )
      return "Playlist not found or is private";
    if (
      e.indexOf("incomplete yt initial data") !== -1 ||
      e.indexOf("unable to download api page") !== -1
    )
      return "YouTube API error — playlist may be unavailable";
    if (e.indexOf("failed after 3 attempts") !== -1)
      return "Download failed after 3 tries";
    var lines = err.split(/[\n\r]+/).filter(function (l) {
      return l.trim().length > 10;
    });
    var short = lines[lines.length - 1] || err.substring(0, 80);
    if (short.length > 100) short = short.substring(0, 97) + "...";
    return short;
  }

  function cleanUrl(url, source) {
    if (!url) return "Untitled " + (source || "track");
    if (url.indexOf("soundcloud.com/") !== -1) {
      var parts = url.replace(/\?.*$/, "").replace(/\/$/, "").split("/");
      if (parts.length >= 2)
        return parts[parts.length - 1]
          .replace(/-/g, " ")
          .replace(/_/g, " ")
          .replace(/\bfeat\b/gi, "feat.")
          .replace(/\bremix\b/gi, "Remix")
          .replace(/\bofficial\b/gi, "")
          .replace(/\bvideo\b/gi, "")
          .replace(/\blyrics?\b/gi, "")
          .replace(/\s+/g, " ")
          .trim()
          .replace(/\b\w/g, function (c) {
            return c.toUpperCase();
          });
    }
    if (url.indexOf("youtube.com/") !== -1 || url.indexOf("youtu.be/") !== -1) {
      var m = url.match(/[?&]v=([^&]+)/) || url.match(/youtu\.be\/([^?&]+)/);
      if (m) return "YouTube: " + m[1].substring(0, 11);
      return "YouTube track";
    }
    if (url.indexOf("spotify:track:") === 0) return "Spotify track";
    if (url.length > 40) url = url.substring(0, 37) + "...";
    return url;
  }

  function updatePillCount(source, count) {
    var pill = document.querySelector('.filter-pill[data-source="' + source + '"]');
    if (pill)
      pill.querySelector(".count").textContent = count > 0 ? "(" + count + ")" : "";
  }
  function updateCounts() {
    document.querySelectorAll(".filter-pill .count").forEach(function (el) {
      el.textContent = "";
    });
  }

  window.switchTab = function (tabName) {
    document.querySelectorAll(".view").forEach(function (v) {
      v.classList.remove("active");
    });
    document.querySelectorAll(".nav-btn").forEach(function (b) {
      b.classList.remove("active");
    });
    document.getElementById("view-" + tabName).classList.add("active");
    document.getElementById("nav-" + tabName).classList.add("active");
    if (tabName === "requests") fetchQueueData();
    if (tabName === "playlists" && typeof fetchPlaylists === "function") fetchPlaylists();
  };

  function showToast(message, type) {
    var container = document.getElementById("toasts");
    var toast = document.createElement("div");
    toast.className = "toast" + (type === "error" ? " error" : "");
    toast.textContent = message;
    container.appendChild(toast);
    requestAnimationFrame(function () {
      toast.classList.add("show");
    });
    setTimeout(function () {
      toast.classList.remove("show");
      setTimeout(function () {
        toast.remove();
      }, 300);
    }, 3000);
  }

  function formatLastSynced(ts) {
    if (!ts) return "never";
    var diff = Math.floor(Date.now() / 1000) - ts;
    if (diff < 60) return "just now";
    if (diff < 3600) return Math.floor(diff / 60) + "m ago";
    if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
    return Math.floor(diff / 86400) + "d ago";
  }
  // Expose shared utilities for playlists.js
  window._w = {
    escHtml: escHtml,
    escAttr: escAttr,
    shortError: shortError,
    cleanUrl: cleanUrl,
    playlistTitle: playlistTitle,
    formatLastSynced: formatLastSynced,
    showToast: showToast,
    API: API,
  };

  fetchQueueData();
  setInterval(function () {
    if (document.getElementById("view-requests").classList.contains("active"))
      fetchQueueData();
    if (
      document.getElementById("view-playlists").classList.contains("active") &&
      typeof fetchPlaylists === "function"
    )
      fetchPlaylists();
  }, 15000);
})();
