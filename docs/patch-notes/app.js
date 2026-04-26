(function () {
  "use strict";

  // The changelog directory lives two levels up from docs/patch-notes/.
  const CHANGELOG_BASE = "../../changelog/";
  const MANIFEST_URL = CHANGELOG_BASE + "manifest.json";

  const $status = document.getElementById("status");
  const $list = document.getElementById("release-list");
  const $view = document.getElementById("release-view");
  const $body = document.getElementById("release-body");
  const $back = document.getElementById("back");
  const $app = document.getElementById("app");

  let releases = [];

  async function loadManifest() {
    try {
      const res = await fetch(MANIFEST_URL, { cache: "no-cache" });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      releases = await res.json();
    } catch (err) {
      $status.textContent =
        "Couldn't load changelog manifest. Run `tools/gen-changelog-manifest.sh` " +
        "and serve the repo root over HTTP.";
      $status.style.color = "var(--danger)";
      console.error(err);
      return;
    }
    renderList();
  }

  function renderList() {
    if (!releases.length) {
      $status.textContent = "No releases yet.";
      return;
    }
    $status.hidden = true;
    $list.hidden = false;
    $list.innerHTML = "";
    for (const r of releases) {
      const li = document.createElement("li");
      if (r.locked) {
        li.appendChild(renderLockedCard(r));
      } else {
        li.appendChild(renderReleaseCard(r));
      }
      $list.appendChild(li);
    }
  }

  function renderReleaseCard(r) {
    const btn = document.createElement("button");
    btn.className = "release-card";
    btn.type = "button";
    btn.innerHTML = `
      <div class="row">
        <span class="codename" translate="no"></span>
        <span class="version notranslate" translate="no"></span>
        <span class="date notranslate" translate="no"></span>
      </div>
      <p class="summary"></p>
      <div class="row" style="margin-top: 10px;">
        <span class="build notranslate" translate="no"></span>
      </div>
    `;
    btn.querySelector(".codename").textContent = r.codename || "—";
    btn.querySelector(".version").textContent = r.version ? `v${r.version}` : "";
    btn.querySelector(".date").textContent = r.date || "";
    btn.querySelector(".summary").textContent = r.summary || "";
    btn.querySelector(".build").textContent = `build #${r.build}`;
    btn.addEventListener("click", () => openRelease(r));
    return btn;
  }

  function renderLockedCard(r) {
    const card = document.createElement("div");
    card.className = "release-card release-card--locked";
    card.setAttribute("aria-disabled", "true");
    card.innerHTML = `
      <div class="row">
        <span class="codename">Scheduled release</span>
        <span class="build notranslate" translate="no"></span>
      </div>
      <p class="countdown notranslate" translate="no" data-publish-at="${escapeHtml(r.publishAt || "")}">—</p>
      <p class="summary muted">Contents unlock automatically at the time above.</p>
    `;
    card.querySelector(".build").textContent = `build #${r.build}`;
    attachCountdown(card.querySelector(".countdown"), r.publishAt);
    return card;
  }

  // Map of element -> interval handle, so we can tear down if we re-render.
  const _countdowns = new WeakMap();

  function attachCountdown(el, iso) {
    const target = new Date(iso);
    if (isNaN(target.getTime())) {
      el.textContent = "(unscheduled)";
      return;
    }
    function tick() {
      const ms = target.getTime() - Date.now();
      if (ms <= 0) {
        el.textContent = "Unlocking…";
        clearInterval(_countdowns.get(el));
        // The server will publish on the next sync (≤ 60s). Refresh soon.
        setTimeout(() => loadManifest(), 3000);
        return;
      }
      const s = Math.floor(ms / 1000);
      const d = Math.floor(s / 86400);
      const h = Math.floor((s % 86400) / 3600);
      const m = Math.floor((s % 3600) / 60);
      const sec = s % 60;
      el.textContent = d > 0
        ? `${d}d ${h}h ${m}m ${sec}s`
        : `${String(h).padStart(2,"0")}:${String(m).padStart(2,"0")}:${String(sec).padStart(2,"0")}`;
    }
    tick();
    const id = setInterval(tick, 1000);
    _countdowns.set(el, id);
  }

  async function openRelease(r) {
    location.hash = `#build-${r.build}`;
    $app.hidden = true;
    $view.hidden = false;
    $body.innerHTML = `<p class="muted">Loading ${r.codename}…</p>`;
    try {
      const res = await fetch(CHANGELOG_BASE + r.file, { cache: "no-cache" });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const raw = await res.text();
      const { data, body } = window.MD.parseFrontmatter(raw);
      renderRelease(data, body);
    } catch (err) {
      $body.innerHTML = `<p style="color: var(--danger)">Failed to load ${r.file}: ${err}</p>`;
    }
    window.scrollTo({ top: 0, behavior: "instant" });
  }

  function renderRelease(data, body) {
    const tags = Array.isArray(data.tags) ? data.tags : [];
    const headParts = [];
    if (data.build) headParts.push(`<span class="tag notranslate" translate="no">build #${escapeHtml(data.build)}</span>`);
    if (data.version) headParts.push(`<span class="tag notranslate" translate="no">v${escapeHtml(data.version)}</span>`);
    if (data.date) headParts.push(`<span class="notranslate" translate="no">${escapeHtml(data.date)}</span>`);
    for (const t of tags) headParts.push(`<span class="tag">${escapeHtml(t)}</span>`);

    const title = data.codename || "(untitled build)";
    const summary = data.summary ? `<p class="muted">${escapeHtml(data.summary)}</p>` : "";

    $body.innerHTML =
      `<h1>${escapeHtml(title)}</h1>` +
      `<div class="meta">${headParts.join("")}</div>` +
      summary +
      window.MD.render(body);
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  $back.addEventListener("click", () => {
    $view.hidden = true;
    $app.hidden = false;
    if (location.hash) {
      history.replaceState(null, "", location.pathname + location.search);
    }
    window.scrollTo({ top: 0, behavior: "instant" });
  });

  // Deep-link: open whichever build the URL hash points at, after manifest
  // has loaded.
  async function start() {
    await loadManifest();
    const m = /^#build-(\d+)$/.exec(location.hash);
    if (m) {
      const r = releases.find((x) => String(x.build) === m[1]);
      if (r) openRelease(r);
    }
  }

  start();
})();
