// The window's texts in each language. Elements name their text with
// `data-i18n` (and `data-i18n-title`, `data-i18n-placeholder`), plus
// `data-i18n-params` (JSON) for the `{name}` placeholders: `translatePage`
// fills them all, so changing the language rewrites the whole window.
// `data-i18n-html` is for our own texts holding markup (<code>).

const STRINGS = {
  fr: {
    // Header and empty state
    "header.refresh": "↻ Actualiser",
    "header.refreshTitle": "Relire le disque",
    "header.eject": "⏏ Éjecter",
    "header.ejectTitle": "Ouvrir le tiroir",
    "header.settings": "⚙ Réglages",
    "header.settingsTitle": "Réglages",
    "drive.line": "{drive} · {offset} · C2 {c2}",
    "drive.offsetUnknown": "décalage inconnu",
    "drive.offset": "décalage {offset}",
    "drive.offsetManual": "décalage {offset} (manuel)",
    "empty.insert": "Insérez un CD audio.",
    "empty.reading": "Lecture du disque…",
    "empty.loading": "Chargement du disque…",
    "empty.ejecting": "Éjection…",
    "empty.noDrive": "Aucun lecteur détecté. Branchez un lecteur CD.",
    "empty.error": "Erreur : {error}",

    // Album
    "album.noCover": "Pas de pochette",
    "album.coverLoading": "Chargement…",
    "album.edition": "Édition",
    "album.manual": "Saisie manuelle",
    "album.artist": "Artiste",
    "album.title": "Titre de l'album",
    "album.year": "Année",
    "album.yearPlaceholder": "AAAA",
    "badge.musicbrainz": "MusicBrainz ✓",
    "badge.noMusicbrainz": "Absent de MusicBrainz",
    "badge.accuraterip": "AccurateRip : {sets} jeux de sommes",
    "badge.noAccuraterip": "Absent d'AccurateRip",
    "badge.tracks": "{count} pistes",

    // Tracks
    "tracks.title": "Titre",
    "tracks.artist": "Artiste",
    "tracks.length": "Durée",
    "tracks.status": "État",
    "track.data": "Piste de données",
    "track.default": "Piste {number}",
    "track.waiting": "En attente",
    "track.reading": "Lecture",
    "track.verifying": "Seconde lecture",
    "track.rereading": "Relecture de {sectors} secteur(s), {attempt}/{maxAttempts}",
    "track.encoding": "Encodage",
    "track.accurate": "AccurateRip ✓ ({confidence})",
    "track.mismatch": "Non conforme à AccurateRip",
    "track.clean": "Lectures identiques ✓",
    "track.errors": "Erreurs de lecture",
    "read.passes": "{passes} lecture(s)",
    "read.c2": "{count} secteur(s) avec erreurs C2",
    "read.mismatches": "{count} secteur(s) lus différemment",
    "read.rereads": "{count} relecture(s)",
    "read.suspicious": "{count} SUSPECT(S)",
    "read.unreadable": "{count} ILLISIBLE(S)",
    "read.cleared": "{count} secteur(s) incertain(s) confirmé(s) par AccurateRip",

    // Footer
    "footer.folder": "Dossier",
    "footer.browse": "Parcourir…",
    "footer.format": "Format",
    "footer.mode": "Mode",
    "footer.secure": "Sécurisé",
    "footer.burst": "Rapide",
    "rip.start": "Ripper",
    "rip.stop": "Arrêter",
    "rip.stopping": "Arrêt…",

    // Messages while and after ripping
    "rip.analyzing": "Analyse du disque (pré-gaps, ISRC)…",
    "rip.doneVerified": "Terminé. {accurate}/{total} piste(s) vérifiée(s) par AccurateRip.",
    "rip.doneUnverified": "Terminé. {clean}/{total} piste(s) sans erreur de lecture (disque absent d'AccurateRip).",
    "rip.cancelled": "Rip arrêté.",
    "rip.failed": "Échec du rip : {error}",
    "rip.openFolder": "Ouvrir le dossier",
    "rip.openFolderFailed": "Impossible d'ouvrir le dossier : {error}",
    "settings.notSaved": "Réglages non enregistrés : {error}",

    // Errors (codes from src/main.rs), then the engine's detail if any
    "error.noDrive": "aucun lecteur CD détecté. Branchez un lecteur CD.",
    "error.noDisc": "aucun disque dans le lecteur.",
    "error.scsi": "le lecteur a refusé une commande",
    "error.driveNotResponding": "le lecteur ne répond plus. Débranchez-le, rebranchez-le, puis cliquez sur Actualiser.",
    "error.io": "erreur d'accès",
    "error.invalidToc": "la table des matières du disque est invalide",
    "error.encode": "erreur d'encodage",
    "error.network": "erreur réseau",
    "error.cancelled": "opération annulée.",
    "error.unsupported": "cette fonction n'est pas encore disponible sur ce système.",
    "error.noScan": "aucun disque analysé.",
    "error.unknownOffset": "ce lecteur n'est pas dans la base d'AccurateRip : indiquez son décalage dans les réglages.",
    "error.alreadyRipping": "un rip est déjà en cours.",
    "error.ejectWhileRipping": "impossible d'éjecter pendant un rip.",
    "error.saveSettings": "impossible d'écrire le fichier des réglages",
    "error.openFolder": "impossible d'ouvrir le dossier",
    "error.internal": "erreur interne",
    "error.lookup": "Recherche en ligne impossible",
    "error.detail": "{text} ({detail})",
    "offset.unknownDrive": "Ce lecteur n'est pas dans la base d'AccurateRip : indiquez son décalage de lecture.",

    // Warnings during a rip
    "warning.coverNotSaved": "Impossible d'enregistrer la pochette",
    "warning.hiddenAudio": "Piste cachée avant la piste 1 non rippée",
    "warning.mcnUnreadable": "Impossible de lire le numéro de catalogue du disque",
    "warning.pregapsUnreadable": "Impossible de lire les pré-gaps : les pistes commencent à leur index 1",

    // Settings
    "settings.title": "Réglages",
    "settings.naming": "Nommage",
    "settings.albumFolder": "Dossier de l'album",
    "settings.trackFile": "Fichier de chaque piste",
    "settings.fields":
      "Champs : <code>{albumartist}</code> <code>{album}</code> <code>{year}</code> <code>{disc}</code> " +
      "<code>{disccount}</code>, et pour les fichiers <code>{track}</code> <code>{title}</code> <code>{artist}</code>. " +
      "Un <code>/</code> dans le dossier crée des sous-dossiers.",
    "settings.example": "Exemple : ",
    "settings.exampleNone": "(insérez un disque pour voir un exemple)",
    "settings.reading": "Lecture",
    "settings.manualOffset": "Décalage de lecture manuel",
    "settings.offsetAuto": "(AccurateRip : {offset})",
    "settings.offsetNone": "(lecteur inconnu d'AccurateRip)",
    "settings.c2": "Utiliser les pointeurs C2",
    "settings.c2Hint": "(à décocher si le lecteur les gère mal)",
    "settings.quality": "Qualité",
    "settings.mp3V0": "VBR V0 (≈ 245 kbit/s, recommandé)",
    "settings.mp3V2": "VBR V2 (≈ 190 kbit/s)",
    "settings.mp3Cbr320": "CBR 320 kbit/s",
    "settings.coverAndDisc": "Pochette et disque",
    "settings.embedCover": "Intégrer la pochette dans les fichiers (sauf WAV)",
    "settings.saveCover": "Enregistrer la pochette (cover.jpg)",
    "settings.eject": "Éjecter le disque à la fin du rip",
    "settings.appearance": "Apparence",
    "settings.theme": "Thème",
    "settings.themeClassic": "Classique",
    "settings.themeCathedrale": "Cathédrale : vitraux, ogives et cierges",
    "settings.language": "Langue",
    "settings.languageAuto": "Automatique (langue du système)",
    "settings.cancel": "Annuler",
    "settings.save": "Enregistrer",
  },

  en: {
    "header.refresh": "↻ Refresh",
    "header.refreshTitle": "Read the disc again",
    "header.eject": "⏏ Eject",
    "header.ejectTitle": "Open the tray",
    "header.settings": "⚙ Settings",
    "header.settingsTitle": "Settings",
    "drive.line": "{drive} · {offset} · C2 {c2}",
    "drive.offsetUnknown": "unknown offset",
    "drive.offset": "offset {offset}",
    "drive.offsetManual": "offset {offset} (manual)",
    "empty.insert": "Insert an audio CD.",
    "empty.reading": "Reading the disc…",
    "empty.loading": "Loading the disc…",
    "empty.ejecting": "Ejecting…",
    "empty.noDrive": "No drive found. Plug in a CD drive.",
    "empty.error": "Error: {error}",

    "album.noCover": "No cover",
    "album.coverLoading": "Loading…",
    "album.edition": "Release",
    "album.manual": "Manual entry",
    "album.artist": "Artist",
    "album.title": "Album title",
    "album.year": "Year",
    "album.yearPlaceholder": "YYYY",
    "badge.musicbrainz": "MusicBrainz ✓",
    "badge.noMusicbrainz": "Not in MusicBrainz",
    "badge.accuraterip": "AccurateRip: {sets} checksum sets",
    "badge.noAccuraterip": "Not in AccurateRip",
    "badge.tracks": "{count} tracks",

    "tracks.title": "Title",
    "tracks.artist": "Artist",
    "tracks.length": "Length",
    "tracks.status": "Status",
    "track.data": "Data track",
    "track.default": "Track {number}",
    "track.waiting": "Waiting",
    "track.reading": "Reading",
    "track.verifying": "Second read",
    "track.rereading": "Re-reading {sectors} sector(s), {attempt}/{maxAttempts}",
    "track.encoding": "Encoding",
    "track.accurate": "AccurateRip ✓ ({confidence})",
    "track.mismatch": "Does not match AccurateRip",
    "track.clean": "Identical reads ✓",
    "track.errors": "Read errors",
    "read.passes": "{passes} pass(es)",
    "read.c2": "{count} sector(s) with C2 errors",
    "read.mismatches": "{count} sector(s) read differently",
    "read.rereads": "{count} re-read(s)",
    "read.suspicious": "{count} SUSPICIOUS",
    "read.unreadable": "{count} UNREADABLE",
    "read.cleared": "{count} uncertain sector(s) proven right by AccurateRip",

    "footer.folder": "Folder",
    "footer.browse": "Browse…",
    "footer.format": "Format",
    "footer.mode": "Mode",
    "footer.secure": "Secure",
    "footer.burst": "Fast",
    "rip.start": "Rip",
    "rip.stop": "Stop",
    "rip.stopping": "Stopping…",

    "rip.analyzing": "Analyzing the disc (pregaps, ISRC)…",
    "rip.doneVerified": "Done. {accurate}/{total} track(s) verified by AccurateRip.",
    "rip.doneUnverified": "Done. {clean}/{total} track(s) without read errors (disc not in AccurateRip).",
    "rip.cancelled": "Rip stopped.",
    "rip.failed": "Rip failed: {error}",
    "rip.openFolder": "Open folder",
    "rip.openFolderFailed": "Cannot open the folder: {error}",
    "settings.notSaved": "Settings not saved: {error}",

    "error.noDrive": "no CD drive found. Plug in a CD drive.",
    "error.noDisc": "no disc in the drive.",
    "error.scsi": "the drive refused a command",
    "error.driveNotResponding": "the drive stopped responding. Unplug it, plug it back in, then click Refresh.",
    "error.io": "access error",
    "error.invalidToc": "the disc's table of contents is invalid",
    "error.encode": "encoding error",
    "error.network": "network error",
    "error.cancelled": "cancelled.",
    "error.unsupported": "this feature is not available on this system yet.",
    "error.noScan": "no disc has been read.",
    "error.unknownOffset": "this drive is not in the AccurateRip database: enter its offset in the settings.",
    "error.alreadyRipping": "a rip is already running.",
    "error.ejectWhileRipping": "cannot eject while ripping.",
    "error.saveSettings": "cannot write the settings file",
    "error.openFolder": "cannot open the folder",
    "error.internal": "internal error",
    "error.lookup": "Online lookup failed",
    "error.detail": "{text} ({detail})",
    "offset.unknownDrive": "This drive is not in the AccurateRip database: enter its read offset.",

    "warning.coverNotSaved": "Cannot save the cover",
    "warning.hiddenAudio": "Hidden track before track 1 not ripped",
    "warning.mcnUnreadable": "Cannot read the disc's catalog number",
    "warning.pregapsUnreadable": "Cannot read the pregaps: tracks start at their index 1",

    "settings.title": "Settings",
    "settings.naming": "Naming",
    "settings.albumFolder": "Album folder",
    "settings.trackFile": "File of each track",
    "settings.fields":
      "Fields: <code>{albumartist}</code> <code>{album}</code> <code>{year}</code> <code>{disc}</code> " +
      "<code>{disccount}</code>, and for files <code>{track}</code> <code>{title}</code> <code>{artist}</code>. " +
      "A <code>/</code> in the folder creates subfolders.",
    "settings.example": "Example: ",
    "settings.exampleNone": "(insert a disc to see an example)",
    "settings.reading": "Reading",
    "settings.manualOffset": "Manual read offset",
    "settings.offsetAuto": "(AccurateRip: {offset})",
    "settings.offsetNone": "(drive unknown to AccurateRip)",
    "settings.c2": "Use C2 pointers",
    "settings.c2Hint": "(uncheck if the drive handles them badly)",
    "settings.quality": "Quality",
    "settings.mp3V0": "VBR V0 (≈ 245 kbit/s, recommended)",
    "settings.mp3V2": "VBR V2 (≈ 190 kbit/s)",
    "settings.mp3Cbr320": "CBR 320 kbit/s",
    "settings.coverAndDisc": "Cover and disc",
    "settings.embedCover": "Embed the cover in the files (except WAV)",
    "settings.saveCover": "Save the cover (cover.jpg)",
    "settings.eject": "Eject the disc when the rip is done",
    "settings.appearance": "Appearance",
    "settings.theme": "Theme",
    "settings.themeClassic": "Classic",
    "settings.themeCathedrale": "Cathedral: stained glass, arches and candles",
    "settings.language": "Language",
    "settings.languageAuto": "Automatic (system language)",
    "settings.cancel": "Cancel",
    "settings.save": "Save",
  },
};

