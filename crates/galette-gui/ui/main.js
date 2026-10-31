// Galette's window. Talks to the Rust side (src/main.rs) through Tauri:
// `invoke` calls a command, `listen` receives the events sent while ripping
// (`rip`) and when a disc is inserted or removed (`drive`).

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: openDialog } = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);

let disc = null; // what the `scan` command returned
let prefs = null; // the saved settings, see `Prefs` in src/main.rs
let ripping = false;
/// The user asked to stop; the rip ends at its next check.
let stopping = false;
let scanning = false;
let lastAlbumDir = null;

// ---------- Disc ----------

async function scan() {
  if (scanning || ripping) return;
  scanning = true;
  showEmpty("empty.reading", {}, true);
  disc = null;
  updateRipButton();
  try {
    disc = await invoke("scan");
    showDisc();
  } catch (error) {
    if (error?.code === "noDisc") showEmpty("empty.insert");
    else if (error?.code === "noDrive") showEmpty("empty.noDrive");
    else showEmpty("empty.error", { error: messageRef(error) });
  }
  scanning = false;
}

// The Rust side watches the drive and tells when its state changes.
listen("drive", ({ payload: state }) => {
  if (ripping) return;
  switch (state) {
    case "ready":
      scan();
      break;
    case "loading":
      disc = null;
      showEmpty("empty.loading", {}, true);
      break;
    case "empty":
      disc = null;
      showEmpty("empty.insert");
      break;
    case "nodrive":
      disc = null;
      $("drive").removeAttribute("data-i18n");
      $("drive").textContent = "";
      showEmpty("empty.noDrive");
      break;
  }
  updateRipButton();
});

/// The empty state, with the text `key`.
function showEmpty(key, params = {}, busy = false) {
  $("album").hidden = true;
  $("empty").hidden = false;
  $("empty").classList.toggle("busy", busy);
  localize($("empty-text"), key, params);
}

function showDisc() {
  $("empty").hidden = true;
  $("album").hidden = false;
  if (disc.warning) setMessage("error.detail", { text: { key: "error.lookup" }, detail: disc.warning.detail ?? "" });
  else setMessage(null);

  showDrive();

  const select = $("release");
  select.replaceChildren();
  disc.releases.forEach((release, i) => select.add(new Option(describe(release), i)));
  // The disc's own titles, first choice when MusicBrainz does not know it.
  if (disc.cdText) {
    const cdText = new Option("", "cdtext");
    localize(cdText, "album.cdText", { artist: disc.cdText.artist, title: disc.cdText.title });
    select.add(cdText);
  }
  const manual = new Option("", -1);
  localize(manual, "album.manual");
  select.add(manual);
  select.value = disc.releases.length ? 0 : disc.cdText ? "cdtext" : -1;

  const badges = $("badges");
  badges.replaceChildren(
    disc.releases.length ? badge("badge.musicbrainz", {}, "good") : badge("badge.noMusicbrainz", {}, "warn"),
    disc.accuraterip === null
      ? badge("badge.noAccuraterip", {}, "warn")
      : badge("badge.accuraterip", { sets: disc.accuraterip }, "good"),
    badge("badge.tracks", { count: disc.tracks.filter((t) => t.isAudio).length }),
  );
  if (disc.cdText) badges.append(badge("badge.cdText", {}, "good"));

  buildTracks();
  updateRipButton();
  selectRelease();
}

function showDrive() {
  if (!disc) return;
  let offset = disc.offset === null ? { key: "drive.offsetUnknown" } : { key: "drive.offset", params: { offset: signed(disc.offset) } };
  if (prefs.offset !== null) offset = { key: "drive.offsetManual", params: { offset: signed(prefs.offset) } };
  const c2 = disc.c2 && prefs.c2 ? "✓" : "✗";
  localize($("drive"), "drive.line", { drive: disc.drive, offset, c2 });
}

function describe(release) {
  const parts = [release.country, release.date, release.label, release.catalog_number].filter(Boolean);
  const cd = release.disc_count > 1 ? ` (CD ${release.disc_number}/${release.disc_count})` : "";
  return `${release.artist} – ${release.title}${cd} · ${parts.join(" · ")}`;
}

function badge(key, params = {}, kind = "") {
  const span = document.createElement("span");
  span.className = `badge ${kind}`;
  localize(span, key, params);
  return span;
}

function currentRelease() {
  const value = $("release").value;
  if (value === "cdtext") return disc.cdText;
  const i = Number(value);
  return i >= 0 ? disc.releases[i] : null;
}

// ---------- Tracks ----------

