// Galette's window. Talks to the Rust side (src/main.rs) through Tauri:
// `invoke` calls a command, `listen` receives the events sent while ripping.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: openDialog } = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);

let disc = null; // what the `scan` command returned
let output = "";
let ripping = false;
let lastAlbumDir = null;

// ---------- Disc ----------

async function scan() {
  showEmpty("Lecture du disque…", true);
  disc = null;
  updateRipButton();
  try {
    disc = await invoke("scan");
    showDisc();
  } catch (error) {
    showEmpty(String(error).includes("no disc") ? "Aucun disque dans le lecteur." : `Erreur : ${error}`);
  }
}

function showEmpty(text, busy = false) {
  $("album").hidden = true;
  $("empty").hidden = false;
  $("empty").classList.toggle("busy", busy);
  $("empty-text").textContent = text;
}

function showDisc() {
  $("empty").hidden = true;
  $("album").hidden = false;
  setMessage(disc.warning, "info");

  const offset = disc.offset === null ? "décalage inconnu" : `décalage ${signed(disc.offset)}`;
  $("drive").textContent = `${disc.drive} · ${offset} · C2 ${disc.c2 ? "✓" : "✗"}`;
  $("offset-field").hidden = disc.offset !== null;

  const select = $("release");
  select.replaceChildren();
  disc.releases.forEach((release, i) => select.add(new Option(describe(release), i)));
  select.add(new Option("Saisie manuelle", -1));
  select.value = disc.releases.length ? 0 : -1;

  const badges = $("badges");
  badges.replaceChildren(
    badge(disc.releases.length ? "MusicBrainz ✓" : "Absent de MusicBrainz", disc.releases.length ? "good" : "warn"),
    badge(
      disc.accuraterip === null ? "Absent d'AccurateRip" : `AccurateRip : ${disc.accuraterip} jeux de sommes`,
      disc.accuraterip === null ? "warn" : "good",
    ),
    badge(`${disc.tracks.filter((t) => t.isAudio).length} pistes`),
  );

  buildTracks();
  updateRipButton();
  selectRelease();
}

function describe(release) {
  const parts = [release.country, release.date, release.label, release.catalog_number].filter(Boolean);
  const cd = release.disc_count > 1 ? ` (CD ${release.disc_number}/${release.disc_count})` : "";
  return `${release.artist} – ${release.title}${cd} · ${parts.join(" · ")}`;
}

function badge(text, kind = "") {
  const span = document.createElement("span");
  span.className = `badge ${kind}`;
  span.textContent = text;
  return span;
}

function currentRelease() {
  const i = Number($("release").value);
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
    row.insertCell().append(textInput("title", track.isAudio ? "" : "Piste de données"));
    row.insertCell().append(textInput("artist"));
    row.insertCell().textContent = track.length;

    const status = row.insertCell();
    status.innerHTML = `<div class="state"></div><div class="bar" hidden><div></div></div>`;
  }
  $("check-all").checked = true;
}

function textInput(field, placeholder = "") {
  const input = document.createElement("input");
  input.dataset.field = field;
  input.placeholder = placeholder;
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
    field(row, "title").placeholder = `Piste ${row.dataset.number}`;
    field(row, "artist").value = info?.artist ?? release?.artist ?? "";
  }
  await loadCover(release);
}

async function loadCover(release) {
  const img = $("cover");
  img.hidden = true;
  $("cover-none").hidden = false;
  if (!release) return;
  $("cover-none").textContent = "Chargement…";
  const url = await invoke("cover", { release }).catch(() => null);
  if (currentRelease() !== release) return; // another edition was chosen meanwhile
  $("cover-none").textContent = "Pas de pochette";
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
    track.title = field(row, "title").value.trim() || `Piste ${number}`;
  }
  return release;
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
  const button = $("rip");
  button.textContent = ripping ? "Arrêter" : "Ripper";
  button.classList.toggle("stop", ripping);
  button.disabled = !ripping && (!disc || checkedTracks().length === 0);
  for (const id of ["refresh", "eject", "browse", "format", "mode", "release"]) $(id).disabled = ripping;
}

