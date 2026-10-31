# À faire plus tard

Liste des choses repérées mais pas encore faites ou pas encore vérifiées.

## À tester, par système

Ce qui n'a pas encore été essayé à la main sur chaque système. À chaque
nouvelle fonction, on l'ajoute ici pour les systèmes où elle n'a pas été
testée, et on la raye une fois vérifiée.

### Linux (Debian)

- [ ] ReplayGain (nouveau) : après un rip du disque entier, les fichiers ont
  les tags REPLAYGAIN_TRACK_* et REPLAYGAIN_ALBUM_* (un lecteur comme
  foobar2000 ou VLC les lit) ; le log a une ligne « ReplayGain » par piste et
  une pour l'album. Essayer FLAC, MP3, ALAC/AAC et AIFF ; la case des
  réglages doit les désactiver.
- [ ] Recherche du décalage (nouveau) : Réglages → Lecture → « Détecter avec
  ce disque » (ou `galette offset`), avec un disque connu d'AccurateRip.
  Si plusieurs décalages sont possibles, recommencer avec un autre disque :
  il doit n'en rester qu'un, celui du lecteur.
- [ ] Test du cache (nouveau) : Réglages → Lecture → « Tester le cache du
  lecteur » (ou `galette cache`) ; le résultat doit s'afficher en quelques
  secondes.
- [ ] Piste cachée avant la piste 1 (HTOA, nouveau) : sur un disque qui en a
  une, la ligne « Piste cachée » apparaît en tête de liste ; elle est rippée
  en piste 00 (ou notée « Silence, pas de fichier »), le cue la référence
  et le log a une section « Track 00 ».
- [ ] Pré-accentuation (nouveau) : sur un disque concerné (rare, surtout des CD
  des années 80), un avertissement s'affiche, le cue a « FLAGS PRE » et le
  log le signale.
- [ ] Réparation CTDB (nouveau) : sur un disque abîmé rippé en entier, les
  pistes abîmées attendent la fin (« Abîmée, réparation à la fin »), puis
  sont réparées si possible ; le log dit « REPAIRED from CTDB parity ».
- [ ] Disque abîmé en mode sécurisé (nouveau) : les relectures abandonnent un
  secteur jamais lu sans erreur après 5 tours ; le rip doit finir en un temps
  raisonnable et le log noter les secteurs suspects ou illisibles. En mode
  rapide, les secteurs avec erreurs C2 doivent aussi être notés suspects.
- [ ] CTDB (nouveau) : après un rip du disque entier, le message final et le
  log disent ce qu'en pense le CTDB (« disque confirmé (n sur m) ») ; une piste
  absente d'AccurateRip mais trouvée dans le CTDB affiche « CTDB ✓ ». Essayer
  aussi un disque absent du CTDB et un rip de quelques pistes seulement.
- [ ] Langue : passer en anglais dans Réglages → Apparence → Langue, faire un
  rip, revenir en français (nouveau).
- [ ] Message sans lecteur : débrancher le lecteur, la fenêtre doit dire
  « Aucun lecteur détecté. Branchez un lecteur CD. » (nouveau).