function buildTracks() {
  const body = $("tracks");
  body.replaceChildren();
  for (const track of disc.tracks) {
    const row = body.insertRow();
    row.dataset.number = track.number;
    if (!track.isAudio) row.className = "data";

    const check = document.createElement("input");
    check.type = "checkbox";
    check.checked = track.isAudio;
    check.disabled = !track.isAudio;
    check.addEventListener("change", updateRipButton);
    row.insertCell().append(check);
    row.insertCell().textContent = track.number;
    const title = textInput("title");
    if (!track.isAudio) localize(title, "track.data", {}, "placeholder");
    row.insertCell().append(title);
    row.insertCell().append(textInput("artist"));
    row.insertCell().textContent = track.length;

    const status = row.insertCell();
    status.innerHTML = `<div class="state"></div><div class="bar" hidden><div></div></div>`;
  }
  $("check-all").checked = true;
}

function textInput(field) {
  const input = document.createElement("input");
  input.dataset.field = field;
  input.spellcheck = false;
  return input;
}

function rows() {
  return [...$("tracks").rows].filter((row) => !row.classList.contains("data"));
}

function field(row, name) {
  return row.querySelector(`[data-field=${name}]`);
}

async function selectRelease() {
  const release = currentRelease();
  $("album-artist").value = release?.artist ?? "";
  $("album-title").value = release?.title ?? "";
  $("album-date").value = release?.date?.slice(0, 4) ?? "";
  for (const row of rows()) {
    const info = release?.tracks.find((t) => t.number === Number(row.dataset.number));
    field(row, "title").value = info?.title ?? "";
    localize(field(row, "title"), ...defaultTitle(Number(row.dataset.number)), "placeholder");
    field(row, "artist").value = info?.artist ?? release?.artist ?? "";
  }
  await loadCover(release);
}

async function loadCover(release) {
  const img = $("cover");
  img.hidden = true;
  $("cover-none").hidden = false;
  if (!release?.id) return; // covers come from MusicBrainz releases only
  localize($("cover-none"), "album.coverLoading");
  const url = await invoke("cover", { release }).catch(() => null);
  if (currentRelease() !== release) return; // another edition was chosen meanwhile
  localize($("cover-none"), "album.noCover");
  if (url) {
    img.src = url;
    img.hidden = false;
    $("cover-none").hidden = true;
  }
}

/// The release as shown on screen, with the user's edits.
function editedRelease() {
  const base = currentRelease();
  const artist = $("album-artist").value.trim();
  const title = $("album-title").value.trim();
  if (!base && !artist && !title) return null; // tracks will only be numbered

  const release = structuredClone(
    base ?? {
      id: "", release_group_id: null, title: "", artist: "", artist_ids: [], date: null,
      original_date: null, country: null, label: null, catalog_number: null, barcode: null,
      disc_number: 1, disc_count: 1, tracks: [],
    },
  );
  if (release.artist !== artist) release.artist_ids = [];
  release.artist = artist;
  release.title = title;
  const year = $("album-date").value.trim();
  if (!release.date?.startsWith(year)) release.date = year || null;

  for (const row of rows()) {
    const number = Number(row.dataset.number);
    let track = release.tracks.find((t) => t.number === number);
    if (!track) {
      track = { id: "", recording_id: null, number, title: "", artist: "", artist_ids: [], length_ms: null };
      release.tracks.push(track);
    }
    const trackArtist = field(row, "artist").value.trim() || artist;
    if (track.artist !== trackArtist) track.artist_ids = [];
    track.artist = trackArtist;
    track.title = field(row, "title").value.trim() || t(...defaultTitle(number));
  }
  return release;
}

/// The title of a track without one: number 0 is the hidden track before track 1.
function defaultTitle(number) {
  return number === 0 ? ["track.hidden", {}] : ["track.default", { number }];
}

// The album artist also changes the tracks that had the same artist.
let previousArtist = "";
$("album-artist").addEventListener("focus", (e) => (previousArtist = e.target.value));
$("album-artist").addEventListener("input", (e) => {
  for (const row of rows()) {
    const input = field(row, "artist");
    if (input.value === previousArtist) input.value = e.target.value;
  }
  previousArtist = e.target.value;
});

// ---------- Ripping ----------

function checkedTracks() {
  return rows()
    .filter((row) => row.cells[0].firstChild.checked)
    .map((row) => Number(row.dataset.number));
}

function updateRipButton() {
  if (!ripping) stopping = false;
  const button = $("rip");
  localize(button, stopping ? "rip.stopping" : ripping ? "rip.stop" : "rip.start");
  button.classList.toggle("stop", ripping);
  button.disabled = stopping || (!ripping && (!disc || checkedTracks().length === 0));
  for (const id of ["refresh", "eject", "browse", "format", "mode", "release", "settings"]) $(id).disabled = ripping;
}

