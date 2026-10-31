# À faire plus tard

Liste des choses repérées mais pas encore faites ou pas encore vérifiées.

## À tester, par système

Ce qui n'a pas encore été essayé à la main sur chaque système. À chaque
nouvelle fonction, on l'ajoute ici pour les systèmes où elle n'a pas été
testée, et on la raye une fois vérifiée.

### Linux (Debian)

- [x] ReplayGain : *Reload* entier en FLAC dans la fenêtre (−10,78 et
  −10,96 dB, album −10,28 dB comme sur Windows ; tags présents, `flac -t`
  sans erreur) ; pistes 1-2 en MP3, ALAC et AIFF (tags présents, lus par
  mutagen), rien en WAV ; la case des réglages les désactive.
- [x] Recherche du décalage : *Brand New Eyes* (4 candidats) puis *Reload*
  donnent +6, dans la fenêtre comme en ligne de commande.
- [x] Test du cache : `galette cache` avec l'ASUS (133 ms depuis le disque,
  4 ms en relecture, 2 Mio suffisent) et dans la fenêtre.
- [ ] Piste cachée avant la piste 1 (HTOA, nouveau) : sur un disque qui en a
  une, la ligne « Piste cachée » apparaît en tête de liste ; elle est rippée
  en piste 00 (ou notée « Silence, pas de fichier »), le cue la référence
  et le log a une section « Track 00 ».
- [ ] Pré-accentuation (nouveau) : sur un disque concerné (rare, surtout des CD
  des années 80), un avertissement s'affiche, le cue a « FLAGS PRE » et le
  log le signale.
- [x] Réparation CTDB : *Reload*, pistes 11 (71 819 échantillons corrigés)
  et 12 (59) réparées puis confirmées par AccurateRip.
- [x] Disque abîmé en mode sécurisé : *Reload* en 12 min, 13/13 pistes
  confirmées.
- [x] Mode rapide sur disque abîmé : *Reload* pistes 11 et 12, les 931 et
  19 secteurs avec erreurs C2 sont notés suspects dans le log.
- [x] CTDB : *Reload* entier, 13/13 pistes trouvées et confirmées.
- [x] CTDB : un rip de quelques pistes dit que seul le disque entier est
  vérifié.
- [ ] CTDB : un disque absent du CTDB.
- [x] Langue : anglais, rip, retour au français.
- [x] Message sans lecteur.
- [x] Formats ALAC, AIFF et WAV : pistes 1-2 de *Reload* en ligne de
  commande, toutes confirmées par AccurateRip, pochette et tags présents
  (sauf WAV), durées exactes.
