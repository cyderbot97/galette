# À faire plus tard

Liste des choses repérées mais pas encore faites ou pas encore vérifiées.

## À tester

- [ ] **MP3** : pas encore testé à la main. Vérifier dans un lecteur (app Musique,
  foobar2000…) les tags, la pochette et la lecture sans blanc entre les pistes.
  Les tests automatiques passent et un fichier a été vérifié (tags ID3 complets,
  durée exacte grâce à l'en-tête LAME).

## Publication

- [ ] Ajouter les paquets Linux (`.deb`, AppImage) à la release 0.2.0, à construire
  sur Debian, puis retirer des notes la phrase qui renvoie Linux vers la 0.1.0.

## Améliorations

- [ ] Réessayer automatiquement quand MusicBrainz répond « HTTP 503 » (limite d'une
  requête par seconde) au lieu d'abandonner le rip.

## Nouveaux formats (dans cet ordre)

- [ ] Opus (libopus, rééchantillonnage en 48 kHz)
- [ ] ALAC (`.m4a`, Apple Lossless, toutes plateformes)
- [ ] AAC (`.m4a`, **macOS uniquement**, l'option doit être absente ailleurs)
- [ ] WavPack

## Plus tard

- [ ] CTDB : vérification puis réparation des rips (protocole déjà étudié,
  commencer par la vérification seule).
- [ ] Windows.
- [ ] HTOA (piste cachée avant la piste 1), CD-Text, pré-accentuation.