async function startRip() {
  if (disc.offset === null && prefs.offset === null) {
    setMessage("offset.unknownDrive", {}, "bad");
    openPrefs();
    return;
  }
  const tracks = checkedTracks();
  const request = {
    release: editedRelease(),
    tracks: tracks.length === rows().length ? [] : tracks,
  };
  for (const r of rows()) {
    const waiting = tracks.includes(Number(r.dataset.number));
    setStatus(r, waiting ? "track.waiting" : null, {}, "");
  }
  setMessage(null);
  try {
    await invoke("start_rip", { request });
    ripping = true;
    updateRipButton();
  } catch (error) {
    setMessage("rip.failed", { error: messageRef(error) }, "bad");
  }
}

function row(number) {
  return $("tracks").querySelector(`tr[data-number="${number}"]`);
}

/// A track's status: the text `key` (none if null), its color `kind`, and a
/// progress bar if `fraction` is given.
function setStatus(r, key, params, kind, fraction = null, phase = "") {
  const state = r.querySelector(".state");
  if (key) localize(state, key, params);
  else {
    state.removeAttribute("data-i18n");
    state.textContent = "";
  }
  state.className = `state ${kind}`;
  const bar = r.querySelector(".bar");
  bar.hidden = fraction === null;
  bar.className = `bar ${phase}`;
  if (fraction !== null) bar.firstChild.style.width = `${Math.round(fraction * 100)}%`;
}

listen("rip", ({ payload: event }) => {
  switch (event.type) {
    case "progress": {
      const r = row(event.number);
      r.scrollIntoView({ block: "nearest" });
      setStatus(r, `track.${event.phase}`, event.reread ?? {}, "", event.fraction, event.phase);
      break;
    }
    case "trackDone": {
      // Colors: green when proven right, red otherwise.
      const kind = { accurate: "accurate", clean: "clean" }[event.status] ?? "bad";
      setStatus(row(event.number), `track.${event.status}`, { confidence: event.confidence }, kind);
      row(event.number).dataset.read = JSON.stringify(event.read);
      showReadSummary(row(event.number));
      break;
    }
    case "repairing":
      setMessage("rip.repairing", {}, "info");
      break;
    case "hiddenSilent":
      setStatus(row(0), "track.silent", {}, "clean");
      break;
    case "analyzing":
      setMessage("rip.analyzing", {}, "info");
      break;
    case "analyzed":
      setMessage(null);
      break;
    case "warning": {
      const warning = messageRef(event.warning, "warning");
      setMessage(warning.key, warning.params, "info");
      break;
    }
    case "finished": {
      ripping = false;
      lastAlbumDir = event.albumDir;
      const ctdb = event.ctdb;
      // A track AccurateRip did not confirm may still be in the CTDB.
      for (const [number, confidence] of ctdb.tracks) {
        const state = row(number)?.querySelector(".state");
        if (confidence > 0 && state && !state.classList.contains("accurate")) {
          setStatus(row(number), "track.ctdb", { confidence }, "accurate");
        }
      }
      const arPerfect = event.verified ? event.accurate === event.total : event.clean === event.total;
      const perfect = arPerfect || ctdb.disc > 0;
      const found = ctdb.tracks.filter(([, confidence]) => confidence > 0).length;
      const ctdbText =
        ctdb.status !== "verified"
          ? { key: `ctdb.${ctdb.status}` }
          : ctdb.disc > 0
            ? { key: "ctdb.disc", params: { confidence: ctdb.disc, total: ctdb.total } }
            : { key: "ctdb.tracks", params: { found, count: ctdb.tracks.length } };
      const summary = { key: event.verified ? "rip.doneVerified" : "rip.doneUnverified", params: event };
      setMessage("rip.doneWithCtdb", { summary, ctdb: ctdbText }, perfect ? "good" : "bad", true);
      break;
    }
    case "cancelled":
      ripping = false;
      setMessage("rip.cancelled", {}, "info");
      clearPending();
      break;
    case "failed":
      ripping = false;
      setMessage("rip.failed", { error: messageRef(event.error) }, "bad");
      clearPending();
      break;
  }
  updateRipButton();
});

function clearPending() {
  for (const r of rows()) {
    const state = r.querySelector(".state");
    if (!state.classList.contains("accurate") && !state.classList.contains("clean") && !state.classList.contains("bad")) {
      setStatus(r, null, {}, "");
    }
  }
}