- [ ] Formats ALAC, AIFF et WAV (seuls FLAC et MP3 ont été rippés sous Linux).
- [ ] CD-Text (nouveau) : sur un disque qui en a, la liste Édition propose
  « CD-Text du disque : … » et le badge « CD-Text ✓ » s'affiche ; le log a
  une ligne « CD-Text ». Vérifier aussi qu'un disque sans CD-Text se lit
  normalement (`galette info` affiche le CD-Text s'il y en a).
- [ ] Thème Cathédrale.
- [ ] Rip d'un album déjà rippé dans un autre format : le nouveau doit aller
  dans « Album (FORMAT) » sans toucher au premier.

### macOS

- [ ] ReplayGain (nouveau) : `galette rip --tracks 1,2` en FLAC testé avec
  *Reload* (−10,78 et −10,96 dB, fichiers intacts). Reste : disque entier
  (gain d'album), autres formats, case des réglages.
- [x] Recherche du décalage : *Reload* (17 décalages possibles) puis
  *Brand New Eyes* (Paramore) donnent +6, en ligne de commande. Dans la
  fenêtre, le premier disque demande bien un second disque ; reste à faire
  la détection avec le second disque dans la fenêtre (corrigé depuis : il
  restait +6 et +12).
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

- [ ] ReplayGain (nouveau) : après un rip du disque entier, les fichiers ont
  les tags REPLAYGAIN_TRACK_* et REPLAYGAIN_ALBUM_* (un lecteur comme
  foobar2000 ou VLC les lit) ; le log a une ligne « ReplayGain » par piste et
  une pour l'album. Essayer FLAC, MP3, ALAC/AAC et AIFF ; la case des
  réglages doit les désactiver.
- [ ] Recherche du décalage (nouveau) : Réglages → Lecture → « Détecter avec
  ce disque » (ou `galette offset`), avec un disque connu d'AccurateRip.
  Si plusieurs décalages sont possibles, recommencer avec un autre disque :
  il doit n'en rester qu'un, celui du lecteur.
- [ ] Test du cache (nouveau) : Réglages → Lecture → « Tester le cache du
  lecteur » (ou `galette cache`) ; le résultat doit s'afficher en quelques
  secondes.
- [ ] Piste cachée avant la piste 1 (HTOA, nouveau) : sur un disque qui en a
  une, la ligne « Piste cachée » apparaît en tête de liste ; elle est rippée
  en piste 00 (ou notée « Silence, pas de fichier »), le cue la référence
  et le log a une section « Track 00 ».
- [ ] Pré-accentuation (nouveau) : sur un disque concerné (rare, surtout des CD
  des années 80), un avertissement s'affiche, le cue a « FLAGS PRE » et le
  log le signale.
- [ ] Réparation CTDB (nouveau) : sur un disque abîmé rippé en entier, les
  pistes abîmées attendent la fin (« Abîmée, réparation à la fin »), puis
  sont réparées si possible ; le log dit « REPAIRED from CTDB parity ».
- [ ] Disque abîmé en mode sécurisé (nouveau) : les relectures abandonnent un
  secteur jamais lu sans erreur après 5 tours ; le rip doit finir en un temps
  raisonnable et le log noter les secteurs suspects ou illisibles. En mode
  rapide, les secteurs avec erreurs C2 doivent aussi être notés suspects.
- [ ] CTDB (nouveau) : après un rip du disque entier, le message final et le
  log disent ce qu'en pense le CTDB (« disque confirmé (n sur m) ») ; une piste
  absente d'AccurateRip mais trouvée dans le CTDB affiche « CTDB ✓ ». Essayer
  aussi un disque absent du CTDB et un rip de quelques pistes seulement.
- [ ] Dans l'application installée : bouton Arrêter pendant un rip,
  éjection, « Ouvrir le dossier », désinstallation.
- [ ] Un lecteur autre que l'ASUS.
- [ ] Langue : passer en anglais, faire un rip, revenir en français (nouveau).
- [ ] Message sans lecteur : débrancher le lecteur, la fenêtre doit dire
  « Aucun lecteur détecté. Branchez un lecteur CD. » (nouveau).
- [ ] Formats ALAC, MP3, AIFF et WAV (le rip testé était en FLAC).
- [ ] Thème Cathédrale.
- [ ] Rip d'un album déjà rippé dans un autre format.
- [ ] CD-Text (nouveau) : sur un disque qui en a, la liste Édition propose
  « CD-Text du disque : … » et le badge « CD-Text ✓ » s'affiche ; le log a
  une ligne « CD-Text ». Vérifier aussi qu'un disque sans CD-Text se lit
  normalement.

## Plus tard

- [ ] Désaccentuation : appliquer le filtre inverse aux pistes avec
  pré-accentuation, en option (utile pour MP3/AAC ; en FLAC on garde l'audio
  du disque tel quel, comme EAC).
- [ ] Pré-accentuation signalée seulement dans le sous-canal Q (et pas dans la
  TOC) : très rare, non détectée.