/// The language shown: "fr" or "en".
let language = "fr";

/// `choice` is the setting: "fr", "en" or "auto" (the system's language,
/// French if it is French, English otherwise).
function setLanguage(choice) {
  const system = (navigator.language || "fr").toLowerCase().startsWith("fr") ? "fr" : "en";
  language = choice === "fr" || choice === "en" ? choice : system;
  document.documentElement.lang = language;
  translatePage();
  for (const listener of languageListeners) listener();
}

/// The text of `key` with its `{name}` placeholders replaced by `params`.
/// A parameter may itself be a text to translate, `{ key, params }`.
/// Placeholders without a parameter stay as they are (the naming fields).
function t(key, params = {}) {
  const text = STRINGS[language][key] ?? STRINGS.fr[key] ?? key;
  return text.replace(/\{(\w+)\}/g, (all, name) => {
    if (!(name in params)) return all;
    const value = params[name];
    return value?.key ? t(value.key, value.params) : String(value ?? "");
  });
}

/// A message from the Rust side (`Message` in src/main.rs) as a text to
/// translate, `{ key, params }`; `prefix` is "error" or "warning".
function messageRef(message, prefix = "error") {
  if (!message || typeof message !== "object") return { key: "error.internal" };
  const text = { key: `${prefix}.${message.code}` };
  return message.detail ? { key: "error.detail", params: { text, detail: message.detail } } : text;
}