languageListeners.push(() => rows().forEach(showReadSummary));

/// How a track was read, as the row's tooltip (in the current language).
function showReadSummary(r) {
  const read = JSON.parse(r.dataset.read || "null");
  if (!read) return;
  const parts = [t("read.passes", { passes: read.passes })];
  const counts = [
    ["read.c2", read.c2Sectors],
    ["read.mismatches", read.mismatches],
    ["read.rereads", read.rereads],
    ["read.suspicious", read.suspicious],
    ["read.unreadable", read.unreadable],
    ["read.cleared", read.cleared],
    ["read.repaired", read.repaired],
  ];
  for (const [key, count] of counts) if (count > 0) parts.push(t(key, { count }));
  r.title = parts.join(", ");
}

/// The message box: the text `key` (hidden if null), colored by `kind`, with
/// an "open folder" button after a rip.
function setMessage(key, params = {}, kind = "info", withFolder = false) {
  const box = $("message");
  box.hidden = !key;
  if (!key) return;
  box.className = `message ${kind}`;
  const text = document.createElement("span");
  localize(text, key, params);
  box.replaceChildren(text);
  if (withFolder) {
    const button = document.createElement("button");
    localize(button, "rip.openFolder");
    button.addEventListener("click", () =>
      invoke("open_folder", { path: lastAlbumDir }).catch((e) =>
        setMessage("rip.openFolderFailed", { error: messageRef(e) }, "bad"),
      ),
    );
    box.append(button);
  }
}

// ---------- Controls ----------

$("rip").addEventListener("click", () => (ripping ? stopRip() : startRip()));

function stopRip() {
  stopping = true;
  updateRipButton();
  invoke("cancel_rip");
}
$("refresh").addEventListener("click", scan);
$("release").addEventListener("change", selectRelease);
$("check-all").addEventListener("change", (e) => {
  for (const r of rows()) r.cells[0].firstChild.checked = e.target.checked;
  updateRipButton();
});

$("eject").addEventListener("click", async () => {
  disc = null;
  updateRipButton();
  showEmpty("empty.ejecting", {}, true);
  try {
    await invoke("eject");
    showEmpty("empty.insert");
  } catch (error) {
    showEmpty("empty.error", { error: messageRef(error) });
  }
});

$("browse").addEventListener("click", async () => {
  const folder = await openDialog({ directory: true, defaultPath: prefs.output });
  if (folder) {
    prefs.output = folder;
    savePrefs();
  }
});
$("format").addEventListener("change", (e) => {
  prefs.format = e.target.value;
  savePrefs();
});
$("mode").addEventListener("change", (e) => {
  prefs.secure = e.target.value === "secure";
  savePrefs();
});

// ---------- Settings ----------

async function savePrefs() {
  showPrefs();
  try {
    await invoke("save_prefs", { prefs });
  } catch (error) {
    setMessage("settings.notSaved", { error: messageRef(error) }, "bad");
  }
}

/// Shows the settings that live in the main window.
function showPrefs() {
  $("output").textContent = prefs.output;
  $("output").title = prefs.output;
  $("format").value = prefs.format;
  $("mode").value = prefs.secure ? "secure" : "burst";
  applyTheme(prefs.theme);
  setLanguage(prefs.language);
  showDrive();
}

function openPrefs() {
  $("pref-folder").value = prefs.naming.folder;
  $("pref-file").value = prefs.naming.file;
  $("pref-manual-offset").checked = prefs.offset !== null;
  $("pref-offset").value = prefs.offset ?? disc?.offset ?? 0;
  $("pref-offset").disabled = prefs.offset === null;
  if (disc?.offset != null) localize($("pref-offset-auto"), "settings.offsetAuto", { offset: signed(disc.offset) });
  else localize($("pref-offset-auto"), "settings.offsetNone");
  $("pref-c2").checked = prefs.c2;
  // Both tests need a disc, and the drive to themselves.
  $("pref-detect-offset").disabled = !disc || ripping;
  $("pref-test-cache").disabled = !disc || ripping;
  localize($("pref-offset-result"), disc ? "offset.hint" : "settings.needDisc");
  if (prefs.cacheKib) localize($("pref-cache-result"), "cache.measured", { kib: prefs.cacheKib });
  else localize($("pref-cache-result"), disc ? "cache.hint" : "settings.needDisc");
  $("pref-mp3-quality").value = prefs.mp3Quality;
  $("pref-embed").checked = prefs.embedCover;
  $("pref-save-cover").checked = prefs.saveCover;
  $("pref-replaygain").checked = prefs.replaygain;
  $("pref-eject").checked = prefs.ejectAfter;
  $("pref-theme").value = prefs.theme;
  $("pref-language").value = prefs.language;
  updatePreview();
  $("prefs").showModal();
}