async function startRip() {
  const tracks = checkedTracks();
  const request = {
    release: editedRelease(),
    tracks: tracks.length === rows().length ? [] : tracks,
    output,
    format: $("format").value,
    secure: $("mode").value === "secure",
    offset: disc.offset === null ? Number($("offset").value) : null,
  };
  for (const r of rows()) {
    const waiting = tracks.includes(Number(r.dataset.number));
    setStatus(r, waiting ? "En attente" : "", "");
  }
  setMessage(null);
  try {
    await invoke("start_rip", { request });
    ripping = true;
    updateRipButton();
  } catch (error) {
    setMessage(String(error), "bad");
  }
}

function row(number) {
  return $("tracks").querySelector(`tr[data-number="${number}"]`);
}

function setStatus(r, text, kind, fraction = null, phase = "") {
  const state = r.querySelector(".state");
  state.textContent = text;
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
      setStatus(r, event.detail, "", event.fraction, event.phase);
      break;
    }
    case "trackDone":
      setStatus(row(event.number), event.text, event.status);
      row(event.number).title = event.read;
      break;
    case "warning":
      setMessage(event.message, "info");
      break;
    case "finished": {
      ripping = false;
      lastAlbumDir = event.albumDir;
      const summary = event.verified
        ? `${event.accurate}/${event.total} piste(s) vérifiée(s) par AccurateRip.`
        : `${event.clean}/${event.total} piste(s) sans erreur de lecture (disque absent d'AccurateRip).`;
      const perfect = event.verified ? event.accurate === event.total : event.clean === event.total;
      setMessage(`Terminé. ${summary}`, perfect ? "good" : "bad", true);
      break;
    }
    case "cancelled":
      ripping = false;
      setMessage("Rip arrêté.", "info");
      clearPending();
      break;
    case "failed":
      ripping = false;
      setMessage(`Échec du rip : ${event.message}`, "bad");
      clearPending();
      break;
  }
  updateRipButton();
});

function clearPending() {
  for (const r of rows()) {
    const state = r.querySelector(".state");
    if (!state.classList.contains("accurate") && !state.classList.contains("clean") && !state.classList.contains("bad")) {
      setStatus(r, "", "");
    }
  }
}

function setMessage(text, kind = "info", withFolder = false) {
  const box = $("message");
  box.hidden = !text;
  if (!text) return;
  box.className = `message ${kind}`;
  box.replaceChildren(document.createTextNode(text));
  if (withFolder) {
    const button = document.createElement("button");
    button.textContent = "Ouvrir le dossier";
    button.addEventListener("click", () => invoke("open_folder", { path: lastAlbumDir }));
    box.append(button);
  }
}

// ---------- Controls ----------

$("rip").addEventListener("click", () => (ripping ? invoke("cancel_rip") : startRip()));
$("refresh").addEventListener("click", scan);
$("release").addEventListener("change", selectRelease);
$("check-all").addEventListener("change", (e) => {
  for (const r of rows()) r.cells[0].firstChild.checked = e.target.checked;
  updateRipButton();
});

$("eject").addEventListener("click", async () => {
  disc = null;
  updateRipButton();
  showEmpty("Éjection…", true);
  try {
    await invoke("eject");
    showEmpty("Insérez un CD audio puis cliquez sur Actualiser.");
  } catch (error) {
    showEmpty(`Erreur : ${error}`);
  }
});

$("browse").addEventListener("click", async () => {
  const folder = await openDialog({ directory: true, defaultPath: output });
  if (folder) setOutput(folder);
});

function setOutput(path) {
  output = path;
  $("output").textContent = path;
  $("output").title = path;
  try {
    localStorage.setItem("output", path);
  } catch {}
}

function signed(n) {
  return n > 0 ? `+${n}` : String(n);
}

// ---------- Start ----------

(async () => {
  let saved = null;
  try {
    saved = localStorage.getItem("output");
  } catch {}
  setOutput(saved || (await invoke("default_output")));
  scan();
})();