- [ ] CD-Text (nouveau) : sur un disque qui en a, la liste Édition propose
  « CD-Text du disque : … » et le badge « CD-Text ✓ » s'affiche ; le log a
  une ligne « CD-Text ». Vérifier aussi qu'un disque sans CD-Text se lit
  normalement (`galette info` affiche le CD-Text s'il y en a).
- [x] Thème Cathédrale.
- [x] Rip d'un album déjà rippé dans un autre format : chacun dans
  « Metallica - Reload (FORMAT) », le FLAC intact.

### macOS

- [x] Version 0.5.0 : le .dmg installé dans Applications fonctionne.
- [ ] ReplayGain : disque entier en FLAC testé (*Brand New Eyes*, gain
  d'album −11,15 dB). Reste : autres formats, case des réglages.
- [x] Recherche du décalage : *Brand New Eyes* (Paramore) puis *Reload*
  donnent +6, dans la fenêtre comme en ligne de commande.
- [x] Test du cache : `galette cache` avec l'ASUS (122 ms depuis le disque,
  3,4 ms en relecture ; 2 Mio lus ailleurs le vident). Reste à essayer dans
  la fenêtre.
- [ ] Piste cachée avant la piste 1 (HTOA, nouveau) : sur un disque qui en a
  une, la ligne « Piste cachée » apparaît en tête de liste ; elle est rippée
  en piste 00 (ou notée « Silence, pas de fichier »), le cue la référence
  et le log a une section « Track 00 ».
- [ ] Pré-accentuation (nouveau) : sur un disque concerné (rare, surtout des CD
  des années 80), un avertissement s'affiche, le cue a « FLAGS PRE » et le
  log le signale.
- [x] Réparation CTDB : testée avec *Reload* (Metallica), pistes 11 et 12
  réparées puis confirmées par AccurateRip.
- [x] Disque abîmé en mode sécurisé : testé avec *Reload* (Metallica), 12/13
  pistes confirmées, la piste rayée finit avec ses secteurs suspects.
- [ ] Mode rapide sur disque abîmé (nouveau) : les secteurs avec erreurs C2
  doivent être notés suspects dans le log.
- [x] CTDB, disque entier et confirmation piste par piste.
- [ ] CTDB : un disque absent du CTDB et un rip de quelques pistes seulement.
- [ ] Message sans lecteur : débrancher le lecteur, la fenêtre doit dire
  « Aucun lecteur détecté. Branchez un lecteur CD. » (nouveau).
- [ ] Bouton Arrêter pendant un rip (corrigé, mais jamais confirmé dans la
  fenêtre sur Mac).
- [ ] Format WAV.
- [ ] Rip d'un album déjà rippé dans un autre format : le nouveau doit aller
  dans « Album (FORMAT) » sans toucher au premier.

### Windows

- [x] ReplayGain : *Reload* entier en FLAC (−10,78 et −10,96 dB comme sur
  Mac, album −10,28 dB), pistes 1-2 en MP3, ALAC et AIFF (tags présents),
  rien en WAV ; la case des réglages les désactive.
- [ ] Recherche du décalage : marche (17 candidats avec *Reload*, 27 avec
  le Metallica noir), mais **deux disques ne donnent pas +6** : +673 en
  ligne de commande (*Reload* + Metallica noir), +12 dans la fenêtre. Voir
  « Plus tard ».
- [x] Test du cache : `galette cache` (162 ms depuis le disque, 3,5 ms en
  relecture, 2 Mio suffisent) et dans la fenêtre.
- [x] Piste cachée avant la piste 1 (HTOA).
- [ ] Pré-accentuation : pas de disque concerné sous la main. Un
  avertissement doit s'afficher, le cue avoir « FLAGS PRE » et le log le
  signaler.
- [x] Réparation CTDB : *Reload*, pistes 11 (70 865 échantillons corrigés)
  et 12 réparées puis confirmées par AccurateRip.
- [x] Disque abîmé en mode sécurisé : *Reload* en 13 min, 13/13 pistes
  confirmées.
- [ ] Mode rapide sur disque abîmé : les secteurs avec erreurs C2 doivent
  être notés suspects dans le log.
- [x] CTDB : *Reload* 13/13 confirmées ; un rip de quelques pistes dit que
  seul le disque entier est vérifié.
- [ ] CTDB : un disque absent du CTDB.
- [x] Dans l'application installée : bouton Arrêter pendant un rip,
  éjection, « Ouvrir le dossier », désinstallation.
- [ ] Un lecteur autre que l'ASUS.
- [x] Langue : anglais, rip, retour au français.
- [x] Message sans lecteur.
- [x] Formats ALAC, MP3, AIFF et WAV (pistes 1-2 de *Reload*, toutes
  confirmées par AccurateRip).
- [x] Thème Cathédrale.
- [x] Rip d'un album déjà rippé dans un autre format : chacun dans
  « Metallica - Reload (FORMAT) », le FLAC intact.
- [x] CD-Text.

## Plus tard

- [ ] Recherche du décalage à peaufiner : avec deux disques, l'ASUS (+6)
  sort à +673 (*Reload* + Metallica noir, en ligne de commande) ou à +12
  (dans la fenêtre). Cause trouvée pour +673 : c'est +6 + 667, et +667 est
  le décalage de 481 modèles de lecteurs. Des rips de lecteurs à +667 mal
  réglés se retrouvent dans AccurateRip, sur tous les disques, avec une forte
  confiance : l'idée que les décalages parasites ne se répètent pas d'un
  disque à l'autre ne tient pas. Piste : écarter un candidat égal à un autre
  plus un décalage de lecteur connu (673 = 6 + 667, 12 = 6 + 6), sauf
  pour les petits écarts.
- [ ] Icône de la fenêtre : sous Linux (KDE), l'icône en haut à gauche de
  la fenêtre est à mettre à jour.
- [ ] Ordre des tags FLAC : l'ajout des tags ReplayGain (`tags::update_flac`,
  crate metaflac) réécrit tous les commentaires Vorbis dans un ordre
  différent à chaque fichier. Sans effet sur les lecteurs, mais peu soigné.
- [ ] Aide de `galette rip` : la description dit encore « to FLAC (or WAV)
  files » alors que tous les formats sont proposés.
- [ ] Pochette : Cover Art Archive a répondu une fois HTTP 500 (erreur
  passagère) et le rip est resté sans pochette ; `http.rs` ne réessaie que
  sur 503 et 429, il pourrait réessayer sur toute erreur 5xx.

- [ ] Désaccentuation : appliquer le filtre inverse aux pistes avec
  pré-accentuation, en option (utile pour MP3/AAC ; en FLAC on garde l'audio
  du disque tel quel, comme EAC).
- [ ] Pré-accentuation signalée seulement dans le sous-canal Q (et pas dans la
  TOC) : très rare, non détectée.