async function updatePreview() {
  const naming = { folder: $("pref-folder").value, file: $("pref-file").value };
  const release = disc ? editedRelease() : null;
  const path = await invoke("preview_naming", { naming, release }).catch(() => "");
  if (path) {
    $("pref-preview").removeAttribute("data-i18n");
    $("pref-preview").textContent = `${prefs.output}/${path}`;
  } else localize($("pref-preview"), "settings.exampleNone");
}

$("settings").addEventListener("click", openPrefs);
$("pref-folder").addEventListener("input", updatePreview);
$("pref-file").addEventListener("input", updatePreview);
$("pref-manual-offset").addEventListener("change", (e) => ($("pref-offset").disabled = !e.target.checked));

$("pref-detect-offset").addEventListener("click", async () => {
  const result = $("pref-offset-result");
  $("pref-detect-offset").disabled = true;
  localize(result, "offset.reading");
  try {
    const found = await invoke("detect_offset");
    const offsets = found.offsets.map(signed).join(", ");
    if (found.status === "found" && found.offsets[0] === disc.offset) {
      localize(result, "offset.confirmed", { offset: offsets });
    } else {
      localize(result, `offset.${found.status}`, { offset: offsets, offsets });
    }
    if (found.status === "found" && found.offsets[0] !== disc.offset) {
      $("pref-manual-offset").checked = true;
      $("pref-offset").disabled = false;
      $("pref-offset").value = found.offsets[0];
    }
  } catch (error) {
    localize(result, "settings.testFailed", { error: messageRef(error) });
  }
  $("pref-detect-offset").disabled = false;
});
listen("tool", ({ payload: [track, fraction] }) => {
  localize($("pref-offset-result"), "offset.progress", { track, percent: Math.round(fraction * 100) });
});

$("pref-test-cache").addEventListener("click", async () => {
  const result = $("pref-cache-result");
  $("pref-test-cache").disabled = true;
  localize(result, "cache.testing");
  try {
    const test = await invoke("test_cache");
    const params = { disc: Math.round(test.discMs), reread: test.rereadMs.toFixed(1), kib: test.flushKib };
    localize(result, !test.caches ? "cache.none" : test.flushKib === null ? "cache.tooLarge" : "cache.emptied", params);
    // A measure, not a choice: kept at once, whether the dialog is saved or not.
    prefs.cacheKib = test.caches && test.flushKib > test.defaultKib ? test.flushKib : null;
    invoke("save_prefs", { prefs }).catch((error) => setMessage("settings.notSaved", { error: messageRef(error) }, "bad"));
  } catch (error) {
    localize(result, "settings.testFailed", { error: messageRef(error) });
  }
  $("pref-test-cache").disabled = false;
});
// The theme shows at once; cancelling the dialog puts the previous one back.
$("pref-theme").addEventListener("change", (e) => applyTheme(e.target.value));
$("pref-language").addEventListener("change", (e) => setLanguage(e.target.value));

/// Themes other than the classic look, each in ui/themes/<name>/.
const THEMES = ["cathedrale"];

function applyTheme(theme) {
  for (const t of THEMES) document.body.classList.toggle(`theme-${t}`, t === theme);
}

$("prefs").addEventListener("close", () => {
  if ($("prefs").returnValue !== "save") {
    showPrefs();
    return;
  }
  prefs.naming = { folder: $("pref-folder").value.trim(), file: $("pref-file").value.trim() };
  prefs.offset = $("pref-manual-offset").checked ? Number($("pref-offset").value) || 0 : null;
  prefs.c2 = $("pref-c2").checked;
  prefs.mp3Quality = $("pref-mp3-quality").value;
  prefs.embedCover = $("pref-embed").checked;
  prefs.saveCover = $("pref-save-cover").checked;
  prefs.replaygain = $("pref-replaygain").checked;
  prefs.ejectAfter = $("pref-eject").checked;
  prefs.theme = $("pref-theme").value;
  prefs.language = $("pref-language").value;
  savePrefs();
});

function signed(n) {
  return n > 0 ? `+${n}` : String(n);
}

// ---------- Start ----------

(async () => {
  setLanguage("auto"); // until the settings are loaded
  const formats = await invoke("formats");
  for (const option of [...$("format").options]) if (!formats.includes(option.value)) option.remove();
  prefs = await invoke("get_prefs");
  showPrefs();
  scan();
})();
