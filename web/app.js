/* ==========================================================
   KATALOG  ·  search, filters, rendering
   No build step and no dependencies. Data comes from
   data/catalogue.js (window.CATALOGUE).
   ========================================================== */
(() => {
  "use strict";

  const TYPES = [
    { key: "movie", label: "Movies",   one: "Movie",   shape: "poster" },
    { key: "tv",    label: "TV Shows", one: "TV Show", shape: "poster" },
    { key: "album", label: "Albums",   one: "Album",   shape: "square" },
    { key: "book",  label: "Books",    one: "Book",    shape: "poster" },
    { key: "comic", label: "Comics",   one: "Comic",   shape: "poster" },
  ];
  const TYPE_BY_KEY = Object.fromEntries(TYPES.map((t) => [t.key, t]));
  const TYPE_ALIASES = {
    movie: "movie", movies: "movie", film: "movie", films: "movie",
    tv: "tv", show: "tv", shows: "tv", series: "tv",
    album: "album", albums: "album", music: "album", record: "album",
    book: "book", books: "book", novel: "book",
    comic: "comic", comics: "comic", manga: "comic", graphic: "comic",
  };

  const SORTS = [
    { key: "relevance", label: "Relevance" },
    { key: "new",       label: "Newest" },
    { key: "old",       label: "Oldest" },
    { key: "az",        label: "A–Z" },
  ];
  const VIEWS = [
    { key: "grid", label: "Covers" },
    { key: "list", label: "List" },
  ];

  /* Cover palettes drawn from the Kaptol colours: [background, text, accent] */
  const PALETTES = [
    ["#1b395b", "#fffabd", "#e84a2d"],
    ["#fffabd", "#1b395b", "#e84a2d"],
    ["#050505", "#fffabd", "#e84a2d"],
    ["#2d0a1e", "#ffb7c5", "#d68c45"],
    ["#d68c45", "#3b2208", "#fffabd"],
    ["#2d7a3e", "#fffabd", "#050505"],
    ["#ffb7c5", "#2d0a1e", "#e84a2d"],
    ["#fcfdff", "#050505", "#e84a2d"],
    ["#3b2208", "#fffabd", "#d68c45"],
    ["#e84a2d", "#fffabd", "#1b395b"],
    ["#fffabd", "#050505", "#2d7a3e"],
  ];
  const SECTION_BG = { movie: "#050505", tv: "#2d0a1e", album: "#e84a2d", book: "#d68c45", comic: "#ffb7c5" };
  const MARKS = ["circle", "ring", "bar", "column", "half", "lines"];

  const state = {
    q: "",
    type: "all",
    genres: new Set(),
    decade: null,
    sort: "relevance",
    view: "grid",
  };

  /* ---------------- helpers ---------------- */
  const $ = (sel, root = document) => root.querySelector(sel);
  const el = (tag, attrs = {}, ...kids) => {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") node.className = v;
      else if (k === "text") node.textContent = v;
      else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
      else if (k === "style") node.style.cssText = v;
      else node.setAttribute(k, v === true ? "" : v);
    }
    for (const kid of kids.flat()) if (kid != null) node.append(kid);
    return node;
  };
  const norm = (s) => String(s).normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase()
    .replace(/[đ]/g, "d").replace(/[^a-z0-9]+/g, " ").trim();
  const hash = (s) => { let h = 2166136261; for (const c of s) { h ^= c.charCodeAt(0); h = Math.imul(h, 16777619); } return h >>> 0; };
  const decadeOf = (y) => Math.floor(y / 10) * 10;
  const byline = (item) => item.creators.map((c) => c.name).join(", ");
  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

  function prepare(item) {
    const meta = Object.values(item.meta || {}).join(" ");
    return {
      ...item,
      _title: norm(item.title),
      _people: norm(item.creators.map((c) => c.name).join(" ")),
      _genres: item.genres.map(norm),
      _rest: norm([item.synopsis, meta, item.year, item.type].join(" ")),
      _decade: decadeOf(item.year),
    };
  }

  const ITEMS = (window.CATALOGUE || []).map(prepare);

  /* ---------------- query parsing ---------------- */
  // Free words must all match (AND). Filters: type:book genre:jazz by:lynch year:1997 1990s 1980-1989
  function parseQuery(raw) {
    const out = { words: [], type: null, genres: [], people: [], yearFrom: null, yearTo: null };
    const tokens = raw.match(/(\w+:)?"[^"]*"|\S+/g) || [];
    for (let tok of tokens) {
      tok = tok.replace(/"/g, "");
      const m = tok.match(/^(\w+):(.+)$/);
      if (m) {
        const key = m[1].toLowerCase(), val = m[2];
        if (key === "type" || key === "t") { out.type = TYPE_ALIASES[norm(val)] || null; continue; }
        if (key === "genre" || key === "g") { out.genres.push(norm(val)); continue; }
        if (key === "by" || key === "who") { out.people.push(norm(val)); continue; }
        if (key === "year" || key === "y") { applyYear(out, val); continue; }
      }
      if (applyYear(out, tok, true)) continue;
      const alias = TYPE_ALIASES[norm(tok)];
      if (alias && tokens.length > 1) { out.type = alias; continue; }
      const n = norm(tok);
      if (n) out.words.push(...n.split(" "));
    }
    return out;
  }
  function applyYear(out, tok, strict) {
    let m;
    if ((m = tok.match(/^(\d{4})\s*[-–]\s*(\d{4})$/))) { out.yearFrom = +m[1]; out.yearTo = +m[2]; return true; }
    if ((m = tok.match(/^(\d{3})0s$/))) { out.yearFrom = +m[1] * 10; out.yearTo = +m[1] * 10 + 9; return true; }
    if ((m = tok.match(/^'?(\d)0s$/))) { const d = 1900 + +m[1] * 10; out.yearFrom = d; out.yearTo = d + 9; return true; }
    if ((m = tok.match(/^(19|20)\d{2}$/))) { out.yearFrom = out.yearTo = +tok; return true; }
    if (!strict && (m = tok.match(/^\d{4}$/))) { out.yearFrom = out.yearTo = +tok; return true; }
    return false;
  }

  function score(item, q) {
    let s = 0;
    for (const w of q.words) {
      let hit = 0;
      if (item._title.split(" ").some((t) => t.startsWith(w))) hit = 6;
      else if (item._title.includes(w)) hit = 4;
      else if (item._people.includes(w)) hit = 3;
      else if (item._genres.some((g) => g.includes(w))) hit = 2;
      else if (item._rest.includes(w)) hit = 1;
      if (!hit) return 0;
      s += hit;
    }
    if (q.words.length && item._title === q.words.join(" ")) s += 20;
    return s || 1;
  }

  /* ---------------- filtering ---------------- */
  // `ignore` drops one facet so its own counts stay useful while it is active.
  function filter(ignore) {
    const q = parseQuery(state.q);
    const results = [];
    for (const item of ITEMS) {
      const type = q.type || state.type;
      if (ignore !== "type" && type !== "all" && item.type !== type) continue;
      if (ignore !== "genre" && state.genres.size && !item.genres.some((g) => state.genres.has(g))) continue;
      if (ignore !== "decade" && state.decade && item._decade !== state.decade) continue;
      if (q.genres.length && !q.genres.every((g) => item._genres.some((ig) => ig.includes(g)))) continue;
      if (q.people.length && !q.people.every((p) => item._people.includes(p))) continue;
      if (q.yearFrom != null && (item.year < q.yearFrom || item.year > q.yearTo)) continue;
      const s = score(item, q);
      if (!s) continue;
      results.push({ item, s });
    }
    const sortKey = state.sort === "relevance" && !q.words.length ? "az" : state.sort;
    const cmp = {
      relevance: (a, b) => b.s - a.s || a.item.title.localeCompare(b.item.title),
      new: (a, b) => b.item.year - a.item.year || a.item.title.localeCompare(b.item.title),
      old: (a, b) => a.item.year - b.item.year || a.item.title.localeCompare(b.item.title),
      az: (a, b) => a.item.title.replace(/^the /i, "").localeCompare(b.item.title.replace(/^the /i, "")),
    }[sortKey];
    return { list: results.sort(cmp).map((r) => r.item), q };
  }

  /* ---------------- covers ---------------- */
  function cover(item, opts = {}) {
    const shape = TYPE_BY_KEY[item.type].shape;
    const src = item.cover || (window.COVERS || {})[item.id];
    const h = hash(item.id);
    const options = PALETTES.filter((p) => p[0] !== (opts.sectionBg || SECTION_BG[item.type]));
    const [bg, fg, acc] = options[h % options.length];
    const mark = MARKS[(h >>> 4) % MARKS.length];
    return el("div", {
      class: "cover", "data-shape": shape, "aria-hidden": "true",
      style: `--c-bg:${bg};--c-fg:${fg};--c-acc:${acc}`,
    },
      el("div", { class: `cover-mark mark-${mark}` }),
      el("div", { class: "cover-art" },
        el("div", { class: "cover-title", text: item.title }),
        el("div", { class: "cover-foot" },
          el("span", { text: item.creators[0].name }),
          el("span", { text: String(item.year) }))),
      src ? el("img", {
        src, alt: "", loading: opts.eager ? "eager" : "lazy", decoding: "async",
        onload: (e) => e.target.classList.add("is-loaded"),
        onerror: (e) => e.target.remove(),
      }) : null);
  }

  /* ---------------- rendering ---------------- */
  function renderTypeBar(counts, total) {
    const bar = $("#type-bar");
    bar.replaceChildren(
      ...[{ key: "all", label: "All" }, ...TYPES].map((t) => {
        const n = t.key === "all" ? total : counts[t.key] || 0;
        return el("button", {
          class: "type-tab", type: "button",
          "aria-pressed": String(state.type === t.key),
          disabled: n === 0 && state.type !== t.key && t.key !== "all",
          onclick: () => { state.type = state.type === t.key ? "all" : t.key; update(); },
        }, el("span", { text: t.label }), el("small", { text: String(n) }));
      }));
  }

  function renderIndex(counts) {
    $("#index-menu").replaceChildren(...TYPES.map((t) =>
      el("li", { class: counts[t.key] ? null : "is-empty" }, el("a", {
        href: `#${t.key}`,
        onclick: (e) => {
          e.preventDefault();
          const target = document.getElementById(`section-${t.key}`);
          if (target) target.scrollIntoView({ behavior: "smooth" });
          else { state.type = t.key; update(); }
        },
      }, el("span", { text: t.label }), el("small", { text: plural(counts[t.key] || 0, "title", "titles") })))));
  }

  function renderChips(root, entries, isActive, onPick) {
    root.replaceChildren(...entries.map(([key, label, n]) =>
      el("button", {
        class: "chip", type: "button", "aria-pressed": String(isActive(key)),
        onclick: () => onPick(key),
      }, el("span", { text: label }), n != null ? el("small", { text: String(n) }) : null)));
  }

  function renderFacets() {
    const forGenres = filter("genre").list;
    const gCount = new Map();
    for (const it of forGenres) for (const g of it.genres) gCount.set(g, (gCount.get(g) || 0) + 1);
    for (const g of state.genres) if (!gCount.has(g)) gCount.set(g, 0);
    const genres = [...gCount].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
    renderChips($("#genre-chips"), genres.map(([g, n]) => [g, g, n]),
      (g) => state.genres.has(g),
      (g) => { state.genres.has(g) ? state.genres.delete(g) : state.genres.add(g); update(); });
    $("#genre-count").textContent = state.genres.size ? `${state.genres.size} selected` : `${genres.length}`;

    const forDecades = filter("decade").list;
    const dCount = new Map();
    for (const it of forDecades) dCount.set(it._decade, (dCount.get(it._decade) || 0) + 1);
    const decades = [...dCount].sort((a, b) => a[0] - b[0]);
    renderChips($("#decade-chips"), decades.map(([d, n]) => [d, `${d}s`, n]),
      (d) => state.decade === d,
      (d) => { state.decade = state.decade === d ? null : d; update(); });

    renderChips($("#sort-chips"), SORTS.map((s) => [s.key, s.label]),
      (k) => state.sort === k, (k) => { state.sort = k; update(); });
    renderChips($("#view-chips"), VIEWS.map((v) => [v.key, v.label]),
      (k) => state.view === k, (k) => { state.view = k; update(); });
  }

  function renderCard(item) {
    return el("article", {
      class: "card", tabindex: "0", role: "button",
      "aria-label": `${item.title}, ${item.year}, ${byline(item)}`,
      onclick: () => openDetail(item.id),
      onkeydown: (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openDetail(item.id); } },
    },
      cover(item),
      el("div", { class: "card-head" },
        el("h3", { class: "card-title", text: item.title }),
        el("span", { class: "card-year", text: String(item.year) })),
      el("p", { class: "card-by", text: byline(item) }),
      el("p", { class: "card-tags", text: item.genres.join(" · ") }));
  }

  function renderRow(item) {
    return el("li", {}, el("button", {
      class: "row", type: "button", onclick: () => openDetail(item.id),
    },
      cover(item),
      el("span", { class: "row-title", text: item.title }),
      el("span", { class: "row-by", text: byline(item) }),
      el("span", { class: "row-tags", text: item.genres.join(" · ") }),
      el("span", { class: "row-year", text: String(item.year) })));
  }

  function renderSections(list) {
    const groups = Object.fromEntries(TYPES.map((t) => [t.key, []]));
    for (const it of list) groups[it.type].push(it);
    const sections = TYPES.filter((t) => groups[t.key].length).map((t) => {
      const items = groups[t.key];
      const body = state.view === "grid"
        ? el("div", { class: "grid" }, items.map(renderCard))
        : el("ol", { class: "list" }, items.map(renderRow));
      return el("section", { class: "cat-section", id: `section-${t.key}`, "data-type": t.key },
        el("div", { class: "section-inner" },
          el("h2", {}, t.label, el("small", { text: plural(items.length, "title", "titles") })),
          body));
    });
    $("#sections").replaceChildren(...sections);
  }

  function renderStatus(list, q) {
    const bits = [];
    if (state.q.trim()) bits.push(`“${state.q.trim()}”`);
    const type = q.type || state.type;
    if (type !== "all") bits.push(TYPE_BY_KEY[type].label);
    if (state.genres.size) bits.push([...state.genres].join(" or "));
    if (state.decade) bits.push(`${state.decade}s`);
    $("#status").textContent = bits.length
      ? `${plural(list.length, "result", "results")} for ${bits.join(" · ")}`
      : "";
    $("#total-count").textContent = plural(ITEMS.length, "title", "titles");
    const empty = list.length === 0;
    $("#empty").hidden = !empty;
    $("#empty-query").textContent = bits.join(" · ");
  }

  function update({ push = true } = {}) {
    const { list, q } = filter();
    const forTypes = filter("type").list;
    const counts = {};
    for (const it of forTypes) counts[it.type] = (counts[it.type] || 0) + 1;
    renderTypeBar(counts, forTypes.length);
    renderIndex(counts);
    renderFacets();
    renderSections(list);
    renderStatus(list, q);
    renderFilterCount();
    document.body.classList.toggle("is-filtering",
      Boolean(state.q.trim() || state.type !== "all" || state.genres.size || state.decade));
    if (push) writeHash();
    observeSections();
  }

  /* ---------------- detail ---------------- */
  const dialog = $("#detail");

  function openDetail(id, { push = true } = {}) {
    const item = ITEMS.find((i) => i.id === id);
    if (!item) return;
    const t = TYPE_BY_KEY[item.type];
    dialog.dataset.type = item.type;
    const rows = [
      ...item.creators.map((c) => [c.role, c.name]),
      ["Released", String(item.year)],
      ...Object.entries(item.meta || {}),
      ["Genres", item.genres.join(", ")],
    ];
    $("#detail-inner").replaceChildren(
      el("button", { class: "detail-close", type: "button", "aria-label": "Close", text: "×", onclick: closeDetail }),
      cover(item, { eager: true }),
      el("div", { class: "detail-body" },
        el("h2", { id: "detail-title" }, item.title),
        el("p", { class: "detail-type", text: `${t.one} · ${byline(item)}` }),
        el("p", { class: "detail-synopsis", text: item.synopsis }),
        el("dl", { class: "rows" }, rows.map(([k, v]) => el("div", {}, el("dt", { text: k }), el("dd", { text: v })))),
        el("div", { class: "chips" },
          item.genres.map((g) => el("button", {
            class: "chip", type: "button",
            onclick: () => { closeDetail(); state.genres = new Set([g]); state.type = "all"; state.q = ""; $("#search").value = ""; update(); scrollToResults(); },
          }, el("span", { text: g }))),
          item.creators.map((c) => el("button", {
            class: "chip", type: "button",
            onclick: () => { closeDetail(); state.q = `by:"${c.name}"`; state.genres.clear(); state.decade = null; state.type = "all"; $("#search").value = state.q; update(); scrollToResults(); },
          }, el("span", { text: `More by ${c.name}` }))))));
    if (!dialog.open) dialog.showModal();
    dialog.scrollTop = 0;
    if (push) writeHash(item.id);
  }
  function closeDetail() {
    if (dialog.open) dialog.close();
  }
  dialog.addEventListener("close", () => writeHash());
  dialog.addEventListener("click", (e) => { if (e.target === dialog) closeDetail(); });

  function scrollToResults() {
    const first = $("#sections section");
    (first || $("#intro")).scrollIntoView({ behavior: "smooth" });
  }

  /* ---------------- URL state ---------------- */
  function writeHash(openId) {
    const p = new URLSearchParams();
    if (state.q.trim()) p.set("q", state.q.trim());
    if (state.type !== "all") p.set("type", state.type);
    if (state.genres.size) p.set("genre", [...state.genres].join("|"));
    if (state.decade) p.set("decade", state.decade);
    if (state.sort !== "relevance") p.set("sort", state.sort);
    if (state.view !== "grid") p.set("view", state.view);
    const id = openId || (dialog.open && new URLSearchParams(location.hash.slice(1)).get("item"));
    if (id) p.set("item", id);
    const next = p.toString();
    history.replaceState(null, "", next ? `#${next}` : location.pathname + location.search);
  }
  function readHash() {
    const p = new URLSearchParams(location.hash.slice(1));
    state.q = p.get("q") || "";
    state.type = TYPE_BY_KEY[p.get("type")] ? p.get("type") : "all";
    state.genres = new Set((p.get("genre") || "").split("|").filter(Boolean));
    state.decade = p.get("decade") ? +p.get("decade") : null;
    state.sort = SORTS.some((s) => s.key === p.get("sort")) ? p.get("sort") : "relevance";
    state.view = p.get("view") === "list" ? "list" : "grid";
    $("#search").value = state.q;
    return p.get("item");
  }

  function reset() {
    state.q = ""; state.type = "all"; state.genres.clear(); state.decade = null;
    $("#search").value = "";
    update();
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  /* ---------------- header colour follows section ---------------- */
  const header = $("#site-header");
  let ticking = false;
  function syncHeader() {
    ticking = false;
    const y = header.getBoundingClientRect().bottom + 2;
    const hit = document.elementFromPoint(window.innerWidth / 2, y);
    const section = hit && hit.closest("main > section, .cat-section, .site-footer");
    if (!section) return;
    const cs = getComputedStyle(section);
    header.style.setProperty("--hdr-bg", cs.backgroundColor);
    header.style.setProperty("--hdr-fg", cs.color);
  }
  function observeSections() { syncHeader(); }
  window.addEventListener("scroll", () => {
    if (!ticking) { ticking = true; requestAnimationFrame(syncHeader); }
  }, { passive: true });
  new ResizeObserver(() => {
    document.documentElement.style.setProperty("--header-h", `${header.offsetHeight}px`);
  }).observe(header);

  /* ---------------- events ---------------- */
  const search = $("#search");
  search.addEventListener("input", () => { state.q = search.value; update(); });
  search.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { search.value = ""; state.q = ""; update(); }
    if (e.key === "Enter") { search.blur(); scrollToResults(); }
  });
  document.addEventListener("keydown", (e) => {
    const typing = /input|textarea/i.test(document.activeElement.tagName);
    if (e.key === "/" && !typing && !dialog.open) { e.preventDefault(); search.focus(); search.select(); }
  });

  const trigger = $("#drawer-trigger");
  trigger.addEventListener("click", () => {
    const open = $("#drawer").classList.toggle("open");
    trigger.setAttribute("aria-expanded", String(open));
  });
  function renderFilterCount() {
    const n = state.genres.size + (state.decade ? 1 : 0);
    $("#filter-count").textContent = n ? String(n) : "";
  }

  document.querySelectorAll("[data-reset]").forEach((b) => b.addEventListener("click", (e) => { e.preventDefault(); reset(); }));

  window.addEventListener("hashchange", () => {
    const id = readHash();
    update({ push: false });
    if (id) openDetail(id, { push: false }); else closeDetail();
  });

  /* ---------------- go ---------------- */
  const openId = readHash();
  update({ push: false });
  if (openId) openDetail(openId, { push: false });
})();