/// Run after every change of language, for texts that are not elements'
/// (tooltips built from several parts).
const languageListeners = [];

/// Gives `element` the text `key` (kept on the element, so that a change of
/// language translates it again). `attribute`: "text", "title" or "placeholder".
function localize(element, key, params = {}, attribute = "text") {
  const suffix = attribute === "text" ? "" : `-${attribute}`;
  element.setAttribute(`data-i18n${suffix}`, key);
  element.setAttribute(`data-i18n${suffix}-params`, JSON.stringify(params));
  applyText(element, attribute);
}

function applyText(element, attribute) {
  const suffix = attribute === "text" ? "" : `-${attribute}`;
  const key = element.getAttribute(`data-i18n${suffix}`);
  const params = JSON.parse(element.getAttribute(`data-i18n${suffix}-params`) || "{}");
  if (attribute === "text") element.textContent = t(key, params);
  else if (attribute === "html") element.innerHTML = t(key, params);
  else element.setAttribute(attribute, t(key, params));
}

function translatePage() {
  for (const attribute of ["text", "html", "title", "placeholder"]) {
    const name = attribute === "text" ? "data-i18n" : `data-i18n-${attribute}`;
    for (const element of document.querySelectorAll(`[${name}]`)) applyText(element, attribute);
  }
}
